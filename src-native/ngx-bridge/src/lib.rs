//! Private, optional NVIDIA NGX bridge. This binary is NOT part of EvoParseq.exe.
//! The SDK's static library is linked only into this separate DLL.
//! Distributed independently of the single-EXE EvoParseq Windows package.
use std::{ffi::{c_char, c_void}, ptr::null_mut};

#[repr(C)]
struct Paths { paths: *const *const u16, count: u32 }
#[repr(C)]
struct Logging { callback: *const c_void, level: u32, other_sinks_disabled: bool }
#[repr(C)]
struct Common { paths: Paths, internal: *mut c_void, logging: Logging }

extern "C" {
    fn NVSDK_NGX_D3D11_Init(app: u64, data: *const u16, device: *mut c_void,
        common: *const Common, version: u32) -> u32;
    fn NVSDK_NGX_D3D11_Shutdown1(device: *mut c_void) -> u32;
    fn NVSDK_NGX_D3D11_GetCapabilityParameters(parameters: *mut *mut c_void) -> u32;
    fn NVSDK_NGX_D3D11_DestroyParameters(parameters: *mut c_void) -> u32;
    fn NVSDK_NGX_D3D11_CreateFeature(context: *mut c_void, feature: u32,
        parameters: *mut c_void, handle: *mut *mut c_void) -> u32;
    fn NVSDK_NGX_D3D11_ReleaseFeature(handle: *mut c_void) -> u32;
    fn NVSDK_NGX_D3D11_EvaluateFeature_C(context: *mut c_void,
        handle: *const c_void, parameters: *mut c_void, callback: *const c_void) -> u32;
    fn NVSDK_NGX_Parameter_GetI(parameters: *mut c_void,
        name: *const c_char, value: *mut i32) -> u32;
    fn NVSDK_NGX_Parameter_SetUI(parameters: *mut c_void, name: *const c_char, value: u32);
    fn NVSDK_NGX_Parameter_SetD3d11Resource(parameters: *mut c_void,
        name: *const c_char, value: *mut c_void);
}

pub struct Session {
    device: *mut c_void,
    parameters: *mut c_void,
    feature: *mut c_void,
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if !self.feature.is_null() { NVSDK_NGX_D3D11_ReleaseFeature(self.feature); }
            if !self.parameters.is_null() { NVSDK_NGX_D3D11_DestroyParameters(self.parameters); }
            NVSDK_NGX_D3D11_Shutdown1(self.device);
        }
    }
}

// All exported functions are private ABI v1 and invoked only after hash verification.
// The caller owns a D3D10Multithread lock for every entrypoint.
#[no_mangle]
pub unsafe extern "C" fn evoparseq_ngx_bridge_abi() -> u32 { 1 }

#[no_mangle]
pub unsafe extern "C" fn evoparseq_ngx_create(
    device: *mut c_void, context: *mut c_void, directory: *const u16,
    out: *mut *mut Session
) -> u32 {
    if device.is_null() || context.is_null() || directory.is_null() || out.is_null() {
        return 0;
    }
    *out = null_mut();
    let paths = [directory];
    let common = Common {
        paths: Paths { paths: paths.as_ptr(), count: 1 },
        internal: null_mut(),
        logging: Logging { callback: std::ptr::null(), level: 0, other_sinks_disabled: false },
    };
    if NVSDK_NGX_D3D11_Init(0, directory, device, &common, 0x15) != 1 { return 0; }
    let mut session = Box::new(Session { device, parameters: null_mut(), feature: null_mut() });
    if NVSDK_NGX_D3D11_GetCapabilityParameters(&mut session.parameters) != 1 { return 0; }
    let mut available = 0i32;
    if NVSDK_NGX_Parameter_GetI(session.parameters, c"VSR.Available".as_ptr(), &mut available) != 1
        || available == 0 { return 0; }
    if NVSDK_NGX_D3D11_CreateFeature(context, 16, session.parameters, &mut session.feature) != 1 {
        return 0;
    }
    *out = Box::into_raw(session);
    1
}

#[no_mangle]
pub unsafe extern "C" fn evoparseq_ngx_evaluate(
    session: *mut Session, context: *mut c_void,
    input: *mut c_void, output: *mut c_void,
    source_width: u32, source_height: u32, rect: *const i32, quality: u32
) -> u32 {
    if session.is_null() || context.is_null() || input.is_null() || output.is_null()
        || rect.is_null() || !(1..=4).contains(&quality) { return 0; }
    let s = &*session;
    let r = std::slice::from_raw_parts(rect, 4);
    if r[2] <= r[0] || r[3] <= r[1] { return 0; }
    NVSDK_NGX_Parameter_SetD3d11Resource(s.parameters, c"Input1".as_ptr(), input);
    NVSDK_NGX_Parameter_SetD3d11Resource(s.parameters, c"Output".as_ptr(), output);
    for (key, value) in [
        (c"Rect.X", 0), (c"Rect.Y", 0), (c"Rect.W", source_width),
        (c"Rect.H", source_height), (c"OutRect.X", r[0] as u32),
        (c"OutRect.Y", r[1] as u32), (c"OutRect.W", (r[2]-r[0]) as u32),
        (c"OutRect.H", (r[3]-r[1]) as u32), (c"VSR.QualityLevel", quality)
    ] {
        NVSDK_NGX_Parameter_SetUI(s.parameters, key.as_ptr(), value);
    }
    (NVSDK_NGX_D3D11_EvaluateFeature_C(context, s.feature, s.parameters,
        std::ptr::null()) == 1) as u32
}

#[no_mangle]
pub unsafe extern "C" fn evoparseq_ngx_destroy(session: *mut Session) {
    if !session.is_null() { drop(Box::from_raw(session)); }
}
