mod vk {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use std::ffi::CStr;

#[macro_use] mod dispatch;

// generate per-instance dispatch table
dispatch_table! {
    instance_dispatch {
        GetInstanceProcAddr
        DestroyInstance
        EnumeratePhysicalDevices
        GetPhysicalDeviceProperties
        GetPhysicalDeviceFeatures
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

struct DeviceEntry {
}

fn modify_device_entries(entries: &mut [DeviceEntry]) {
}

vk_function! {
    fn enumerate_devices(
        instance: vk::VkInstance,
        dev_count: *mut u32,
        devices: *mut vk::VkPhysicalDevice
    ) -> VkResult {
        let dispatch = instance_dispatch::get(instance as usize)
                      .ok_or(vk::VkResult_VK_ERROR_UNKNOWN)?;

        if !devices.is_null() {
            (dispatch.enumerate_physical_devices)(instance, dev_count, devices).from_vk()?;

            dbg!(*dev_count);

            let devices: &mut [vk::VkPhysicalDevice] = std::slice::from_raw_parts_mut(
                devices,
                *dev_count as usize
            );
            dbg!(&devices);

            // modify device list
            for dev in devices.iter() {
                let mut props = std::mem::MaybeUninit::uninit();
                let mut features = std::mem::MaybeUninit::uninit();
                (dispatch.get_physical_device_properties)(*dev, props.as_mut_ptr());
                (dispatch.get_physical_device_features)(*dev, features.as_mut_ptr());
                dbg!(std::mem::MaybeUninit::assume_init(props));
                dbg!(std::mem::MaybeUninit::assume_init(features));
            }

            devices.swap(0, 1);

            // truncate if needed
            *dev_count = devices.len() as u32;

            dbg!(*dev_count);
        } else {
            // the user didn't provide a device buffer, and we may filter devices - in order to
            // present a valid view of reality to the caller, we need to query for the devices,
            // filter, and then return the resulting count.
            (dispatch.enumerate_physical_devices)(instance, dev_count, devices).from_vk()?;
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
