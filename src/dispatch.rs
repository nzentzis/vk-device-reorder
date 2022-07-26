//! Storage and implementation of instance dispatch tables
//!
//! This module also contains the `VulkanFunc` trait, which is used to then bindgen function types
//! into callable methods.
pub(super) trait ProjectOption {
    type Inner;
}

impl<T> ProjectOption for Option<T> {
    type Inner = T;
}

#[repr(transparent)]
pub(crate) struct GetProcAddr {
    pub(crate) func_ptr: crate::vk::PFN_vkGetInstanceProcAddr
}

impl GetProcAddr {
    pub(crate) unsafe fn get_raw(
        &self,
        instance: Option<crate::vk::VkInstance>,
        func: &str,
    ) -> crate::vk::PFN_vkVoidFunction {
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

macro_rules! dispatch_table {
    { $(
        $name:ident {
            $($vk_name:ident)*
        }
    )* } => {
        $(
            #[allow(dead_code)]
            mod $name {
                use std::collections::HashMap;
                use std::sync::RwLock;
                use std::sync::Arc;

                lazy_static::lazy_static! {
                    static ref TABLE: RwLock<HashMap<usize, Arc<Record>>> = RwLock::new(HashMap::new());
                }

                paste::paste! {
                    pub(crate) struct Record {
                        $(
                            pub [<$vk_name:snake>] : <
                                crate::vk:: [< PFN_vk $vk_name >]
                                as crate::dispatch::ProjectOption
                            >::Inner,
                        )*
                    }
                }

                impl Record {
                    fn build(
                        instance: crate::vk::VkInstance,
                        gpa: crate::dispatch::GetProcAddr,
                    ) -> Option<Self> {
                        unsafe {
                            paste::paste! {
                                Some(Self {
                                    $(
                                        [<$vk_name:snake>]: get_proc_addr!(
                                            gpa(instance, stringify!([< vk $vk_name >]) =>
                                                crate::vk:: [< PFN_vk $vk_name >])
                                        )?,
                                    )*
                                })
                            }
                        }
                    }
                }

                /// Build a dispatch table and associate it with the given key
                ///
                /// Returns whether the table was built successfully
                pub(crate) fn build(
                    key: usize,
                    instance: crate::vk::VkInstance,
                    gpa: crate::dispatch::GetProcAddr,
                ) -> bool {
                    if let Some(rec) = Record::build(instance, gpa) {
                        TABLE.write().unwrap().insert(key, Arc::new(rec));
                        true
                    } else {
                        false
                    }
                }

                /// Get the dispatch table entry associated with the given key, if any
                pub(crate) fn get(key: usize) -> Option<Arc<Record>> {
                    TABLE.read().unwrap().get(&key).map(Arc::clone)
                }

                /// Remove the dispatch table entry associated with the given key, if any
                pub(crate) fn destroy(key: usize) {
                    TABLE.write().unwrap().remove(&key);
                }
            }
        )*
    }
}
