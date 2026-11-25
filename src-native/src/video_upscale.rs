//! Optional GPU postprocessing selection. NVIDIA uses the existing D3D11 video processor.
use windows::{
    core::Interface,
    Win32::Graphics::{Direct3D11::ID3D11Device, Dxgi::IDXGIDevice},
};

pub enum Effect {
    Nvidia,
    #[cfg(feature = "vendor-video")]
    Intel(Box<evoparseq_video::intel_vsr::IntelAiVsr>),
}

impl Effect {
    pub unsafe fn new(device: &ID3D11Device) -> anyhow::Result<Self> {
        let vendor = device
            .cast::<IDXGIDevice>()?
            .GetAdapter()?
            .GetDesc()?
            .VendorId;
        match vendor {
            0x10de => Ok(Self::Nvidia),
            #[cfg(feature = "vendor-video")]
            0x8086 => Ok(Self::Intel(Box::new(
                evoparseq_video::intel_vsr::IntelAiVsr::new(device)?,
            ))),
            _ => anyhow::bail!("AI upscaling is unavailable for the selected GPU"),
        }
    }

    pub fn vendor(&self) -> &'static str {
        match self {
            Self::Nvidia => "NVIDIA",
            #[cfg(feature = "vendor-video")]
            Self::Intel(_) => "Intel",
        }
    }

    pub fn ready_status(&self) -> &'static str {
        match self {
            Self::Nvidia => "Waiting for an eligible frame to request NVIDIA driver VSR",
            #[cfg(feature = "vendor-video")]
            Self::Intel(_) => "Waiting for upscaling; Intel AI capability not yet checked",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{
        Foundation::HMODULE,
        Graphics::{Direct3D::D3D_DRIVER_TYPE_WARP, Direct3D11::*},
    };

    #[test]
    fn effect_checks_the_actual_render_device_and_reports_nvidia_pending() {
        let mut device = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
            .unwrap();
        }
        // The software adapter must never be reported as a hardware AI effect.
        let error = unsafe { Effect::new(&device.unwrap()) }.err().unwrap();
        assert_eq!(
            error.to_string(),
            "AI upscaling is unavailable for the selected GPU"
        );
        assert_eq!(Effect::Nvidia.vendor(), "NVIDIA");
        assert_eq!(
            Effect::Nvidia.ready_status(),
            "Waiting for an eligible frame to request NVIDIA driver VSR",
        );
    }
}
