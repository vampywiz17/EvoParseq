//! Shared session commands and presentation state, independent of window APIs.
//! Native views own their controls and notifications; media workers stay separate.
use super::{host_displays::Displays, profile::Video};
#[cfg(test)]
use crate::session_preferences::{number, resolution, MAX_BANDWIDTH_MBPS, MAX_FPS};
#[cfg(test)]
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Clone, Debug)]
pub enum Event {
    NativeSession(Command),
    Shortcut(crate::input::Shortcut),
    // Keep the platform event payload; host-size matching queries the current
    // viewport rather than using possibly stale dimensions from the input FIFO.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "resize handling reads the current viewport")
    )]
    Size(i32, i32),
    Focus(bool),
    Clipboard,
    // Local scheduling hint only; never translated into remote mouse input.
    CursorReady,
    // Local fail-safe; never translated into remote input.
    InputOverflow,
    Motion(i32, i32),
    RelativeMotion(i32, i32),
    RelativeMode(bool),
    Button(bool, i32, i32, i32),
    // Character events remain part of the platform contract and its UTF-16
    // tests. Remote typing uses physical USB usages and the host's layout.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "remote typing uses physical key events")
    )]
    Text(u32),
    Key(bool, &'static str, i32),
    Scroll(i32, i32),
    Fullscreen(bool),
    #[cfg(feature = "bud")]
    HidInputRelease,
}

impl Event {
    pub(crate) fn is_motion(&self) -> bool {
        matches!(self, Self::Motion(..) | Self::RelativeMotion(..))
    }

    /// Only adjacent movement in the same coordinate mode is replaceable.
    /// Checked sums preserve every relative delta, including integer extremes.
    pub(crate) fn merge_motion(&mut self, next: &Self) -> bool {
        match (self, next) {
            (Self::Motion(x, y), Self::Motion(nx, ny)) => {
                (*x, *y) = (*nx, *ny);
                true
            }
            (Self::RelativeMotion(x, y), Self::RelativeMotion(nx, ny)) => {
                if let (Some(sx), Some(sy)) = (x.checked_add(*nx), y.checked_add(*ny)) {
                    (*x, *y) = (sx, sy);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

pub use crate::session_controls::{Command, Status};

pub use crate::session_preferences::{
    current_choice, resolution_text, PanelFields, BANDWIDTH_CHOICES, FPS_CHOICES, HOST_RESOLUTION,
    RESOLUTION_CHOICES,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notification {
    Status(Status),
    DisplaysChanged,
    DisplaySelected,
}

/// Only the services needed by the host-settings controller. A view translates
/// notifications to its own event mechanism; it never drives the video worker.
pub trait View {
    fn session(&self) -> &State;
    fn display_size(&self) -> (u32, u32);
    fn notify(&self, notification: Notification);

    fn waiting(&self) {
        self.session().video_pending.store(true, Ordering::Release);
        self.notify(Notification::Status(Status::Waiting));
    }
    fn finished(&self, confirmed: bool) {
        self.session().video_pending.store(false, Ordering::Release);
        self.notify(Notification::Status(if confirmed {
            Status::Confirmed
        } else {
            Status::Unconfirmed
        }));
    }
    fn result(&self, ok: bool) {
        self.notify(Notification::Status(if ok {
            Status::Applied
        } else {
            Status::Failed
        }));
    }
}

pub use crate::session_preferences::HostVideo;

/// Keeps the existing lock/atomic boundaries; no new queue, worker or polling.
#[derive(Default)]
pub struct State {
    pub host_video: Mutex<HostVideo>,
    pub displays: Mutex<Displays>,
    pub video_pending: AtomicBool,
    pub hid_primary_ready: AtomicBool,
}
impl State {
    pub fn panel_video(&self, saved: Video) -> Video {
        self.host_video
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .panel_video(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_core::profile::Resolution;

    #[test]
    fn session_fields_keep_resolution_aliases_boundaries_and_local_preferences() {
        for resolution_value in [
            Resolution::Host,
            Resolution::Client,
            Resolution::Fixed(3440, 1440),
        ] {
            let rendered = resolution_text(resolution_value);
            let fields = PanelFields {
                resolution: &rendered,
                bandwidth: " 100 ",
                fps: "240",
                constant_fps: true,
                rotated: false,
                immersive: 3,
                fullscreen: true,
            };
            let video = fields.parse().unwrap();
            assert_eq!(video.resolution, resolution_value);
            assert_eq!(video.bandwidth_mbps, Some(MAX_BANDWIDTH_MBPS));
            assert_eq!(video.fps, Some(MAX_FPS));
            assert_eq!(video.constant_fps, Some(true));
            assert_eq!(video.rotated, Some(false));
            assert_eq!(video.immersive, 3);
            assert!(video.fullscreen);
        }
        assert_eq!(
            resolution(" 3440 × 1440 ").unwrap(),
            Resolution::Fixed(3440, 1440)
        );
        let mut fields = PanelFields {
            resolution: "1920 x 1080",
            bandwidth: "10",
            fps: "60",
            constant_fps: false,
            rotated: true,
            immersive: 0,
            fullscreen: false,
        };
        for bad in ["Keep host", "", "-1", "101", "4294967296"] {
            fields.bandwidth = bad;
            assert!(fields.parse().is_err());
        }
        fields.bandwidth = "10";
        for bad in ["Keep host", "", "0", "241"] {
            fields.fps = bad;
            assert!(fields.parse().is_err());
        }
        fields.fps = "60";
        for bad in ["wrong", "0 x 1080", "8193 x 1080", "1920 x 199"] {
            fields.resolution = bad;
            assert!(fields.parse().is_err());
        }
        fields.resolution = HOST_RESOLUTION;
        fields.immersive = 4;
        assert!(fields.parse().is_err());
    }
    #[test]
    fn session_feedback_and_parse_errors_do_not_depend_on_native_message_ids() {
        assert_eq!(
            number("Keep host", 100).unwrap_err().to_string(),
            "Enter a number."
        );
        assert_eq!(
            number("101", 100).unwrap_err().to_string(),
            "Value must be 1–100."
        );
        assert_eq!(
            resolution("wrong").unwrap_err().to_string(),
            "Enter resolution as width x height."
        );
        assert!(Status::Applied.text().is_empty());
        assert!(Status::Confirmed.text().is_empty());
        assert_eq!(Status::Waiting.text(), "Applying settings...");
        assert!(Status::Unconfirmed.text().contains("Host did not confirm"));
        assert_eq!(
            Status::AttentionSent.text(),
            "Ctrl+Alt+Del requested from the host."
        );
        assert!(Status::AttentionFailed.text().contains("Could not send"));
        assert!(Status::Failed.text().contains("Could not apply or save"));
    }

    #[test]
    fn reopening_panel_preserves_saved_requests_and_local_controls() {
        let state = State::default();
        *state.host_video.lock().unwrap() = HostVideo::from_report(&serde_json::json!({
            "bandwidth_mbps":80,"fps":144,"constant_fps":false,"rotated":true
        }));
        let saved = Video {
            resolution: Resolution::Fixed(1920, 1080),
            bandwidth_mbps: Some(50),
            fps: Some(60),
            constant_fps: Some(true),
            rotated: Some(false),
            immersive: 3,
            fullscreen: true,
        };
        assert_eq!(state.panel_video(saved.clone()), saved);
        let host = state.panel_video(Video::default());
        assert_eq!(host.bandwidth_mbps, Some(80));
        assert_eq!(host.fps, Some(144));
        assert_eq!(host.constant_fps, Some(false));
        assert_eq!(host.rotated, Some(true));
        assert_eq!(host.resolution, Resolution::Host);
        assert!(!host.fullscreen);
    }

    #[test]
    fn unknown_or_out_of_range_host_preferences_keep_existing_panel_defaults() {
        for report in [
            Value::Null,
            serde_json::json!({"bandwidth_mbps":0,"fps":241,"constant_fps":1,"rotated":"true"}),
            serde_json::json!({"bandwidth_mbps":101,"fps":-1}),
            serde_json::json!({"bandwidth_mbps":u64::MAX,"fps":0}),
        ] {
            let host = HostVideo::from_report(&report);
            assert_eq!(host, HostVideo::default());
            let panel = host.panel_video(Video::default());
            assert_eq!(panel.bandwidth_mbps, Some(10));
            assert_eq!(panel.fps, Some(60));
            assert_eq!(panel.constant_fps, Some(false));
            assert_eq!(panel.rotated, Some(false));
            assert!(panel.validate().is_ok());
        }
    }
}
