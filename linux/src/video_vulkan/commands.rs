//! Typed calls for the Vulkan Video entry points exposed by ash.
use ash::vk;

pub struct Commands {
    device: vk::Device,
    queue: ash::khr::video_queue::Device,
    decode: ash::khr::video_decode_queue::Device,
}

impl Commands {
    pub fn new(instance: &ash::Instance, device: &ash::Device) -> Self {
        Self {
            device: device.handle(),
            queue: ash::khr::video_queue::Device::new(instance, device),
            decode: ash::khr::video_decode_queue::Device::new(instance, device),
        }
    }
    pub unsafe fn create_video_session(
        &self,
        info: &vk::VideoSessionCreateInfoKHR<'_>,
        _: Option<&vk::AllocationCallbacks<'_>>,
    ) -> Result<vk::VideoSessionKHR, vk::Result> {
        let mut result = vk::VideoSessionKHR::null();
        unsafe {
            (self.queue.fp().create_video_session_khr)(
                self.device,
                info,
                std::ptr::null(),
                &mut result,
            )
            .result()?
        };
        Ok(result)
    }
    pub unsafe fn destroy_video_session(
        &self,
        session: vk::VideoSessionKHR,
        _: Option<&vk::AllocationCallbacks<'_>>,
    ) {
        unsafe {
            (self.queue.fp().destroy_video_session_khr)(self.device, session, std::ptr::null())
        };
    }
    pub unsafe fn get_video_session_memory_requirements(
        &self,
        session: vk::VideoSessionKHR,
    ) -> Result<Vec<vk::VideoSessionMemoryRequirementsKHR<'static>>, vk::Result> {
        let mut count = 0;
        unsafe {
            (self.queue.fp().get_video_session_memory_requirements_khr)(
                self.device,
                session,
                &mut count,
                std::ptr::null_mut(),
            )
            .result()?
        };
        let mut result = vec![vk::VideoSessionMemoryRequirementsKHR::default(); count as usize];
        unsafe {
            (self.queue.fp().get_video_session_memory_requirements_khr)(
                self.device,
                session,
                &mut count,
                result.as_mut_ptr(),
            )
            .result()?
        };
        result.truncate(count as usize);
        Ok(result)
    }
    pub unsafe fn bind_video_session_memory(
        &self,
        session: vk::VideoSessionKHR,
        bindings: &[vk::BindVideoSessionMemoryInfoKHR<'_>],
    ) -> Result<(), vk::Result> {
        unsafe {
            (self.queue.fp().bind_video_session_memory_khr)(
                self.device,
                session,
                bindings.len() as u32,
                bindings.as_ptr(),
            )
            .result()
        }
    }
    pub unsafe fn create_video_session_parameters(
        &self,
        info: &vk::VideoSessionParametersCreateInfoKHR<'_>,
        _: Option<&vk::AllocationCallbacks<'_>>,
    ) -> Result<vk::VideoSessionParametersKHR, vk::Result> {
        let mut result = vk::VideoSessionParametersKHR::null();
        unsafe {
            (self.queue.fp().create_video_session_parameters_khr)(
                self.device,
                info,
                std::ptr::null(),
                &mut result,
            )
            .result()?
        };
        Ok(result)
    }
    pub unsafe fn destroy_video_session_parameters(
        &self,
        parameters: vk::VideoSessionParametersKHR,
        _: Option<&vk::AllocationCallbacks<'_>>,
    ) {
        // No object exists before the first parameter creation or on partial
        // initialization failure. Older RADV dereferences this null handle.
        if parameters == vk::VideoSessionParametersKHR::null() {
            return;
        }
        unsafe {
            (self.queue.fp().destroy_video_session_parameters_khr)(
                self.device,
                parameters,
                std::ptr::null(),
            )
        };
    }
    pub unsafe fn cmd_begin_video_coding(
        &self,
        command: vk::CommandBuffer,
        info: &vk::VideoBeginCodingInfoKHR<'_>,
    ) {
        unsafe { (self.queue.fp().cmd_begin_video_coding_khr)(command, info) };
    }
    pub unsafe fn cmd_control_video_coding(
        &self,
        command: vk::CommandBuffer,
        info: &vk::VideoCodingControlInfoKHR<'_>,
    ) {
        unsafe { (self.queue.fp().cmd_control_video_coding_khr)(command, info) };
    }
    pub unsafe fn cmd_end_video_coding(
        &self,
        command: vk::CommandBuffer,
        info: &vk::VideoEndCodingInfoKHR<'_>,
    ) {
        unsafe { (self.queue.fp().cmd_end_video_coding_khr)(command, info) };
    }
    pub unsafe fn cmd_decode_video(
        &self,
        command: vk::CommandBuffer,
        info: &vk::VideoDecodeInfoKHR<'_>,
    ) {
        unsafe { (self.decode.fp().cmd_decode_video_khr)(command, info) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::{CStr, c_char, c_void},
        sync::atomic::{AtomicU64, Ordering},
    };
    use vk::Handle as _;

    static LAST_DESTROYED: AtomicU64 = AtomicU64::new(u64::MAX);
    unsafe extern "system" fn destroy_parameters(
        _: vk::Device,
        parameters: vk::VideoSessionParametersKHR,
        _: *const vk::AllocationCallbacks<'_>,
    ) {
        LAST_DESTROYED.store(parameters.as_raw(), Ordering::SeqCst);
    }
    unsafe extern "system" fn get_proc(
        _: vk::Device,
        name: *const c_char,
    ) -> vk::PFN_vkVoidFunction {
        if unsafe { CStr::from_ptr(name) } == c"vkDestroyVideoSessionParametersKHR" {
            Some(unsafe {
                std::mem::transmute::<
                    vk::PFN_vkDestroyVideoSessionParametersKHR,
                    unsafe extern "system" fn(),
                >(destroy_parameters)
            })
        } else {
            None
        }
    }
    #[test]
    fn linux_parameter_first_creation_and_partial_cleanup_never_dispatch_null_destroy() {
        // CPU-only dispatch fixture: no loader, driver or Vulkan device is used.
        let instance = unsafe {
            ash::Instance::load_with(
                |name| {
                    if name == c"vkGetDeviceProcAddr" {
                        get_proc as *const () as *const c_void
                    } else {
                        std::ptr::null()
                    }
                },
                vk::Instance::null(),
            )
        };
        let device = unsafe { ash::Device::load_with(|_| std::ptr::null(), vk::Device::null()) };
        let commands = Commands::new(&instance, &device);
        LAST_DESTROYED.store(u64::MAX, Ordering::SeqCst);
        unsafe {
            commands.destroy_video_session_parameters(vk::VideoSessionParametersKHR::null(), None);
        }
        assert_eq!(LAST_DESTROYED.load(Ordering::SeqCst), u64::MAX);
        unsafe {
            commands
                .destroy_video_session_parameters(vk::VideoSessionParametersKHR::from_raw(7), None);
        }
        assert_eq!(LAST_DESTROYED.load(Ordering::SeqCst), 7);
        unsafe {
            commands.destroy_video_session_parameters(vk::VideoSessionParametersKHR::null(), None);
        }
        assert_eq!(LAST_DESTROYED.load(Ordering::SeqCst), 7);
    }
}
