//! Optional RTX Video SDK 1.1 D3D11 adapter. No CPU pixel access.
//! The public C ABI is bound directly; no NVIDIA sample implementation is copied.
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};

pub struct Surfaces {
    pub input: ID3D11Texture2D,
    pub output: ID3D11Texture2D,
    clear: ID3D11RenderTargetView,
    key: (u32, u32, u32, u32, DXGI_FORMAT),
}

impl Surfaces {
    unsafe fn new(
        device: &ID3D11Device,
        source: (u32, u32),
        target: (u32, u32),
        format: DXGI_FORMAT,
    ) -> anyhow::Result<Self> {
        let texture = |size: (u32, u32), flags: u32| -> windows::core::Result<ID3D11Texture2D> {
            let mut resource = None;
            device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.0,
                    Height: size.1,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: flags,
                    ..Default::default()
                },
                None,
                Some(&mut resource),
            )?;
            resource.ok_or_else(windows::core::Error::from_thread)
        };
        let input = texture(
            source,
            (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
        )?;
        let output = texture(
            target,
            (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_UNORDERED_ACCESS).0
                as u32,
        )?;
        let mut clear = None;
        device.CreateRenderTargetView(&output, None, Some(&mut clear))?;
        Ok(Self {
            input,
            output,
            clear: clear.ok_or_else(windows::core::Error::from_thread)?,
            key: (source.0, source.1, target.0, target.1, format),
        })
    }
}

pub struct Vsr {
    sdk: sdk::Instance,
    surfaces: Option<Surfaces>,
    quality: u8,
    evaluated: bool,
}
impl Vsr {
    pub unsafe fn new(device: &ID3D11Device, quality: u8) -> anyhow::Result<Self> {
        anyhow::ensure!((1..=4).contains(&quality), "Invalid RTX VSR quality");
        Ok(Self {
            sdk: sdk::Instance::new(device)?,
            surfaces: None,
            quality,
            evaluated: false,
        })
    }
    /// One source-sized conversion target and one UAV output, reused across frames.
    /// The caller skips VSR when the viewport does not upscale in both dimensions.
    pub unsafe fn prepare(
        &mut self,
        device: &ID3D11Device,
        source: (u32, u32),
        target: (u32, u32),
        format: DXGI_FORMAT,
    ) -> anyhow::Result<&Surfaces> {
        let key = (source.0, source.1, target.0, target.1, format);
        if self.surfaces.as_ref().is_none_or(|s| s.key != key) {
            self.surfaces = Some(Surfaces::new(device, source, target, format)?);
        }
        Ok(self.surfaces.as_ref().unwrap())
    }
    pub fn warming(&self) -> bool {
        !self.evaluated
    }
    pub unsafe fn evaluate(
        &mut self,
        context: &ID3D11DeviceContext,
        target: [i32; 4],
    ) -> anyhow::Result<()> {
        let surfaces = self
            .surfaces
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VSR surfaces missing"))?;
        context.ClearRenderTargetView(&surfaces.clear, &[0.0, 0.0, 0.0, 1.0]);
        let result = self.sdk.evaluate(context, surfaces, target, self.quality);
        self.evaluated = result.is_ok();
        result
    }
}

mod sdk {
    use super::*;
    use std::{ffi::c_void, ptr::null_mut};
    use windows::{core::Interface, Win32::Graphics::Direct3D10::ID3D10Multithread};

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x100;
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x1000;

    type Create = unsafe extern "C" fn(*mut c_void, *mut c_void, *const u16,
        *mut *mut c_void) -> u32;
    type Evaluate = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void,
        *mut c_void, u32, u32, *const i32, u32) -> u32;
    type Destroy = unsafe extern "C" fn(*mut c_void);
    type Abi = unsafe extern "C" fn() -> u32;

    struct Library(*mut c_void);
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe { FreeLibrary(self.0); }
        }
    }
    impl Library {
        unsafe fn open(path: &std::path::Path) -> anyhow::Result<Self> {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let module = LoadLibraryExW(wide.as_ptr(), null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
            anyhow::ensure!(!module.is_null(), "Cannot load the verified NVIDIA NGX bridge");
            Ok(Self(module))
        }
        unsafe fn symbol<T: Copy>(&self, symbol: &'static [u8]) -> anyhow::Result<T> {
            let ptr = GetProcAddress(self.0, symbol.as_ptr());
            anyhow::ensure!(!ptr.is_null(), "NVIDIA NGX bridge ABI symbol missing");
            Ok(std::mem::transmute_copy(&ptr))
        }
    }

    struct ContextLock(ID3D10Multithread);
    impl ContextLock {
        unsafe fn enter(device: &ID3D11Device) -> anyhow::Result<Self> {
            let lock: ID3D10Multithread = device.cast()?;
            lock.Enter();
            Ok(Self(lock))
        }
    }
    impl Drop for ContextLock {
        fn drop(&mut self) {
            unsafe { self.0.Leave(); }
        }
    }

    pub struct Instance {
        device: ID3D11Device,
        library: Library,
        session: *mut c_void,
        evaluate: Evaluate,
        destroy: Destroy,
    }
    impl Instance {
        pub unsafe fn new(device: &ID3D11Device) -> anyhow::Result<Self> {
            let root = crate::vsr_runtime::validated_directory()?;
            let library = Library::open(&root.join("evoparseq-ngx-bridge.dll"))?;
            let abi: Abi = library.symbol(b"evoparseq_ngx_bridge_abi\0")?;
            anyhow::ensure!(abi() == 1, "Unsupported NVIDIA NGX bridge ABI");
            let create: Create = library.symbol(b"evoparseq_ngx_create\0")?;
            let evaluate: Evaluate = library.symbol(b"evoparseq_ngx_evaluate\0")?;
            let destroy: Destroy = library.symbol(b"evoparseq_ngx_destroy\0")?;
            use std::os::windows::ffi::OsStrExt;
            let path: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
            let context = device.GetImmediateContext()?;
            let _lock = ContextLock::enter(device)?;
            let mut session = null_mut();
            anyhow::ensure!(create(device.as_raw(), context.as_raw(), path.as_ptr(),
                &mut session) == 1 && !session.is_null(),
                "NVIDIA NGX VSR initialization failed or GPU/driver unsupported");
            Ok(Self { device: device.clone(), library, session, evaluate, destroy })
        }
        pub unsafe fn evaluate(&self, context: &ID3D11DeviceContext,
            surfaces: &Surfaces, rect: [i32; 4], quality: u8) -> anyhow::Result<()> {
            let _lock = ContextLock::enter(&self.device)?;
            anyhow::ensure!((self.evaluate)(self.session, context.as_raw(),
                surfaces.input.as_raw(), surfaces.output.as_raw(), surfaces.key.0,
                surfaces.key.1, rect.as_ptr(), u32::from(quality)) == 1,
                "NVIDIA NGX VSR frame evaluation failed");
            Ok(())
        }
    }
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                let _lock = ContextLock::enter(&self.device).ok();
                (self.destroy)(self.session);
            }
        }
    }
}
