//! Shared application colors and embedded icons; no runtime assets or per-frame decoding.
use windows_sys::Win32::{System::LibraryLoader::*, UI::WindowsAndMessaging::*};

pub const fn colorref(rgb: [u8; 3]) -> u32 {
    rgb[0] as u32 | ((rgb[1] as u32) << 8) | ((rgb[2] as u32) << 16)
}
pub const BACKGROUND: u32 = colorref(crate::app_style::BACKGROUND);
pub const BLUE: u32 = colorref(crate::app_style::BLUE);

pub unsafe fn icon() -> HICON {
    // LoadIcon returns a shared resource owned by Windows, not by the caller.
    LoadIconW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance(1),
    )
}

pub unsafe fn launcher_icon() -> HICON {
    // Use the embedded 40 px resource; Windows owns this shared handle.
    LoadImageW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance(1),
        IMAGE_ICON,
        40,
        40,
        LR_SHARED,
    ) as HICON
}
