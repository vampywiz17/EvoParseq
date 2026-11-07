//! Text-only clipboard policy shared by desktop ports. Platform adapters own
//! clipboard APIs, focus/visibility permission and notification delivery.
use std::time::{Duration, Instant};
pub const MAX_TEXT: usize = 1024 * 1024;
pub(crate) const REMOTE_TEXT: usize = 102399;
pub fn remote_text(mut text: String) -> String {
    if text.len() > REMOTE_TEXT {
        let mut end = REMOTE_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}
pub(crate) struct Pending {
    pub text: String,
    pub expires: Instant,
    pub next_try: Instant,
    pub attempts: u8,
}
pub(crate) fn receive(
    last: &mut String,
    pending: &mut Option<Pending>,
    text: &str,
    allowed: bool,
    now: Instant,
    write: impl FnMut(&str) -> bool,
) {
    *pending = None;
    if !allowed || text == last || text.len() > MAX_TEXT || text.contains('\0') {
        return;
    }
    *pending = Some(Pending {
        text: text.into(),
        expires: now + Duration::from_secs(1),
        next_try: now,
        attempts: 0,
    });
    retry(last, pending, allowed, now, write);
}
pub(crate) fn retry(
    last: &mut String,
    pending: &mut Option<Pending>,
    allowed: bool,
    now: Instant,
    mut write: impl FnMut(&str) -> bool,
) {
    let Some(value) = pending.as_mut() else {
        return;
    };
    if !allowed || now >= value.expires {
        *pending = None;
        return;
    }
    if now < value.next_try {
        return;
    }
    value.attempts += 1;
    if write(&value.text) {
        *last = pending.take().unwrap().text;
    } else if value.attempts >= 5 {
        *pending = None;
        crate::event_log::record(
            "WARN",
            "clipboard_write_unavailable",
            serde_json::json!({"attempts":5}),
        );
    } else {
        value.next_try = now + Duration::from_millis(200);
    }
}
pub(crate) fn send_local(
    last: &mut String,
    text: String,
    mut send: impl FnMut(&[u8]) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if text.len() > MAX_TEXT || text.contains('\0') {
        return Ok(());
    }
    let text = remote_text(text);
    if text != *last {
        send(&crate::control::text(17, 7, &text)?)?;
        *last = text;
    }
    Ok(())
}
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub struct State {
    last: String,
    pending: Option<Pending>,
}
#[cfg(any(target_os = "linux", test))]
impl State {
    pub fn receive(&mut self, text: &str, allowed: bool, write: impl FnMut(&str) -> bool) {
        receive(
            &mut self.last,
            &mut self.pending,
            text,
            allowed,
            Instant::now(),
            write,
        );
    }
    pub fn local(
        &mut self,
        text: String,
        allowed: bool,
        send: impl FnMut(&[u8]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.pending = None;
        if allowed {
            send_local(&mut self.last, text, send)
        } else {
            Ok(())
        }
    }
    #[cfg(target_os = "linux")]
    pub fn revoke(&mut self) {
        self.pending = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_never_echoes_or_overwrites_after_focus_loss_and_keeps_utf8_boundary() {
        let mut s = State::default();
        let mut writes = 0;
        s.receive("host", true, |_| {
            writes += 1;
            true
        });
        s.receive("host", true, |_| {
            writes += 1;
            true
        });
        assert_eq!(writes, 1);
        s.local("host".into(), true, |_| panic!("echo")).unwrap();
        s.receive("private", false, |_| panic!("unfocused write"));
        s.local("private".into(), false, |_| panic!("unfocused read"))
            .unwrap();
        assert_eq!(remote_text("á".repeat(60000)).len(), 102398);
        s.local("client".into(), true, |_| anyhow::bail!("offline"))
            .unwrap_err();
        assert_eq!(s.last, "host");
    }
}
