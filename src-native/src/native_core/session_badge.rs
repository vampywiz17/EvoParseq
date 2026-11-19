//! Local draggable launcher. Standard BUTTON keyboard/click behavior is retained;
//! a captured drag moves only this popup and never enters the remote input FIFO.
use super::{controls::wide, session_ui::OPEN};
use std::{
    ptr::{null, null_mut},
    sync::atomic::Ordering,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::ClientToScreen,
    System::LibraryLoader::*,
    UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

pub(super) const SIZE: i32 = crate::app_style::session::BADGE_SIZE;
const SUBCLASS: usize = 1;

#[derive(Clone, Copy)]
struct Drag {
    start: POINT,
    position: POINT,
    moved: bool,
}
struct Badge {
    owner: HWND,
    position: POINT,
    drag: Option<Drag>,
}

fn bounded(position: POINT, width: i32, height: i32) -> POINT {
    let [x, y] = crate::app_style::session::bounded([position.x, position.y], [width, height]);
    POINT { x, y }
}
unsafe fn screen_point(hwnd: HWND, lp: LPARAM) -> POINT {
    let mut point = POINT {
        x: lp as i16 as i32,
        y: (lp >> 16) as i16 as i32,
    };
    ClientToScreen(hwnd, &mut point);
    point
}

unsafe extern "system" fn button_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    let badge = GetParent(hwnd);
    let pointer = GetWindowLongPtrW(badge, GWLP_USERDATA) as *mut Badge;
    if !pointer.is_null() {
        match msg {
            #[cfg(test)]
            m if m == WM_APP + 90 => {
                // Execute a synthetic gesture on the owning UI thread so OS
                // capture-generated mouse moves cannot split the fixture.
                SendMessageW(hwnd, WM_LBUTTONDOWN, 1, wp as LPARAM);
                SendMessageW(hwnd, WM_MOUSEMOVE, 1, lp);
                SendMessageW(hwnd, WM_LBUTTONUP, 0, wp as LPARAM);
                return 0;
            }
            WM_LBUTTONDOWN => {
                (*pointer).drag = Some(Drag {
                    start: screen_point(hwnd, lp),
                    position: (*pointer).position,
                    moved: false,
                });
            }
            WM_MOUSEMOVE => {
                if let Some(mut drag) = (*pointer).drag {
                    let point = screen_point(hwnd, lp);
                    let dx = point.x - drag.start.x;
                    let dy = point.y - drag.start.y;
                    drag.moved |= dx.abs() >= GetSystemMetrics(SM_CXDRAG)
                        || dy.abs() >= GetSystemMetrics(SM_CYDRAG);
                    (*pointer).drag = Some(drag);
                    if drag.moved {
                        (*pointer).position = POINT {
                            x: drag.position.x + dx,
                            y: drag.position.y + dy,
                        };
                        SendMessageW(hwnd, BM_SETSTATE, 0, 0);
                        position(badge, None);
                        return 0;
                    }
                }
            }
            WM_LBUTTONUP => {
                let dragged = (*pointer).drag.take().is_some_and(|drag| drag.moved);
                if dragged {
                    SendMessageW(hwnd, BM_SETSTATE, 0, 0);
                    if GetCapture() == hwnd {
                        ReleaseCapture();
                    }
                    return 0;
                }
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                (*pointer).drag = None;
            }
            _ => {}
        }
    }
    if msg == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(button_proc), SUBCLASS);
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let owner = (*(lp as *const CREATESTRUCTW)).lpCreateParams as HWND;
        let state = Box::new(Badge {
            owner,
            position: POINT {
                x: crate::app_style::session::BADGE_START[0],
                y: crate::app_style::session::BADGE_START[1],
            },
            drag: None,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Badge;
    match msg {
        WM_COMMAND if !state.is_null() && wp & 65535 == OPEN && (wp >> 16) as u32 == BN_CLICKED => {
            SendMessageW((*state).owner, WM_COMMAND, OPEN, 0);
            return 0;
        }
        BM_CLICK => {
            SendMessageW(GetDlgItem(hwnd, OPEN as i32), BM_CLICK, 0, 0);
            return 0;
        }
        WM_DRAWITEM if super::ui::draw_button(lp) => return 1,
        WM_ERASEBKGND => return 1,
        WM_NCDESTROY if !state.is_null() => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(state));
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

pub(super) unsafe fn install(window: &crate::window::Window) {
    if window.native_session_button.load(Ordering::Acquire) != 0 {
        return;
    }
    let class = wide("EvoParseq.NativeSessionBadge");
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: GetModuleHandleW(null()),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: class.as_ptr(),
        ..std::mem::zeroed()
    });
    let parent = window.handle();
    let badge = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        class.as_ptr(),
        wide("Session").as_ptr(),
        WS_POPUP | WS_CLIPCHILDREN,
        16,
        16,
        SIZE,
        SIZE,
        parent,
        null_mut(),
        GetModuleHandleW(null()),
        parent.cast(),
    );
    if badge.is_null() {
        return;
    }
    // Keep the accessible name; owner drawing shows only the application icon.
    let button = CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide("Session").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        0,
        SIZE,
        SIZE,
        badge,
        OPEN as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    if button.is_null() || SetWindowSubclass(button, Some(button_proc), SUBCLASS, 0) == 0 {
        DestroyWindow(badge);
        return;
    }
    SetLayeredWindowAttributes(badge, 0, crate::app_style::session::BADGE_ALPHA, LWA_ALPHA);
    window
        .native_session_button
        .store(badge as usize, Ordering::Release);
}

pub(super) unsafe fn position(hwnd: HWND, insert_after: Option<HWND>) {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Badge;
    if pointer.is_null() {
        return;
    }
    let owner = (*pointer).owner;
    if IsIconic(owner) != 0 {
        return;
    }
    let mut bounds = std::mem::zeroed();
    GetClientRect(owner, &mut bounds);
    let point = bounded((*pointer).position, bounds.right, bounds.bottom);
    (*pointer).position = point;
    let mut origin = point;
    ClientToScreen(owner, &mut origin);
    SetWindowPos(
        hwnd,
        insert_after.unwrap_or(HWND_TOP),
        origin.x,
        origin.y,
        SIZE,
        SIZE,
        SWP_NOACTIVATE
            | SWP_NOOWNERZORDER
            | if insert_after.is_some() {
                0
            } else {
                SWP_NOZORDER
            },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launcher_stays_inside_resized_client_bounds() {
        for (point, size, expected) in [
            ((-80, -20), (800, 600), (0, 0)),
            ((900, 700), (800, 600), (752, 552)),
            ((20, 30), (800, 600), (20, 30)),
            ((20, 30), (30, 20), (0, 0)),
        ] {
            let actual = bounded(
                POINT {
                    x: point.0,
                    y: point.1,
                },
                size.0,
                size.1,
            );
            assert_eq!((actual.x, actual.y), expected);
        }
    }
}
