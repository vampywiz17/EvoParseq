//! Host settings transactions only; decoding/presentation stay on their workers.
use super::transport::Attempt;
use super::{
    host_displays, host_video,
    profile::Video,
    session_model::{HostVideo, Notification, Status, View},
};
use anyhow::{ensure, Result};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

struct DisplayResolution {
    stream: usize,
    resolution: crate::session_preferences::Resolution,
    size: (u32, u32),
    sent: bool,
}

#[derive(Default)]
pub(super) struct HostSettings {
    config: host_video::Config,
    secondary: Option<DisplayResolution>,
    query_deadline: Option<Instant>,
    confirmation: Option<host_video::Confirmation>,
    pending: Option<Video>,
    requested_display: Option<(u32, u32)>,
    pending_output: Option<String>,
    outputs_deadline: Option<Instant>,
}

impl HostSettings {
    pub fn new(video: &Video, view: &impl View) -> Self {
        view.session()
            .hid_primary_ready
            .store(false, Ordering::Release);
        let pending = video.has_host_settings().then(|| video.clone());
        if pending.is_some() {
            view.waiting();
        }
        Self {
            pending,
            ..Default::default()
        }
    }

    pub fn busy(&self) -> bool {
        self.secondary.is_some()
            || self.confirmation.is_some()
            || self.pending.is_some()
            || self.pending_output.is_some()
    }

    pub fn connected(&mut self, attempt: &Attempt, view: &impl View) -> Result<()> {
        self.query_outputs(attempt, view);
        self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
        if attempt.send(&host_video::query()?).is_err() {
            crate::event_log::record(
                "WARN",
                "native_host_video_query_failed",
                serde_json::json!({}),
            );
        }
        Ok(())
    }

    pub fn receive(&mut self, bytes: &[u8], attempt: &Attempt, view: &impl View) {
        self.query_deadline = None;
        if self.config.receive(bytes).is_err() {
            self.hid_primary_ready(attempt, view, false);
            self.pending = None;
            self.pending_output = None;
            self.finish_secondary(view, Status::Failed, false);
            crate::event_log::record(
                "WARN",
                "native_host_video_configuration_invalid",
                host_video::diagnostic(bytes),
            );
            if self.confirmation.is_none() {
                self.failed(view);
            }
            return;
        }

        view.session()
            .displays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current = self.config.output_id().map(str::to_owned);
        view.notify(Notification::DisplaysChanged);
        if !self.send_output(attempt, view) {
            return;
        }
        let observed = self.observed(view);
        let host_video = HostVideo::from_report(&observed);
        crate::event_log::record("INFO", "native_host_video_configuration", observed);
        *view
            .session()
            .host_video
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = host_video;
        if !self.ensure_hid_primary(attempt, view) {
            return;
        }
        if !self.busy() {
            view.notify(Notification::Status(Status::Confirmed));
        }
        if self.confirmation.is_none() {
            if self.secondary.is_some() {
                self.send_secondary(attempt, view);
            } else {
                self.send_pending(attempt, view);
            }
        }
    }

    pub fn display_resolution(
        &mut self,
        attempt: &Attempt,
        view: &impl View,
        stream: usize,
        resolution: crate::session_preferences::Resolution,
        size: (u32, u32),
    ) -> Result<()> {
        ensure!(
            (1..3).contains(&stream),
            "Invalid additional display stream"
        );
        ensure!(
            !self.busy() && self.query_deadline.is_none() && self.outputs_deadline.is_none(),
            "Waiting for the previous host request"
        );
        Video {
            resolution,
            ..Default::default()
        }
        .validate()?;
        // Refresh the complete configuration before patching one stream. Never
        // write a cached snapshot over another window's recent changes.
        attempt.send(&host_video::query()?)?;
        self.secondary = Some(DisplayResolution {
            stream,
            resolution,
            size,
            sent: false,
        });
        self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
        view.waiting();
        view.notify(Notification::DisplayResolution {
            stream,
            status: Status::Waiting,
            resolution: None,
        });
        Ok(())
    }
    fn send_secondary(&mut self, attempt: &Attempt, view: &impl View) {
        let Some(request) = self.secondary.as_mut().filter(|r| !r.sent) else {
            return;
        };
        let stream = request.stream;
        let result = (|| -> Result<()> {
            let frame = self
                .config
                .resolution(stream, request.resolution, request.size)?;
            let confirmation =
                host_video::Confirmation::resolution(&frame, stream, Instant::now())?;
            attempt.send(&frame)?;
            request.sent = true;
            self.confirmation = Some(confirmation);
            Ok(())
        })();
        crate::event_log::record(
            if result.is_ok() { "INFO" } else { "WARN" },
            "additional_display_resolution_requested",
            serde_json::json!({"stream":stream,"sent":result.is_ok()}),
        );
        if result.is_err() {
            self.finish_secondary(view, Status::Failed, false);
            self.failed(view);
        }
    }
    fn finish_secondary(&mut self, view: &impl View, status: Status, confirmed: bool) {
        if let Some(request) = self.secondary.take() {
            self.confirmation = None;
            self.query_deadline = None;
            view.notify(Notification::DisplayResolution {
                stream: request.stream,
                status,
                resolution: confirmed.then_some(request.resolution),
            });
        }
    }

    pub fn secondary_stream(&self) -> Option<usize> {
        self.secondary.as_ref().map(|request| request.stream)
    }
    pub fn cancel_secondary(&mut self, view: &impl View) {
        self.finish_secondary(view, Status::Unconfirmed, false);
        view.finished(false);
    }

    fn take_output(&mut self) -> Option<String> {
        // Match the host's output IDs and complete settings from the same
        // refresh. Either reply can arrive first; neither cached half suffices.
        if self.query_deadline.is_some() || self.outputs_deadline.is_some() {
            return None;
        }
        self.pending_output.take()
    }

    fn send_output(&mut self, attempt: &Attempt, view: &impl View) -> bool {
        if let Some(id) = self.take_output() {
            let result = (|| -> Result<()> {
                view.session()
                    .displays
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .validate_selection(&id, attempt.hid_mouse_enabled())?;
                attempt.send(&self.config.select_output(&id)?)?;
                self.confirmation = Some(host_video::Confirmation::output(&id, Instant::now()));
                Ok(())
            })();
            crate::event_log::record(
                if result.is_ok() { "INFO" } else { "WARN" },
                "native_host_display_requested",
                serde_json::json!({"sent":result.is_ok(),"resolution":"host"}),
            );
            if result.is_err() {
                self.failed(view);
                return false;
            }
        }
        true
    }
    fn send_pending(&mut self, attempt: &Attempt, view: &impl View) {
        let Some(next) = self.take_pending() else {
            return;
        };
        let size = view.display_size();
        match self.send(attempt, &next, size) {
            Ok(()) => {
                self.requested_display = Some(size);
                view.waiting();
            }
            Err(_) => {
                crate::event_log::record(
                    "WARN",
                    "native_host_video_request_failed",
                    serde_json::json!({}),
                );
                self.failed(view);
            }
        }
    }

    fn take_pending(&mut self) -> Option<Video> {
        // Output-list and configuration replies may arrive in either order.
        // Never consume the initial request without its complete patch base.
        if !self.config.ready() {
            return None;
        }
        self.pending.take()
    }
    fn observed(&self, view: &impl View) -> serde_json::Value {
        let mut report = self.config.observed();
        let displays = view
            .session()
            .displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        report["display_index"] = serde_json::json!(displays
            .outputs
            .iter()
            .position(|o| Some(o.id.as_str()) == self.config.output_id())
            .map(|i| i + 1));
        report["display_count"] = serde_json::json!(displays.outputs.len());
        report
    }

    pub fn poll(&mut self, attempt: &Attempt, view: &impl View) -> Result<()> {
        if self
            .outputs_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.outputs_deadline = None;
            let mut displays = view
                .session()
                .displays
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            displays.loading = false;
            displays.outputs.clear();
            drop(displays);
            self.hid_primary_ready(attempt, view, false);
            if self.pending_output.take().is_some() || attempt.hid_mouse_enabled() {
                self.pending = None;
                self.failed(view);
            }
            view.notify(Notification::DisplaysChanged);
            crate::event_log::record(
                "WARN",
                "native_host_display_query_timeout",
                serde_json::json!({}),
            );
        }

        if let Some(verify) = self.confirmation.as_mut() {
            match verify.poll(&self.config, Instant::now()) {
                host_video::Verification::Query => {
                    if attempt.send(&host_video::query()?).is_err() {
                        self.confirmation = None;
                        self.pending = None;
                        self.finish_secondary(view, Status::Unconfirmed, false);
                        view.finished(false);
                    }
                }
                status @ (host_video::Verification::Confirmed
                | host_video::Verification::TimedOut) => {
                    let confirmed = status == host_video::Verification::Confirmed;
                    self.confirmation = None;
                    if let Some(request) = &self.secondary {
                        crate::event_log::record(
                            if confirmed { "INFO" } else { "WARN" },
                            "additional_display_resolution_result",
                            serde_json::json!({"stream":request.stream,"confirmed":confirmed,"resolution":request.resolution}),
                        );
                        self.finish_secondary(
                            view,
                            if confirmed {
                                Status::Confirmed
                            } else {
                                Status::Unconfirmed
                            },
                            confirmed,
                        );
                        view.finished(confirmed);
                        return Ok(());
                    }
                    crate::event_log::record(
                        if confirmed { "INFO" } else { "WARN" },
                        if confirmed {
                            "native_host_video_confirmed"
                        } else {
                            "native_host_video_not_confirmed"
                        },
                        self.observed(view),
                    );
                    view.finished(confirmed);
                    if confirmed && self.ensure_hid_primary(attempt, view) {
                        self.send_pending(attempt, view);
                    } else if !confirmed {
                        self.pending = None;
                    }
                }
                host_video::Verification::Waiting => {}
            }
        }
        if self
            .query_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.query_deadline = None;
            self.pending = None;
            self.pending_output = None;
            self.finish_secondary(view, Status::Unconfirmed, false);
            crate::event_log::record(
                "WARN",
                "native_host_video_query_timeout",
                serde_json::json!({}),
            );
            self.failed(view);
        }
        Ok(())
    }

    fn hid_primary_ready(&self, attempt: &Attempt, view: &impl View, ready: bool) {
        if !attempt.hid_mouse_enabled() {
            return;
        }
        let previous = view
            .session()
            .hid_primary_ready
            .swap(ready, Ordering::AcqRel);
        if previous != ready {
            if previous {
                let _ = attempt.send(&crate::control::header(24, 0, 0, 0));
            }
            crate::event_log::record(
                if ready { "INFO" } else { "WARN" },
                "hid_primary_display_state",
                serde_json::json!({"ready":ready}),
            );
            view.notify(Notification::DisplaysChanged);
        }
    }
    fn ensure_hid_primary(&mut self, attempt: &Attempt, view: &impl View) -> bool {
        if !attempt.hid_mouse_enabled() {
            return true;
        }
        let primary = {
            let displays = view
                .session()
                .displays
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            // Verified stock Windows host preserves EnumAdapters1/EnumOutputs
            // order. Both DXGI APIs specify primary first; other OSes are unknown.
            match displays.primary(self.config.windows_host(), self.config.output_id()) {
                host_displays::Primary::Ready => {
                    drop(displays);
                    self.hid_primary_ready(attempt, view, true);
                    return true;
                }
                host_displays::Primary::Waiting => None,
                host_displays::Primary::Select(id) => Some(id.to_owned()),
            }
        };
        self.hid_primary_ready(attempt, view, false);
        let Some(id) = primary else {
            let displays = view
                .session()
                .displays
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let unavailable = self.config.ready() && !displays.loading;
            drop(displays);
            if unavailable {
                self.pending = None;
                self.failed(view);
            }
            return false;
        };
        if self.confirmation.is_some() {
            return false;
        }
        let result = self
            .config
            .select_output(&id)
            .and_then(|frame| attempt.send(&frame));
        crate::event_log::record(
            if result.is_ok() { "INFO" } else { "WARN" },
            "hid_primary_display_requested",
            serde_json::json!({"sent":result.is_ok()}),
        );
        if result.is_ok() {
            self.confirmation = Some(host_video::Confirmation::output(&id, Instant::now()));
            view.waiting();
        } else {
            self.pending = None;
            self.failed(view);
        }
        false
    }

    fn query_outputs(&mut self, attempt: &Attempt, view: &impl View) -> bool {
        self.hid_primary_ready(attempt, view, false);
        let sent = attempt
            .send(&host_displays::query().expect("Empty display query fits control frame"))
            .is_ok();
        self.outputs_deadline = sent.then(|| Instant::now() + QUERY_TIMEOUT);
        let mut displays = view
            .session()
            .displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        displays.loading = sent;
        displays.hid_mouse = attempt.hid_mouse_enabled();
        if !sent {
            displays.outputs.clear();
        }
        drop(displays);
        view.notify(Notification::DisplaysChanged);
        if !sent {
            crate::event_log::record(
                "WARN",
                "native_host_display_query_failed",
                serde_json::json!({}),
            );
        }
        sent
    }
    pub fn receive_outputs(&mut self, bytes: &[u8], attempt: &Attempt, view: &impl View) {
        self.outputs_deadline = None;
        let result = host_displays::parse(bytes);
        let valid = result.is_ok();
        let mut displays = view
            .session()
            .displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        displays.outputs = result.unwrap_or_default();
        displays.loading = false;
        let count = displays.outputs.len();
        drop(displays);
        crate::event_log::record(
            if valid { "INFO" } else { "WARN" },
            "native_host_displays_received",
            serde_json::json!({"valid":valid,"count":count}),
        );
        if !self.send_output(attempt, view) {
            view.notify(Notification::DisplaysChanged);
            return;
        }
        if self.ensure_hid_primary(attempt, view) && self.confirmation.is_none() {
            self.send_pending(attempt, view);
        }
        view.notify(Notification::DisplaysChanged);
    }
    pub fn refresh_outputs(&mut self, attempt: &Attempt, view: &impl View) -> Result<()> {
        ensure!(!self.busy(), "Waiting for the previous host request");
        self.query_outputs(attempt, view);
        attempt.send(&host_video::query()?)?;
        Ok(())
    }
    pub fn select_output(&mut self, attempt: &Attempt, view: &impl View, id: &str) -> Result<bool> {
        ensure!(
            !attempt.hid_mouse_enabled(),
            "Display switching is unavailable with HID mouse input"
        );
        ensure!(!self.busy(), "Waiting for the previous host request");
        let displays = view
            .session()
            .displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        displays.validate_selection(id, attempt.hid_mouse_enabled())?;
        if displays.current.as_deref() == Some(id) {
            return Ok(false);
        }
        drop(displays);
        // Refresh both halves, as the stock-host/lowlat transaction does.
        // Virtual-display IDs may change after the picker was populated.
        ensure!(
            self.query_outputs(attempt, view),
            "Could not refresh host displays"
        );
        attempt.send(&host_video::query()?)?;
        self.pending_output = Some(id.to_owned());
        self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
        view.waiting();
        Ok(true)
    }

    pub fn apply(
        &mut self,
        attempt: &Attempt,
        view: &impl View,
        video: &Video,
    ) -> Result<(u32, u32)> {
        ensure!(!self.busy(), "Waiting for the previous host request");
        let size = view.display_size();
        if video.has_host_settings() {
            if self.config.ready() {
                self.send(attempt, video, size)?;
            } else {
                self.pending = Some(video.clone());
                self.query_deadline = Some(Instant::now() + QUERY_TIMEOUT);
                attempt.send(&host_video::query()?)?;
            }
        }
        if self.busy() {
            view.waiting();
        }
        Ok(size)
    }

    pub fn applied_locally(&mut self, size: (u32, u32)) {
        self.requested_display = Some(size);
    }

    pub fn result(&self, view: &impl View, ok: bool) {
        if !self.busy() {
            view.result(ok);
        }
    }

    pub fn resized(&mut self, attempt: &Attempt, view: &impl View, video: &Video) -> Result<()> {
        let size = view.display_size();
        if self.requested_display == Some(size) || !self.config.ready() || self.busy() {
            return Ok(());
        }
        self.send(attempt, video, size)?;
        view.waiting();
        self.requested_display = Some(size);
        Ok(())
    }

    fn failed(&self, view: &impl View) {
        view.session().video_pending.store(false, Ordering::Release);
        view.result(false);
    }

    fn send(&mut self, attempt: &Attempt, video: &Video, size: (u32, u32)) -> Result<()> {
        let frame = self.config.request(video, size)?;
        let confirmation = host_video::Confirmation::new(&frame, Instant::now())?;
        attempt.send(&frame)?;
        crate::event_log::record(
            "INFO",
            "native_host_video_requested",
            serde_json::json!({"fps":video.fps,"constant_fps":video.constant_fps,"bandwidth_mbps":video.bandwidth_mbps,"rotated":video.rotated,"resolution":video.resolution}),
        );
        self.confirmation = Some(confirmation);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct TestView {
        state: super::super::session_model::State,
        notifications: std::cell::RefCell<Vec<(Notification, bool)>>,
    }
    impl View for TestView {
        fn session(&self) -> &super::super::session_model::State {
            &self.state
        }
        fn display_size(&self) -> (u32, u32) {
            (1920, 1080)
        }
        fn notify(&self, notification: Notification) {
            self.notifications.borrow_mut().push((
                notification,
                self.state.video_pending.load(Ordering::Acquire),
            ));
        }
    }

    #[test]
    fn secondary_resolution_owns_the_transaction_and_only_confirms_its_target() {
        let view = TestView::default();
        let mut settings = HostSettings::default();
        settings.secondary = Some(DisplayResolution {
            stream: 2,
            resolution: crate::session_preferences::Resolution::Fixed(1280, 720),
            size: (1920, 1080),
            sent: false,
        });
        view.waiting();
        assert!(settings.busy());
        assert_eq!(settings.secondary_stream(), Some(2));
        settings.finish_secondary(&view, Status::Confirmed, true);
        view.finished(true);
        assert!(!settings.busy());
        assert!(view
            .notifications
            .borrow()
            .iter()
            .any(|(event, _)| matches!(
                event,
                Notification::DisplayResolution {
                    stream: 2,
                    status: Status::Confirmed,
                    resolution: Some(crate::session_preferences::Resolution::Fixed(1280, 720))
                }
            )));
        settings.secondary = Some(DisplayResolution {
            stream: 1,
            resolution: crate::session_preferences::Resolution::Host,
            size: (1920, 1080),
            sent: true,
        });
        settings.query_deadline = Some(Instant::now());
        settings.cancel_secondary(&view);
        assert!(!settings.busy());
        assert!(settings.query_deadline.is_none());
        assert!(settings.confirmation.is_none());
        assert!(!view.state.video_pending.load(Ordering::Acquire));
        assert!(view
            .notifications
            .borrow()
            .iter()
            .any(|(event, _)| matches!(
                event,
                Notification::DisplayResolution {
                    stream: 1,
                    status: Status::Unconfirmed,
                    resolution: None
                }
            )));
    }
    #[test]
    fn settings_controller_notifies_without_a_window_and_publishes_pending_first() {
        let view = TestView::default();
        view.state.hid_primary_ready.store(true, Ordering::Release);
        let settings = HostSettings::new(
            &Video {
                fps: Some(60),
                ..Default::default()
            },
            &view,
        );
        assert!(!view.state.hid_primary_ready.load(Ordering::Acquire));
        // A local result must not enable Apply while a host request is pending.
        settings.result(&view, true);
        assert_eq!(view.notifications.borrow().len(), 1);
        settings.failed(&view);
        view.waiting();
        view.finished(false);
        view.finished(true);
        assert_eq!(
            *view.notifications.borrow(),
            vec![
                (Notification::Status(Status::Waiting), true),
                (Notification::Status(Status::Failed), false),
                (Notification::Status(Status::Waiting), true),
                (Notification::Status(Status::Unconfirmed), false),
                (Notification::Status(Status::Confirmed), false),
            ]
        );
    }

    #[test]
    fn display_selection_waits_for_both_fresh_replies_in_either_order() {
        for config_first in [false, true] {
            let deadline = Instant::now() + QUERY_TIMEOUT;
            let mut settings = HostSettings {
                pending_output: Some("secondary".into()),
                query_deadline: Some(deadline),
                outputs_deadline: Some(deadline),
                ..Default::default()
            };
            // A usable cached configuration must not satisfy the new request.
            settings
                .config
                .receive(br#"{"video":[{"output":"old","hostOS":1}]}"#)
                .unwrap();
            assert!(settings.take_output().is_none());
            if config_first {
                settings.query_deadline = None;
            } else {
                settings.outputs_deadline = None;
            }
            assert!(settings.take_output().is_none());
            assert!(settings.pending_output.is_some());
            settings.query_deadline = None;
            settings.outputs_deadline = None;
            assert_eq!(settings.take_output().as_deref(), Some("secondary"));
            assert!(settings.take_output().is_none());
        }
    }

    #[test]
    fn output_list_reply_cannot_consume_video_request_before_host_config() {
        let mut settings = HostSettings {
            pending: Some(Video {
                fps: Some(60),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(settings.take_pending().is_none());
        assert!(settings.pending.is_some());
        settings
            .config
            .receive(br#"{"video":[{"output":"fixture","hostOS":1}]}"#)
            .unwrap();
        assert_eq!(settings.take_pending().unwrap().fps, Some(60));
        assert!(settings.take_pending().is_none());
    }
}
