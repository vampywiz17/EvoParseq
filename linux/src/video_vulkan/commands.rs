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
