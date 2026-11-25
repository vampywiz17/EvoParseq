//! Request RTX Video Super Resolution from the installed D3D11 driver.
use windows::{
    core::{Result, GUID, HRESULT},
    Win32::Graphics::Direct3D11::{ID3D11VideoContext, ID3D11VideoProcessor},
};

// NVIDIA's driver extension as used by Chromium's D3D11 presenter.
// https://chromium.googlesource.com/chromium/src/+/59be26606ae9962880b6f20ed255fe7e8e484c19/ui/gl/swap_chain_presenter.cc
const SUPER_RESOLUTION: GUID = GUID::from_u128(0xd43ce1b3_1f4b_48ac_baee_c3c25375e6f7);
const ENABLE_SUPER_RESOLUTION: [u32; 3] = [1, 2, 1];

pub unsafe fn request(
    context: &ID3D11VideoContext,
    processor: &ID3D11VideoProcessor,
) -> Result<()> {
    set_extension(|stream, guid, size, payload| {
        HRESULT(context.VideoProcessorSetStreamExtension(processor, stream, guid, size, payload))
    })
}

fn set_extension(
    set: impl FnOnce(u32, &GUID, u32, *const std::ffi::c_void) -> HRESULT,
) -> Result<()> {
    set(
        0,
        &SUPER_RESOLUTION,
        std::mem::size_of_val(&ENABLE_SUPER_RESOLUTION) as u32,
        ENABLE_SUPER_RESOLUTION.as_ptr().cast(),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{E_INVALIDARG, S_OK};

    #[test]
    fn driver_request_uses_the_expected_payload_and_preserves_rejection() {
        for status in [S_OK, E_INVALIDARG] {
            let result = set_extension(|stream, guid, size, payload| {
                assert_eq!(stream, 0);
                assert_eq!(
                    *guid,
                    GUID::from_u128(0xd43ce1b3_1f4b_48ac_baee_c3c25375e6f7)
                );
                assert_eq!(size, 12);
                assert_eq!(unsafe { payload.cast::<[u32; 3]>().read() }, [1, 2, 1]);
                status
            });
            if status.is_ok() {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().code(), status);
            }
        }
    }
}
