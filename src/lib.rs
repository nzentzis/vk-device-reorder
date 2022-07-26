mod vk {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use std::ffi::CStr;
use std::collections::HashMap;
use std::sync::RwLock;

macro_rules! vk_function {
    { fn $name:ident ( $($arg_name:ident : $arg_ty:ty),* ) -> $res_ty:ty $body:block } => {
        mod $name {
            use super::*;
            pub const IMPLEMENTATION: unsafe extern "C" fn($($arg_name : $arg_ty),*) -> $res_ty =
                implementation as unsafe extern "C" fn($($arg_name : $arg_ty),*) -> $res_ty;
            pub const TYPE_ERASED: unsafe extern "C" fn() = unsafe {
                std::mem::transmute::<_, unsafe extern "C" fn()>(IMPLEMENTATION)
            };
            unsafe extern "C" fn implementation($($arg_name : $arg_ty),*) -> $res_ty $body
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

#[repr(transparent)]
struct GetProcAddr {
    func_ptr: vk::PFN_vkGetInstanceProcAddr
}

impl GetProcAddr {
    unsafe fn get_raw(&self, instance: Option<vk::VkInstance>, func: &str) -> vk::PFN_vkVoidFunction {
        let cstr = std::ffi::CString::new(func).ok()?;
        let ptr = self.func_ptr?;
        (ptr)(instance.unwrap_or_else(std::ptr::null_mut), cstr.as_ptr())
    }
}

macro_rules! get_proc_addr {
    ($gpa:ident ( $name:expr => $t:ty )) => {
        {
            type Inner = <$t as IntoIterator>::Item;
            let erased = $gpa.get_raw(None, $name);
            erased.map(|p| {
                std::mem::transmute::<unsafe extern "C" fn(), Inner>(p)
            })
        }
    };
    ($gpa:ident ( $inst:expr, $name:expr => $t:ty )) => {
        {
            type Inner = <$t as IntoIterator>::Item;
            let erased = $gpa.get_raw(Some($inst), $name);
            erased.map(|p| {
                std::mem::transmute::<unsafe extern "C" fn(), Inner>(p)
            })
        }
    };
}


lazy_static::lazy_static! {
    /// Global mapping of instance pointer to per-instance data
    static ref INSTANCE_TABLE: RwLock<HashMap<usize, InstanceData>> = RwLock::new(HashMap::new());
}

/// Per-Vulkan-instance data
struct InstanceData {
    gpa: GetProcAddr,
    destroy: <vk::PFN_vkDestroyInstance as IntoIterator>::Item,
    enumerate: <vk::PFN_vkEnumeratePhysicalDevices as IntoIterator>::Item,
    phys_dev_props: <vk::PFN_vkGetPhysicalDeviceProperties as IntoIterator>::Item,
    phys_dev_features: <vk::PFN_vkGetPhysicalDeviceFeatures as IntoIterator>::Item,
}

impl InstanceData {
    unsafe fn new(instance: vk::VkInstance, gpa: GetProcAddr) -> Option<Self> {
        let instance_gpa = get_proc_addr!(gpa("vkGetInstanceProcAddr" => vk::PFN_vkGetInstanceProcAddr));
        let instance_gpa = GetProcAddr {func_ptr: instance_gpa};

        Some(Self {
            enumerate: get_proc_addr!(gpa(instance, "vkEnumeratePhysicalDevices" => vk::PFN_vkEnumeratePhysicalDevices))?,
            destroy: get_proc_addr!(gpa(instance, "vkDestroyInstance" => vk::PFN_vkDestroyInstance))?,
            phys_dev_props: get_proc_addr!(gpa(instance, "vkGetPhysicalDeviceProperties" => vk::PFN_vkGetPhysicalDeviceProperties))?,
            phys_dev_features: get_proc_addr!(gpa(instance, "vkGetPhysicalDeviceFeatures" => vk::PFN_vkGetPhysicalDeviceFeatures))?,
            gpa: instance_gpa,
        })
    }
}

vk_function! {
    fn create_instance(
        create_info: *const vk::VkInstanceCreateInfo,
        allocator: *const vk::VkAllocationCallbacks,
        instance: *mut vk::VkInstance
    ) -> vk::VkResult {
        // find the layer instance create info for the next layer in the chain
        let mut next_chain = (*create_info).pNext as *mut vk::VkLayerInstanceCreateInfo;
        while !next_chain.is_null() &&
              !((*next_chain).sType == vk::VkStructureType_VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO &&
                (*next_chain).function == vk::VkLayerFunction__VK_LAYER_LINK_INFO) {
            next_chain = (*next_chain).pNext as *mut vk::VkLayerInstanceCreateInfo;
        }

        // if there's no next element, we're the last layer in the chain
        if next_chain.is_null() {
            return vk::VkResult_VK_ERROR_INITIALIZATION_FAILED;
        }

        // check that the next element is valid
        if (*next_chain).u.pLayerInfo.is_null() {
            return vk::VkResult_VK_ERROR_INITIALIZATION_FAILED;
        }

        let gpa = (*(*next_chain).u.pLayerInfo).pfnNextGetInstanceProcAddr;
        let gpa = GetProcAddr {func_ptr: gpa};

        // advance chain for next layer, and call its create function
        (*next_chain).u.pLayerInfo = (*(*next_chain).u.pLayerInfo).pNext;
        let create_func = match get_proc_addr!(gpa("vkCreateInstance" => vk::PFN_vkCreateInstance)) {
            Some(x) => x,
            None => {
                return vk::VkResult_VK_ERROR_INITIALIZATION_FAILED;
            }
        };

        let rc = (create_func)(create_info, allocator, instance);
        if rc != vk::VkResult_VK_SUCCESS {
            return rc;
        }

        // create and store per-instance data
        let inst_data = match InstanceData::new(*instance, gpa) {
            Some(x) => x,
            None => {
                // TODO: clean up next layer?
                return vk::VkResult_VK_ERROR_INITIALIZATION_FAILED;
            }
        };
        let tbl_key = (*instance) as usize;
        INSTANCE_TABLE.write().unwrap().insert(tbl_key, inst_data);

        vk::VkResult_VK_SUCCESS
    }
}

vk_function! {
    fn destroy_instance(
        instance: vk::VkInstance,
        allocator: *const vk::VkAllocationCallbacks
    ) {
        if let Some(inst) = INSTANCE_TABLE.write().unwrap().remove(&(instance as usize)) {
            (inst.destroy)(instance, allocator);
        }
    }
}

vk_function! {
    fn enumerate_devices(
        instance: vk::VkInstance,
        dev_count: *mut u32,
        devices: *mut vk::VkPhysicalDevice
    ) -> vk::VkResult {
        let instance_lock = INSTANCE_TABLE.read().unwrap();
        let instance_table = match instance_lock.get(&(instance as usize)) {
            Some(x) => x,
            None => {
                return vk::VkResult_VK_ERROR_UNKNOWN;
            }
        };

        let rc = (instance_table.enumerate)(instance, dev_count, devices);
        if rc != vk::VkResult_VK_SUCCESS {
            return rc;
        }

        // don't try to reorder devices if the device list pointer isn't valid
        if devices.is_null() {
            dbg!();
            return rc;
        }

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
            (instance_table.phys_dev_props)(*dev, props.as_mut_ptr());
            (instance_table.phys_dev_features)(*dev, features.as_mut_ptr());
            dbg!(std::mem::MaybeUninit::assume_init(props));
            dbg!(std::mem::MaybeUninit::assume_init(features));
        }

        devices.swap(0, 1);

        // truncate if needed
        *dev_count = devices.len() as u32;

        dbg!(*dev_count);

        vk::VkResult_VK_SUCCESS
    }
}

#[no_mangle]
pub unsafe extern "C" fn vkGetInstanceProcAddr(
    instance: vk::VkInstance,
    name: *const std::os::raw::c_char,
) -> vk::PFN_vkVoidFunction {
    let tbl_key = instance as usize;

    let name_str = CStr::from_ptr(name);
    println!("name is {}", String::from_utf8_lossy(name_str.to_bytes()));
    match name_str.to_bytes() {
        b"vkCreateInstance" => Some(create_instance::TYPE_ERASED),
        b"vkDestroyInstance" => Some(destroy_instance::TYPE_ERASED),
        b"vkEnumeratePhysicalDevices" => Some(enumerate_devices::TYPE_ERASED),
        _ => {
            let inst_table = INSTANCE_TABLE.read().unwrap();
            let inst_data = inst_table.get(&tbl_key)?;
            let gpa = (inst_data.gpa.func_ptr)?;
            (gpa)(instance, name)
        }
    }
}
