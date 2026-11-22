use std::{env, path::PathBuf};

fn main() {
    let sdk = PathBuf::from(env::var_os("EVOPARSEQ_RTX_VIDEO_SDK")
        .expect("The optional NGX bridge requires EVOPARSEQ_RTX_VIDEO_SDK"));
    let lib = sdk.join("lib/Windows/x64/nvsdk_ngx_s.lib");
    assert!(lib.is_file(), "Missing NVIDIA NGX static SDK library");
    println!("cargo:rustc-link-search=native={}", lib.parent().unwrap().display());
    println!("cargo:rustc-link-lib=static=nvsdk_ngx_s");
    for name in ["advapi32", "shell32", "ole32", "shlwapi", "version",
                 "wintrust", "crypt32", "dxgi", "d3d11"] {
        println!("cargo:rustc-link-lib={name}");
    }
}
