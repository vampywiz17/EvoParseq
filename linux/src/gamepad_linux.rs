//! Linux device adapter through the runtime's documented SDL2 controller API
//! (Freedesktop 25.08 supplies SDL2-compat/SDL3). No window/video subsystem,
//! gamepad mapping policy, transport, polling thread or bundled SDK here.
use crate::gamepad::{self, Motors, State};
use std::{
    ffi::c_void,
    ptr::NonNull,
    time::{Duration, Instant},
};
#[link(name = "SDL2")]
unsafe extern "C" {
    fn SDL_InitSubSystem(flags: u32) -> i32;
    fn SDL_QuitSubSystem(flags: u32);
    fn SDL_NumJoysticks() -> i32;
    fn SDL_IsGameController(index: i32) -> i32;
    fn SDL_GameControllerOpen(index: i32) -> *mut c_void;
    fn SDL_GameControllerClose(pad: *mut c_void);
    fn SDL_GameControllerGetAttached(pad: *mut c_void) -> i32;
    fn SDL_GameControllerGetJoystick(pad: *mut c_void) -> *mut c_void;
    fn SDL_JoystickInstanceID(joystick: *mut c_void) -> i32;
    fn SDL_JoystickGetDeviceInstanceID(index: i32) -> i32;
    fn SDL_GameControllerUpdate();
    fn SDL_GameControllerGetAxis(pad: *mut c_void, axis: i32) -> i16;
    fn SDL_GameControllerGetButton(pad: *mut c_void, button: i32) -> u8;
    fn SDL_GameControllerRumble(pad: *mut c_void, low: u16, high: u16, duration: u32) -> i32;
}
const SUBSYSTEM: u32 = 0x2000;
struct Pad {
    raw: NonNull<c_void>,
    id: i32,
}
impl Drop for Pad {
    fn drop(&mut self) {
        unsafe {
            SDL_GameControllerRumble(self.raw.as_ptr(), 0, 0, 0);
            SDL_GameControllerClose(self.raw.as_ptr());
        }
    }
}
pub struct Devices {
    pads: [Option<Pad>; 4],
    discovery: Instant,
    updated: Instant,
}
impl Devices {
    pub fn new() -> anyhow::Result<Self> {
        anyhow::ensure!(
            unsafe { SDL_InitSubSystem(SUBSYSTEM) } == 0,
            "Controller subsystem unavailable"
        );
        let now = Instant::now();
        Ok(Self {
            pads: std::array::from_fn(|_| None),
            discovery: now,
            updated: now,
        })
    }
    fn update(&mut self) {
        let now = Instant::now();
        if now >= self.updated {
            unsafe { SDL_GameControllerUpdate() };
            self.updated = now + gamepad::POLL;
        }
        if now < self.discovery {
            return;
        }
        self.discovery = now + Duration::from_secs(1);
        let count = unsafe { SDL_NumJoysticks() }.clamp(0, 64);
        for index in 0..count {
            let id = unsafe { SDL_JoystickGetDeviceInstanceID(index) };
            if id < 0
                || self.pads.iter().flatten().any(|pad| pad.id == id)
                || unsafe { SDL_IsGameController(index) } == 0
            {
                continue;
            }
            let Some(slot) = self.pads.iter_mut().find(|pad| pad.is_none()) else {
                break;
            };
            if let Some(raw) = NonNull::new(unsafe { SDL_GameControllerOpen(index) }) {
                let id =
                    unsafe { SDL_JoystickInstanceID(SDL_GameControllerGetJoystick(raw.as_ptr())) };
                *slot = Some(Pad { raw, id });
            }
        }
    }
}
impl gamepad::polling::Devices for Devices {
    fn read(&mut self, index: u32) -> Option<State> {
        let i = usize::try_from(index).ok().filter(|i| *i < 4)?;
        if self.pads[i]
            .as_ref()
            .is_some_and(|p| unsafe { SDL_GameControllerGetAttached(p.raw.as_ptr()) } == 0)
        {
            self.pads[i] = None;
            return None; // Shared manager observes unplug before reusing this slot.
        }
        self.update();
        let raw = self.pads[i].as_ref()?.raw.as_ptr();
        let axis = |id, flat, invert| {
            let v = unsafe { SDL_GameControllerGetAxis(raw, id) };
            let value = f32::from(v) / if v < 0 { 32768. } else { 32767. };
            gamepad::stick(if invert { -value } else { value }, flat)
        };
        let mut buttons = 0;
        for (button, mask) in [
            (0, 0x1000),
            (1, 0x2000),
            (2, 0x4000),
            (3, 0x8000),
            (4, 0x20),
            (5, 0x400),
            (6, 0x10),
            (7, 0x40),
            (8, 0x80),
            (9, 0x100),
            (10, 0x200),
            (11, 1),
            (12, 2),
            (13, 4),
            (14, 8),
        ] {
            if unsafe { SDL_GameControllerGetButton(raw, button) } != 0 {
                buttons |= mask
            }
        }
        Some(State {
            buttons,
            lx: axis(0, 7849. / 32767., false),
            ly: axis(1, 7849. / 32767., true),
            rx: axis(2, 8689. / 32767., false),
            ry: axis(3, 8689. / 32767., true),
            lt: gamepad::trigger(
                f32::from(unsafe { SDL_GameControllerGetAxis(raw, 4) }).max(0.) / 32767.,
                30. / 255.,
            ),
            rt: gamepad::trigger(
                f32::from(unsafe { SDL_GameControllerGetAxis(raw, 5) }).max(0.) / 32767.,
                30. / 255.,
            ),
        })
    }
}
impl Motors for Devices {
    fn rumble(&mut self, id: i32, low: u8, high: u8) {
        if let Some(pad) = usize::try_from(id)
            .ok()
            .and_then(|id| self.pads.get(id))
            .and_then(Option::as_ref)
        {
            unsafe {
                SDL_GameControllerRumble(
                    pad.raw.as_ptr(),
                    u16::from(low) * 257,
                    u16::from(high) * 257,
                    if low == 0 && high == 0 { 0 } else { u32::MAX },
                );
            }
        }
    }
}
impl Drop for Devices {
    fn drop(&mut self) {
        for pad in &mut self.pads {
            *pad = None;
        }
        unsafe { SDL_QuitSubSystem(SUBSYSTEM) };
    }
}
