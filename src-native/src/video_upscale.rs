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
