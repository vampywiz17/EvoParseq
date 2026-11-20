//! Opt-in observation of the authenticated Phaze USB channel.
//! No devices are announced and no input reports are sent.
use anyhow::{ensure, Result};

const PREFIX: usize = 8;
const HEADER: usize = 33;
const MAX_BODY: usize = 128 * 1024;
pub(crate) const LINK_REPLY: &[u8] = &[16, 1];
pub(crate) const PONG_BYTES: usize = 1 + PREFIX + HEADER;

#[derive(Default)]
pub(crate) struct Probe {
    controls_host: bool,
    link_seen: bool,
    acknowledged: bool,
    failed: bool,
    pending: Vec<u8>,
    pong: Option<[u8; PONG_BYTES]>,
}

impl Probe {
    pub(crate) fn control(&mut self, granted: bool) {
        if self.controls_host && !granted {
            self.link_seen = false;
            self.acknowledged = false;
            self.pending.clear();
            self.pong = None;
        }
        self.controls_host = granted;
    }

    pub(crate) fn link_state(&mut self, state: u8) {
        if state == 0 {
            self.link_seen = true;
        }
    }

    pub(crate) fn wants_reply(&self) -> bool {
        !self.failed && self.controls_host && self.link_seen && !self.acknowledged
    }

    pub(crate) fn replied(&mut self) {
        self.acknowledged = true;
    }

    pub(crate) fn disable(&mut self) {
        self.failed = true;
        self.pending.clear();
        self.pong = None;
    }

    pub(crate) fn take_pong(&mut self) -> Option<[u8; PONG_BYTES]> {
        self.pong.take()
    }

    pub(crate) fn receive(
        &mut self,
        chunk: &[u8],
        mut observe: impl FnMut(u8, usize),
    ) -> Result<()> {
        if self.failed {
            return Ok(());
        }
        ensure!(
            chunk.len() <= MAX_BODY + PREFIX - self.pending.len(),
            "Phaze USB probe buffer limit"
        );
        self.pending.extend_from_slice(chunk);
        let mut consumed = 0;
        while let Some(prefix) = self.pending[consumed..].get(..PREFIX) {
            let stream = le(&prefix[..4]);
            let body_len = le(&prefix[4..]) as usize;
            ensure!(stream == 0, "Phaze USB probe unknown stream");
            ensure!(
                (HEADER..=MAX_BODY).contains(&body_len),
                "Phaze USB probe invalid body length"
            );
            let body_start = consumed + PREFIX;
            let Some(body) = self.pending.get(body_start..body_start + body_len) else {
                break;
            };
            let payload_len = le(&body[29..33]) as usize;
            ensure!(
                payload_len == body_len - HEADER,
                "Phaze USB probe payload length mismatch"
            );
            observe(body[0], payload_len);
            if self.controls_host && self.acknowledged && body[0] == 10 && payload_len == 0 {
                let mut pong = [0; PONG_BYTES];
                pong[0] = 15;
                pong[5..9].copy_from_slice(&(HEADER as u32).to_le_bytes());
                pong[9..].copy_from_slice(body);
                pong[9] = 11;
                self.pong = Some(pong);
            }
            consumed = body_start + body_len;
        }
        self.pending.drain(..consumed);
        Ok(())
    }
}

fn le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("checked USB field length"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receives_split_and_coalesced_records_without_reading_payload() -> Result<()> {
        let mut record = vec![0; PREFIX + HEADER + 4];
        record[4..8].copy_from_slice(&37u32.to_le_bytes());
        record[PREFIX] = 12;
        record[PREFIX + 29..PREFIX + 33].copy_from_slice(&4u32.to_le_bytes());
        let mut probe = Probe::default();
        probe.receive(&record[..8], |_, _| panic!("incomplete message"))?;
        probe.receive(&record[8..12], |_, _| panic!("incomplete message"))?;
        let mut seen = Vec::new();
        probe.receive(&[&record[12..], &record].concat(), |kind, len| {
            seen.push((kind, len))
        })?;
        assert_eq!(seen, [(12, 4), (12, 4)]);
        assert!(probe.pending.is_empty());
        assert!(probe
            .receive(&[0, 0, 0, 0, 255, 255, 255, 255], |_, _| {})
            .is_err());
        Ok(())
    }

    #[test]
    fn usb_ping_echoes_request_fields_without_submitting_input() -> Result<()> {
        let mut ping = [0; PREFIX + HEADER];
        ping[4..8].copy_from_slice(&(HEADER as u32).to_le_bytes());
        ping[8] = 10;
        ping[13..37].fill(0x42);
        let mut probe = Probe::default();
        probe.receive(&ping, |_, _| {})?;
        assert!(probe.take_pong().is_none());
        probe.control(true);
        probe.link_state(0);
        probe.replied();
        probe.receive(&ping, |_, _| {})?;
        let pong = probe.take_pong().unwrap();
        ping[8] = 11;
        assert_eq!(pong[0], 15);
        assert_eq!(pong[1..], ping);
        ping[8] = 10;
        probe.receive(&ping, |_, _| {})?;
        probe.control(false);
        assert!(probe.take_pong().is_none());
        Ok(())
    }

    #[test]
    fn replies_once_only_while_the_host_grants_control() {
        let mut probe = Probe::default();
        probe.link_state(0);
        assert!(!probe.wants_reply());
        probe.control(true);
        assert!(probe.wants_reply());
        probe.replied();
        assert!(!probe.wants_reply());
        probe.control(false);
        probe.control(true);
        assert!(!probe.wants_reply());
        probe.link_state(0);
        assert!(probe.wants_reply());
        probe.disable();
        assert!(!probe.wants_reply());
    }
}
