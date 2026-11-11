//! Shared Win32 primitives for the native picker and session panel.
use std::ptr::null;
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*,
};

pub(super) const BG: u32 = crate::branding::BACKGROUND;
pub(super) const FIELD: u32 = crate::branding::colorref(crate::app_style::FIELD);
pub(super) const FG: u32 = crate::branding::colorref(crate::app_style::TEXT);

pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub(super) unsafe fn text(hwnd: HWND, limit: i32) -> String {
    let length = GetWindowTextLengthW(hwnd).clamp(0, limit) as usize;
    // Account controls can contain passwords/OTP codes. Clear the temporary
    // UTF-16 buffer even when this helper is used for ordinary menu controls.
    let mut buffer = zeroize::Zeroizing::new(vec![0u16; length + 1]);
    let used = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&buffer[..used])
}

pub(super) unsafe fn font(size: i32, weight: i32) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide("Segoe UI").as_ptr(),
    )
}

pub(super) unsafe fn create(
    parent: HWND,
    font: HFONT,
    class: &str,
    label: &str,
    id: usize,
    style: u32,
    rect: [i32; 4],
) -> HWND {
    let hwnd = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(label).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        rect[0],
        rect[1],
        rect[2],
        rect[3],
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(hwnd, WM_SETFONT, font as usize, 0);
    hwnd
}
