mod vk {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use std::ffi::CStr;
use std::ptr;
use std::mem::MaybeUninit;

#[macro_use] mod dispatch;
mod config;

// generate per-instance dispatch table
dispatch_table! {
    instance_dispatch {
        GetInstanceProcAddr,
        DestroyInstance,
        EnumeratePhysicalDevices,
        GetPhysicalDeviceProperties,
        GetPhysicalDeviceFeatures,
        GetPhysicalDeviceDisplayPropertiesKHR?,
    }
}

type VkResult = Result<(), vk::VkResult>;

trait IntoVkResult {
    fn into_vk_result(self) -> vk::VkResult;
}

impl IntoVkResult for VkResult {
    fn into_vk_result(self) -> vk::VkResult {
        match self {
            Ok(_) => vk::VkResult_VK_SUCCESS,
            Err(e) => e,
        }
    }
}

trait FromVkResult {
    fn from_vk(self) -> VkResult;
}

impl FromVkResult for vk::VkResult {
    fn from_vk(self) -> VkResult {
        if self == vk::VkResult_VK_SUCCESS {
            Ok(())
        } else {
            Err(self)
        }
    }
}

macro_rules! vk_function {
    { fn $name:ident ( $($arg_name:ident : $arg_ty:ty),* ) -> $res_ty:ty $body:block } => {
        mod $name {
            use super::*;
            pub const IMPLEMENTATION: unsafe extern "C" fn($($arg_name : $arg_ty),*) -> vk::VkResult =
                implementation as unsafe extern "C" fn($($arg_name : $arg_ty),*) -> vk::VkResult;
            pub const TYPE_ERASED: unsafe extern "C" fn() = unsafe {
                std::mem::transmute::<_, unsafe extern "C" fn()>(IMPLEMENTATION)
            };
            unsafe fn impl_body($($arg_name : $arg_ty),*) -> $res_ty $body
            unsafe extern "C" fn implementation($($arg_name : $arg_ty),*) -> vk::VkResult {
                impl_body($($arg_name),*).into_vk_result()
            }
        }
    };
    { fn $name:ident ( $($arg_name:ident : $arg_ty:ty),* ) $body:block } => {
        mod $name {
            use super::*;
            pub const IMPLEMENTATION: unsafe extern "C" fn($($arg_name : $arg_ty),*) =
                implementation as unsafe extern "C" fn($($arg_name : $arg_ty),*);
            pub const TYPE_ERASED: unsafe extern "C" fn() = unsafe {
                std::mem::transmute::<_, unsafe extern "C" fn()>(IMPLEMENTATION)
            };
            unsafe extern "C" fn implementation($($arg_name : $arg_ty),*) $body
        }
    };
}

vk_function! {
    fn create_instance(
        create_info: *const vk::VkInstanceCreateInfo,
        allocator: *const vk::VkAllocationCallbacks,
        instance: *mut vk::VkInstance
    ) -> VkResult {
        // find the layer instance create info for the next layer in the chain
        let mut next_chain = (*create_info).pNext as *mut vk::VkLayerInstanceCreateInfo;
        while !next_chain.is_null() &&
              !((*next_chain).sType == vk::VkStructureType_VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO &&
                (*next_chain).function == vk::VkLayerFunction__VK_LAYER_LINK_INFO) {
            next_chain = (*next_chain).pNext as *mut vk::VkLayerInstanceCreateInfo;
        }

        // if there's no next element, we're the last layer in the chain
        if next_chain.is_null() {
            return Err(vk::VkResult_VK_ERROR_INITIALIZATION_FAILED);
        }

        // check that the next element is valid
        if (*next_chain).u.pLayerInfo.is_null() {
            return Err(vk::VkResult_VK_ERROR_INITIALIZATION_FAILED);
        }

        let gpa = (*(*next_chain).u.pLayerInfo).pfnNextGetInstanceProcAddr;
        let gpa = dispatch::GetProcAddr {func_ptr: gpa};

        // advance chain for next layer, and call its create function
        (*next_chain).u.pLayerInfo = (*(*next_chain).u.pLayerInfo).pNext;
        let create_func = match get_proc_addr!(gpa("vkCreateInstance" => vk::PFN_vkCreateInstance)) {
            Some(x) => x,
            None => {
                return Err(vk::VkResult_VK_ERROR_INITIALIZATION_FAILED);
            }
        };

        (create_func)(create_info, allocator, instance).from_vk()?;

        // create and store per-instance data
        if !instance_dispatch::build((*instance) as usize, *instance, gpa) {
            // failed to build dispatch table
            // TODO: clean up next layer?
            return Err(vk::VkResult_VK_ERROR_INITIALIZATION_FAILED);
        }

        Ok(())
    }
}

vk_function! {
    fn destroy_instance(
        instance: vk::VkInstance,
        allocator: *const vk::VkAllocationCallbacks
    ) {
        if let Some(rec) = instance_dispatch::get(instance as usize) {
            (rec.destroy_instance)(instance, allocator);
        }
        instance_dispatch::destroy(instance as usize);
    }
}

#[derive(Debug)]
struct DeviceEntry {
    /// Index into the devices array that this entry represents
    ///
    /// This must be left unmodified.
    arr_idx: usize,

    /// Device name
    name: String,

    /// Raw device properties
    props: vk::VkPhysicalDeviceProperties,

    /// Raw device features
    features: vk::VkPhysicalDeviceFeatures,

    /// Whether the device has any displays attached
    displays: Vec<DisplayInfo>,
}

#[derive(Debug)]
struct DisplayInfo {
    /// Name of the display
    name: String,

    /// Physical resolution
    resolution: (u32, u32),
}

unsafe fn get_displays(
    dispatch: &instance_dispatch::Record,
    dev: vk::VkPhysicalDevice,
) -> Result<Vec<DisplayInfo>, vk::VkResult> {
    if let Some(get_props) = dispatch.get_physical_device_display_properties_k_h_r {
        let mut num_displays = 0;
        (get_props)(dev, &mut num_displays, ptr::null_mut()).from_vk()?;

        let mut props_arr = Vec::with_capacity(num_displays as usize);
        (get_props)(dev, &mut num_displays, props_arr.as_mut_ptr()).from_vk()?;
        props_arr.set_len(num_displays as usize);

        Ok(props_arr.into_iter().map(|entry| {
            DisplayInfo {
                name: String::new(),
                resolution: (entry.physicalResolution.width, entry.physicalResolution.height),
            }
        }).collect())
    } else {
        Ok(Vec::new())
    }
}

impl DeviceEntry {
    unsafe fn from_device(
        dispatch: &instance_dispatch::Record,
        dev: vk::VkPhysicalDevice,
        arr_idx: usize,
    ) -> Self {
        let mut props = MaybeUninit::uninit();
        let mut features = MaybeUninit::uninit();
        (dispatch.get_physical_device_properties)(dev, props.as_mut_ptr());
        (dispatch.get_physical_device_features)(dev, features.as_mut_ptr());

        let props = MaybeUninit::assume_init(props);
        let features = MaybeUninit::assume_init(features);

        let name = {
            let name = props.deviceName.as_slice();
            let name = &*(name as *const _ as *const [u8]);
            let length = name.iter().position(|b| *b == 0).unwrap_or(name.len());
            String::from_utf8_lossy(&name[..length]).into_owned()
        };

        // query attached display devices
        let displays = get_displays(dispatch, dev).unwrap_or_else(|_| Vec::new());

        Self {
            arr_idx,
            props,
            features,
            name,
            displays,
        }
    }
}

/// Modify the given vector of device entries
fn modify_device_entries(entries: &mut Vec<DeviceEntry>) {
    let conf: &config::Config = &config::CONFIG;
    entries.retain(|ent| !conf.rules_matching(ent)
                              .any(|r| r.hide));
    entries.sort_unstable_by_key(|ent| conf.rules_matching(ent)
                                           .filter_map(|r| r.priority)
                                           .map(|x| -x) // negate, so higher priorities go to the
                                                        // top of the resulting list
                                           .max()
                                           .unwrap_or(0));
}

struct DeviceArray<'a> {
    entries: Vec<DeviceEntry>,
    array: &'a mut [vk::VkPhysicalDevice],
}

impl<'a> DeviceArray<'a> {
    unsafe fn from_raw(
        dispatch: &instance_dispatch::Record,
        array: &'a mut [vk::VkPhysicalDevice]
    ) -> Self {
        let entries = array.iter().enumerate()
                     .map(|(i, d)| DeviceEntry::from_device(dispatch, *d, i))
                     .collect();

        Self {
            array,
            entries,
        }
    }

    /// Commit changes to the `entries` array back to `array`, returning the number of items to
    /// keep.
    fn commit(self) -> usize {
        // allocate temporary buffer
        let temp_buf = self.entries.into_iter()
                      .map(|e| self.array[e.arr_idx])
                      .collect::<Vec<_>>();

        // copy back into array
        self.array[..temp_buf.len()].copy_from_slice(&temp_buf);

        temp_buf.len()
    }
}

vk_function! {
    fn enumerate_devices(
        instance: vk::VkInstance,
        dev_count: *mut u32,
        devices: *mut vk::VkPhysicalDevice
    ) -> VkResult {
        let dispatch = instance_dispatch::get(instance as usize)
                      .ok_or(vk::VkResult_VK_ERROR_UNKNOWN)?;
        
        // query the real device list and process it via our filtering function
        let mut num_devs = 0;
        (dispatch.enumerate_physical_devices)(instance, &mut num_devs, ptr::null_mut()).from_vk()?;

        let mut devices_temp = Vec::with_capacity(num_devs as usize);
        (dispatch.enumerate_physical_devices)(instance, &mut num_devs, devices_temp.as_mut_ptr())
            .from_vk()?;
        devices_temp.set_len(num_devs as usize);

        let mut array = DeviceArray::from_raw(&dispatch, devices_temp.as_mut_slice());
        modify_device_entries(&mut array.entries);
        let new_len = array.commit();
        devices_temp.truncate(new_len);

        // if the user provided a buffer, copy the 
        if !devices.is_null() {
            let user_count = *dev_count as usize;

            let devices: &mut [MaybeUninit<vk::VkPhysicalDevice>] = std::slice::from_raw_parts_mut(
                devices as *mut MaybeUninit<vk::VkPhysicalDevice>,
                *dev_count as usize
            );

            if user_count >= devices_temp.len() {
                // copy all entries
                *dev_count = devices_temp.len() as u32;
                for (arr_entry, dev) in devices.iter_mut().zip(devices_temp) {
                    arr_entry.write(dev);
                }
            } else {
                // copy up to the allocated amount
                for (arr_entry, dev) in devices[..user_count].iter_mut().zip(devices_temp) {
                    arr_entry.write(dev);
                }
                return vk::VkResult_VK_INCOMPLETE.from_vk();
            }
        } else {
            *dev_count = devices_temp.len() as u32;
        }

        Ok(())
    }
}

#[no_mangle]
pub unsafe extern "C" fn vkGetInstanceProcAddr(
    instance: vk::VkInstance,
    name: *const std::os::raw::c_char,
) -> vk::PFN_vkVoidFunction {
    let tbl_key = instance as usize;
    let name_str = CStr::from_ptr(name);
    match name_str.to_bytes() {
        b"vkCreateInstance" => Some(create_instance::TYPE_ERASED),
        b"vkDestroyInstance" => Some(destroy_instance::TYPE_ERASED),
        b"vkEnumeratePhysicalDevices" => Some(enumerate_devices::TYPE_ERASED),
        _ => {
            let dispatch = instance_dispatch::get(tbl_key)?;
            (dispatch.get_instance_proc_addr)(instance, name)
        }
    }
}
