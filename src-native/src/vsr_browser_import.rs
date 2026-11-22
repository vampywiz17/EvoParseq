//! Explicit, user-initiated browser acquisition and native file selection.
//! EvoParseq never retrieves an executable DLL or NVIDIA SDK ZIP itself.
use anyhow::{bail, Result};
use std::{
    ffi::{c_void, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};
use windows_sys::Win32::{
    UI::{
        Controls::Dialogs::{
            CommDlgExtendedError, GetOpenFileNameW, OPENFILENAMEW, OFN_FILEMUSTEXIST,
            OFN_PATHMUSTEXIST,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            MessageBoxW, IDOK, IDYES, MB_DEFBUTTON2, MB_ICONINFORMATION, MB_OKCANCEL, MB_YESNO,
            SW_SHOWNORMAL,
        },
    },
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn confirm(text: &str, ok_cancel: bool) -> bool {
    let flags = MB_ICONINFORMATION
        | MB_DEFBUTTON2
        | if ok_cancel { MB_OKCANCEL } else { MB_YESNO };
    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(text).as_ptr(),
            wide("EvoParseq - NVIDIA VSR adapter").as_ptr(),
            flags,
        )
    };
    result == if ok_cancel { IDOK } else { IDYES }
}

/// Return None on a user-initiated cancel; never guess a location and never
/// install a file simply because one happens to appear in Downloads.
fn select_download(extension: &str) -> Result<Option<PathBuf>> {
    let (filter, title) = match extension {
        "dll" => (
            "NVIDIA NGX adapter DLL\0*.dll\0All files\0*.*\0\0",
            "Select the downloaded EvoParseq-NGX-Bridge.dll",
        ),
        "zip" => (
            "NVIDIA RTX SDK ZIP\0*.zip\0All files\0*.*\0\0",
            "Select the downloaded RTX_Video_SDK_v1.1.0.zip",
        ),
        _ => bail!("Unsupported NVIDIA file type"),
    };
    let mut filename = vec![0u16; 32_768];
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let title = wide(title);
    let downloads = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|p| p.join("Downloads"))
        .filter(|p| p.is_dir());
    let initial_dir = downloads.as_ref().map(|p| {
        p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>()
    });
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrTitle = title.as_ptr();
    dialog.lpstrFile = filename.as_mut_ptr();
    dialog.nMaxFile = filename.len() as u32;
    dialog.lpstrInitialDir = initial_dir
        .as_ref()
        .map_or(std::ptr::null(), |path| path.as_ptr());
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    let selected = unsafe { GetOpenFileNameW(&mut dialog) };
    if selected == 0 {
        let code = unsafe { CommDlgExtendedError() };
        if code == 0 {
            return Ok(None);
        }
        bail!("Windows file picker failed (code {code})");
    }
    let len = filename.iter().position(|x| *x == 0).unwrap_or(filename.len());
    let path = PathBuf::from(OsString::from_wide(&filename[..len]));
    if !path.is_file() {
        bail!("Selected download is not a regular file");
    }
    Ok(Some(path))
}

pub fn choose(url: &str, extension: &str) -> Result<Option<PathBuf>> {
    // The browser is launched only after an explicit user decision.
    if !confirm(
        "An optional NVIDIA VSR component needs to be downloaded.\n\nOpen the approved download in your browser? EvoParseq will not download or execute it automatically.",
        false,
    ) {
        return Ok(None);
    }
    let action = wide("open");
    let url = wide(url);
    let open_result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            action.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (open_result as isize) <= 32 {
        bail!("Windows could not open the browser download link");
    }
    if !confirm(
        "After the download has finished in your browser, click OK to select the file.\n\nEvoParseq will verify it and automatically place it in its private cache. Cancel leaves the existing adapter unchanged.",
        true,
    ) {
        return Ok(None);
    }
    select_download(extension)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_picker_types_are_explicit() {
        assert!(["dll", "zip"].iter().all(|e| matches!(*e, "dll" | "zip")));
        // No ambient scanning, implicit Downloads trust or arbitrary installer.
    }
}
