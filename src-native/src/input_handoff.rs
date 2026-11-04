//! One bounded UI-to-session handoff for controllers, keyboards and pointing devices.
use crate::{desktop_input, gamepad};
use std::{collections::VecDeque, sync::Mutex};
#[derive(Clone, Debug)]
#[cfg_attr(not(target_os = "linux"), derive(Copy))]
pub enum Event {
    Controller(gamepad::Event),
    Desktop(desktop_input::Event),
    #[cfg(target_os = "linux")]
    Session(crate::session_controls::Command),
}
#[cfg(target_os = "linux")]
impl From<crate::session_controls::Command> for Event {
    fn from(command: crate::session_controls::Command) -> Self {
        Self::Session(command)
    }
}
impl From<gamepad::Event> for Event {
    fn from(e: gamepad::Event) -> Self {
        Self::Controller(e)
    }
}
impl From<desktop_input::Event> for Event {
    fn from(e: desktop_input::Event) -> Self {
        Self::Desktop(e)
    }
}
#[derive(Default)]
pub struct Inbox(Mutex<(VecDeque<Event>, bool)>);
impl Inbox {
    pub fn push(&self, event: impl Into<Event>) {
        let event = event.into();
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if q.1 {
            return;
        }
        match (q.0.back_mut(), &event) {
            (
                Some(Event::Controller(gamepad::Event::Axes(id, old, hat))),
                Event::Controller(gamepad::Event::Axes(next, state, nhat)),
            ) if *id == *next
                && *hat == *nhat
                && (old.lt == 0) == (state.lt == 0)
                && (old.rt == 0) == (state.rt == 0) =>
            {
                *old = *state;
                return;
            }
            (
                Some(Event::Desktop(desktop_input::Event::Pointer(old))),
                Event::Desktop(desktop_input::Event::Pointer(next)),
            ) => {
                if old.merge(*next) {
                    return;
                }
            }
            _ => {}
        }
        if q.0.len() == 128 {
            q.0.clear();
            q.1 = true;
        } else {
            q.0.push_back(event);
        }
    }
    pub fn take(&self, into: &mut VecDeque<Event>) -> anyhow::Result<()> {
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        anyhow::ensure!(
            !q.1,
            "Input queue overloaded. Input was released; reconnect to continue."
        );
        std::mem::swap(&mut q.0, into);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mouse_bursts_sum_relative_motion_without_losing_click_edges() {
        let q = Inbox::default();
        for _ in 0..1000 {
            q.push(desktop_input::Event::Pointer(desktop_input::Pointer {
                x: 1.,
                y: -2.,
                relative: true,
                moved: true,
                viewport: (1920, 1080),
                buttons: 0,
                wheel: (0, 0),
            }));
        }
        q.push(desktop_input::Event::Pointer(desktop_input::Pointer {
            x: 0.,
            y: 0.,
            relative: true,
            moved: false,
            viewport: (1920, 1080),
            buttons: 1,
            wheel: (0, 0),
        }));
        q.push(desktop_input::Event::Pointer(desktop_input::Pointer {
            x: 0.,
            y: 0.,
            relative: true,
            moved: false,
            viewport: (1920, 1080),
            buttons: 0,
            wheel: (0, 0),
        }));
        let mut batch = VecDeque::new();
        q.take(&mut batch).unwrap();
        assert_eq!(batch.len(), 3);
        assert!(
            matches!(batch[0],Event::Desktop(desktop_input::Event::Pointer(p)) if p.x==1000.&&p.y== -2000.)
        );
    }
}
