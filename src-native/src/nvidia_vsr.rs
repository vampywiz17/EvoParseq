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
    use std::{
        ffi::{c_char, c_void},
        os::windows::ffi::OsStrExt,
        path::Path,
        ptr::null_mut,
    };
    use windows::{core::Interface, Win32::Graphics::Direct3D10::ID3D10Multithread};

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x100;
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x1000;

    /// Return a regular Win32 absolute directory for NGX feature discovery.
    /// std::fs::canonicalize() on Windows usually returns a verbatim \\?\\ path.
    /// NVIDIA NGX appends feature names using '/', which may produce invalid
    /// paths when combined with a verbatim Win32 prefix. Do not change the
    /// verified filesystem location, only the spelling passed to the SDK.
    ///
    /// The returned UTF-16 buffer has one trailing NUL and owns its storage
    /// until NVSDK_NGX_D3D11_Init returns.
    fn ngx_search_directory(root: &Path) -> anyhow::Result<(Vec<u16>, bool)> {
        anyhow::ensure!(root.is_absolute(), "NGX runtime directory must be absolute");
        let mut path: Vec<u16> = root.as_os_str().encode_wide().collect();
        anyhow::ensure!(!path.contains(&0), "NGX runtime directory contains a NUL");
        // Match the Windows separator spelling expected by NGX's path builder.
        for unit in &mut path {
            if *unit == b'/' as u16 {
                *unit = b'\\' as u16;
            }
        }
        const SLASH: u16 = b'\\' as u16;
        const VERBATIM: [u16; 4] = [SLASH, SLASH, b'?' as u16, SLASH];
        const VERBATIM_UNC: [u16; 8] = [
            SLASH,
            SLASH,
            b'?' as u16,
            SLASH,
            b'U' as u16,
            b'N' as u16,
            b'C' as u16,
            SLASH,
        ];
        let mut removed_prefix = false;
        if path.starts_with(&VERBATIM_UNC) {
            let mut normal = vec![SLASH, SLASH];
            normal.extend_from_slice(&path[VERBATIM_UNC.len()..]);
            path = normal;
            removed_prefix = true;
        } else if path.starts_with(&VERBATIM) {
            path.drain(..VERBATIM.len());
            removed_prefix = true;
        }
        let drive = path.len() >= 3
            && matches!(path[0], 65..=90 | 97..=122)
            && path[1] == b':' as u16
            && path[2] == SLASH;
        let unc = path.len() > 4
            && path.starts_with(&[SLASH, SLASH])
            && path[2] != SLASH
            && path[2..].contains(&SLASH);
        anyhow::ensure!(
            drive || unc,
            "NGX runtime directory has an unsupported path form"
        );
        // The SDK may use legacy Win32 file APIs. Avoid handing it a normal
        // path that already exceeds traditional MAX_PATH and cannot resolve.
        anyhow::ensure!(
            path.len() < 248,
            "NGX runtime directory exceeds Win32 path limit"
        );
        path.push(0);
        Ok((path, removed_prefix))
    }

    #[cfg(test)]
    mod search_directory_tests {
        use super::*;

        fn decode(wide: &[u16]) -> String {
            assert_eq!(wide.last().copied(), Some(0));
            String::from_utf16(&wide[..wide.len() - 1]).unwrap()
        }

        #[test]
        fn canonical_drive_directory_becomes_normal_windows_path() {
            let (wide, converted) = ngx_search_directory(Path::new(
                r"\\?\C:\Users\Test\AppData\Local\EvoParseq\vsr\1.1.0",
            ))
            .unwrap();
            assert!(converted);
            assert_eq!(
                decode(&wide),
                r"C:\Users\Test\AppData\Local\EvoParseq\vsr\1.1.0"
            );
        }

        #[test]
        fn canonical_unc_directory_retains_server_and_share() {
            let (wide, converted) =
                ngx_search_directory(Path::new(r"\\?\UNC\server\share\NGX")).unwrap();
            assert!(converted);
            assert_eq!(decode(&wide), r"\\server\share\NGX");
        }

        #[test]
        fn ordinary_path_is_preserved_and_slashes_are_normalized() {
            let (wide, converted) = ngx_search_directory(Path::new("C:/Users/Test/VSR")).unwrap();
            assert!(!converted);
            assert_eq!(decode(&wide), r"C:\Users\Test\VSR");
        }

        #[test]
        fn unicode_path_and_invalid_forms_are_handled_without_lossy_encoding() {
            let (wide, _) = ngx_search_directory(Path::new(r"\\?\C:\Felhasználó\VSR")).unwrap();
            assert_eq!(decode(&wide), r"C:\Felhasználó\VSR");
            assert!(ngx_search_directory(Path::new(r"relative\VSR")).is_err());
            assert!(ngx_search_directory(Path::new(r"\\?\Volume{0000}\VSR")).is_err());
        }
    }

    // The private bridge v3 ABI retains an optional callback argument, but
    // production passes None: no vendor SDK messages are collected or logged.
    type SdkLogCallback = unsafe extern "C" fn(*const c_char, u32, u32);

    #[repr(C)]
    #[derive(Default)]
    struct CreateDiagnostics {
        stage: u32,
        sdk_result: u32,
        available: i32,
    }
    type Create = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *const u16,
        *mut *mut c_void,
        *mut CreateDiagnostics,
        Option<SdkLogCallback>,
    ) -> u32;
    type Evaluate = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        *const i32,
        u32,
    ) -> u32;
    type Destroy = unsafe extern "C" fn(*mut c_void);
    type Abi = unsafe extern "C" fn() -> u32;

    struct Library(*mut c_void);
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
    impl Library {
        unsafe fn open(path: &std::path::Path) -> anyhow::Result<Self> {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let module = LoadLibraryExW(
                wide.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            );
            anyhow::ensure!(
                !module.is_null(),
                "Cannot load the verified NVIDIA NGX bridge"
            );
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
            unsafe {
                self.0.Leave();
            }
        }
    }

    pub struct Instance {
        device: ID3D11Device,
        _library: Library,
        session: *mut c_void,
        evaluate: Evaluate,
        destroy: Destroy,
    }
    impl Instance {
        pub unsafe fn new(device: &ID3D11Device) -> anyhow::Result<Self> {
            let root = crate::vsr_runtime::validated_directory()?;
            let library = Library::open(&crate::vsr_runtime::validated_bridge_path()?)?;
            let abi: Abi = library.symbol(b"evoparseq_ngx_bridge_abi\0")?;
            anyhow::ensure!(abi() == 3, "Unsupported NVIDIA NGX bridge ABI");
            let create: Create = library.symbol(b"evoparseq_ngx_create\0")?;
            let evaluate: Evaluate = library.symbol(b"evoparseq_ngx_evaluate\0")?;
            let destroy: Destroy = library.symbol(b"evoparseq_ngx_destroy\0")?;
            // Use a normal absolute Win32 path when passing the *verified*
            // runtime directory into the NVIDIA SDK; leave the bridge path and
            // on-disk SHA-256 checks unchanged.
            let (path, _) = ngx_search_directory(&root)?;
            let context = device.GetImmediateContext()?;
            let _lock = ContextLock::enter(device)?;
            let mut session = null_mut();
            let mut diagnostics = CreateDiagnostics::default();
            // Preserve the original direct-SDK init inputs: app=0,
            // API=0x15, D3D11 device/context and the same verified path list.
            let created = create(
                device.as_raw(),
                context.as_raw(),
                path.as_ptr(),
                &mut session,
                &mut diagnostics,
                None,
            );
            if created != 1 || session.is_null() {
                let stage = match diagnostics.stage {
                    1 => "invalid bridge ABI input",
                    2 => "NGX D3D11 initialization",
                    3 => "NGX D3D11 capability parameters",
                    4 => "NGX VSR availability query",
                    5 => "VSR.Available is false",
                    6 => "NGX VSR feature creation",
                    _ => "unknown NGX bridge initialization failure",
                };
                anyhow::bail!(
                    "NVIDIA NGX VSR failed at {stage}: status 0x{:08X}, available={}",
                    diagnostics.sdk_result,
                    diagnostics.available
                );
            }
            Ok(Self {
                device: device.clone(),
                _library: library,
                session,
                evaluate,
                destroy,
            })
        }
        pub unsafe fn evaluate(
            &self,
            context: &ID3D11DeviceContext,
            surfaces: &Surfaces,
            rect: [i32; 4],
            quality: u8,
        ) -> anyhow::Result<()> {
            let _lock = ContextLock::enter(&self.device)?;
            anyhow::ensure!(
                (self.evaluate)(
                    self.session,
                    context.as_raw(),
                    surfaces.input.as_raw(),
                    surfaces.output.as_raw(),
                    surfaces.key.0,
                    surfaces.key.1,
                    rect.as_ptr(),
                    u32::from(quality)
                ) == 1,
                "NVIDIA NGX VSR frame evaluation failed"
            );
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
