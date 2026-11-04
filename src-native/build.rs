use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rustc-check-cfg=cfg(nvidia_vsr_sdk)");
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"desktop-telemetry\"))");
    println!("cargo:rerun-if-env-changed=EVOPARSEQ_RTX_VIDEO_SDK");
    if env::var("TARGET").unwrap_or_default() == "x86_64-pc-windows-msvc" {
        if let Some(root) = env::var_os("EVOPARSEQ_RTX_VIDEO_SDK") {
            let root = fs::canonicalize(root).expect("RTX Video SDK folder missing");
            let runtime = root.join("bin/Windows/x64/rel/nvngx_vsr.dll");
            assert!(runtime.is_file(), "RTX VSR release runtime missing");
            assert!(
                root.join("lib/Windows/x64/nvsdk_ngx_s.lib").is_file(),
                "RTX Video static library missing"
            );
            println!("cargo:rustc-cfg=nvidia_vsr_sdk");
            println!(
                "cargo:rustc-link-search=native={}",
                root.join("lib/Windows/x64").display()
            );
            println!("cargo:rustc-link-lib=static=nvsdk_ngx_s");
            for library in [
                "advapi32", "shell32", "ole32", "shlwapi", "version", "wintrust", "crypt32",
                "dxgi", "d3d11",
            ] {
                println!("cargo:rustc-link-lib={library}");
            }
        }
    }

    println!("cargo:rerun-if-changed=../assets/evoparseq.ico");
    if !env::var("TARGET")
        .unwrap_or_default()
        .contains("windows-msvc")
    {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon = fs::canonicalize("../assets/evoparseq.ico").expect("Application icon missing");
    let icon = icon
        .display()
        .to_string()
        .trim_start_matches(r"\\?\")
        .replace('\\', "\\\\");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let tuple = format!(
        "{},{},{}",
        env::var("CARGO_PKG_VERSION_MAJOR").unwrap(),
        env::var("CARGO_PKG_VERSION_MINOR").unwrap(),
        env::var("CARGO_PKG_VERSION_PATCH").unwrap()
    );
    let resource = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {tuple},0
PRODUCTVERSION {tuple},0
FILETYPE 1
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "FileDescription", "EvoParseq native client\0"
   VALUE "ProductName", "EvoParseq\0"
   VALUE "FileVersion", "{version}\0"
   VALUE "ProductVersion", "{version}\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x0409, 1200
 END
END
"#
    );
    let source = output.join("application.rc");
    let compiled = output.join("application.res");
    fs::write(&source, resource).unwrap();
    let compiler = env::var_os("RC").map(PathBuf::from).unwrap_or_else(|| {
        let root =
            PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK location missing"))
                .join("Windows Kits/10/bin");
        let mut candidates: Vec<_> = fs::read_dir(root)
            .expect("Install the Windows SDK resource compiler")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("x64/rc.exe"))
            .filter(|path| path.is_file())
            .collect();
        candidates.sort();
        candidates.pop().expect("Windows SDK rc.exe missing")
    });
    assert!(
        Command::new(compiler)
            .arg("/nologo")
            .arg("/fo")
            .arg(&compiled)
            .arg(source)
            .status()
            .expect("Resource compiler failed")
            .success(),
        "Windows resources failed"
    );
    println!("cargo:rustc-link-arg={}", compiled.display());
}
