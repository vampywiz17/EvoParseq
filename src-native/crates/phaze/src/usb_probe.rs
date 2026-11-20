//! USB channel framing and virtual HID devices for authenticated Phaze sessions.
use crate::{
    usb_hid::{Device, KEYBOARD_ID, MOUSE_ID, VID},
    Input,
};
use anyhow::{ensure, Result};
use std::collections::VecDeque;

const PREFIX: usize = 8;
const HEADER: usize = 33;
const MAX_BODY: usize = 128 * 1024;
pub(crate) const LINK_REPLY: &[u8] = &[16, 1];
const REPLY_HEADER: usize = 1 + PREFIX + HEADER;
const HELLO_BYTES: usize = REPLY_HEADER + 130;
const USB_VERSION: [u8; 2] = 9u16.to_le_bytes();
const PROBE_NAME: &[u8] = b"EvoParseq";
const MAX_OUTGOING: usize = 192;
const CHUNK: usize = 1100;
const LINK_DOWN: [u8; 2] = [16, 0];
const OPEN_PAYLOAD: usize = 0x10090;
const STALL: [u8; 4] = (-32i32).to_le_bytes();

#[derive(Default)]
pub(crate) struct Probe {
    controls_host: bool,
    link_seen: bool,
    acknowledged: bool,
    failed: bool,
    pending: Vec<u8>,
    hello: Option<[u8; HELLO_BYTES]>,
    pong: Option<[u8; REPLY_HEADER]>,
    devices: Vec<Device>,
    outgoing: VecDeque<Vec<u8>>,
}

impl Probe {
    pub(crate) fn new(mouse: bool, keyboard: bool) -> Self {
        let devices = [(mouse, MOUSE_ID), (keyboard, KEYBOARD_ID)]
            .into_iter()
            .filter_map(|(on, id)| on.then_some(id))
            .map(Device::new)
            .collect();
        Self {
            devices,
            ..Self::default()
        }
    }

    pub(crate) fn input(&mut self, input: Input) -> bool {
        if input == Input::Release {
            for device in &mut self.devices {
                device.release();
            }
            return false;
        }
        let id = if matches!(input, Input::Key { .. }) {
            KEYBOARD_ID
        } else {
            MOUSE_ID
        };
        if let Some(device) = self.devices.iter_mut().find(|dev| dev.id == id) {
            if self.controls_host && !self.failed {
                device.input(input);
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn control(&mut self, granted: bool) {
        if self.controls_host && !granted {
            self.link_seen = false;
            self.acknowledged = false;
            self.pending.clear();
            self.hello = None;
            self.pong = None;
            self.clear_devices();
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
        self.hello = None;
        self.pong = None;
        self.clear_devices();
    }

    fn clear_devices(&mut self) {
        self.outgoing.clear();
        for device in &mut self.devices {
            device.close();
            device.announced = false;
        }
    }

    pub(crate) fn disconnect(&mut self) -> Vec<Vec<u8>> {
        if !self.acknowledged {
            return Vec::new();
        }
        let mut frames = Vec::new();
        // Complete pending interrupt reads with all buttons/keys released. A
        // partially queued USB record is instead cancelled by link teardown.
        if self.outgoing.is_empty() {
            for device in &mut self.devices {
                device.release();
                if let Some((request, payload)) = device.complete() {
                    frames.push(record(19, &request, &payload));
                }
            }
        }
        self.disable();
        frames.push(LINK_DOWN.to_vec());
        frames
    }

    pub(crate) fn next_device_frame(&mut self) -> Option<Vec<u8>> {
        if let Some(frame) = self.outgoing.pop_front() {
            return Some(frame);
        }
        let (request, payload) = self.devices.iter_mut().find_map(Device::complete)?;
        Some(record(19, &request, &payload))
    }

    pub(crate) fn take_hello(&mut self) -> Option<[u8; HELLO_BYTES]> {
        if self.outgoing.is_empty() {
            self.hello.take()
        } else {
            None
        }
    }

    pub(crate) fn take_pong(&mut self) -> Option<[u8; REPLY_HEADER]> {
        if self.outgoing.is_empty() {
            self.pong.take()
        } else {
            None
        }
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
            if self.controls_host && self.acknowledged {
                match (body[0], &body[HEADER..]) {
                    (0, hello) if hello.len() == 66 && hello.starts_with(&USB_VERSION) => {
                        let mut reply = response::<HELLO_BYTES>(1, body);
                        reply[REPLY_HEADER..REPLY_HEADER + 2].copy_from_slice(&USB_VERSION);
                        reply[REPLY_HEADER + 2..REPLY_HEADER + 2 + PROBE_NAME.len()]
                            .copy_from_slice(PROBE_NAME);
                        self.hello = Some(reply);
                    }
                    (10, []) => self.pong = Some(response(11, body)),
                    _ if !self.devices.is_empty() => {
                        let request: [u8; HEADER] = body[..HEADER].try_into()?;
                        device_request(
                            &mut self.devices,
                            &mut self.outgoing,
                            &request,
                            &body[HEADER..],
                        )?;
                    }
                    _ => {}
                }
            }
            consumed = body_start + body_len;
        }
        self.pending.drain(..consumed);
        Ok(())
    }
}

fn device_request(
    devices: &mut [Device],
    queue: &mut VecDeque<Vec<u8>>,
    req: &[u8; HEADER],
    payload: &[u8],
) -> Result<()> {
    if req[0] == 9 && payload.is_empty() {
        for device in devices {
            enqueue(queue, announcement(req, device))?;
            if !device.announced {
                // A selected virtual device is offered once per USB link.
                let mut bind = [0; 5];
                bind[..4].copy_from_slice(&device.id.to_le_bytes());
                bind[4] = 1;
                let mut header = [0; HEADER];
                header[25..29].copy_from_slice(&device.id.to_le_bytes());
                enqueue(queue, record(250, &header, &bind))?;
                device.announced = true;
            }
        }
        return Ok(());
    }
    let id = le(&req[25..29]);
    let Some(device) = devices.iter_mut().find(|device| device.id == id) else {
        return Ok(());
    };
    match req[0] {
        25 => {
            device.open();
            let mut details = vec![0; OPEN_PAYLOAD];
            details[..5].copy_from_slice(&[1, 0, 0, 0, 1]);
            let descriptor = device.device_descriptor();
            let config = device.configuration();
            details[5..5 + descriptor.len()].copy_from_slice(&descriptor);
            let offset = 5 + descriptor.len();
            details[offset..offset + config.len()].copy_from_slice(&config);
            // Version 9's fixed descriptor block is followed by the device name.
            let name_offset = 0x10037 - HEADER;
            let name = device.name().as_bytes();
            details[name_offset..name_offset + name.len()].copy_from_slice(name);
            enqueue(queue, record(39, req, &details))?;
            enqueue(queue, announcement(req, device))?;
        }
        13 if payload.len() >= 16 => {
            let setup = payload[8..16].try_into()?;
            let data = if device.opened {
                device.control(setup, &payload[16..])
            } else {
                let mut data = payload[8..16].to_vec();
                data.extend(STALL);
                data.extend([0; 4]);
                data
            };
            enqueue(queue, record(14, req, &data))?;
        }
        18 => {
            // The little-endian endpoint address is the first word.
            let endpoint = payload.get(..4).map(le);
            if endpoint != Some(0x81) || !device.enqueue_read(*req) {
                enqueue(queue, record(19, req, &STALL))?;
            }
        }
        35 => {
            device.close();
            enqueue(queue, record(7, req, &[0; 4]))?;
        }
        _ => {}
    }
    Ok(())
}

fn announcement(req: &[u8; HEADER], device: &Device) -> Vec<u8> {
    let mut payload = [0; 75];
    payload[..4].copy_from_slice(&device.id.to_le_bytes());
    payload[4] = if device.opened { 3 } else { 1 };
    payload[5..7].copy_from_slice(&VID.to_le_bytes());
    payload[7..9].copy_from_slice(&device.product().to_le_bytes());
    payload[9] = 1;
    let name = device.name().as_bytes();
    payload[11..11 + name.len()].copy_from_slice(name);
    record(16, req, &payload)
}

fn record(kind: u8, request: &[u8; HEADER], data: &[u8]) -> Vec<u8> {
    let mut frame = response::<REPLY_HEADER>(kind, request).to_vec();
    frame[5..9].copy_from_slice(&((HEADER + data.len()) as u32).to_le_bytes());
    frame[38..42].copy_from_slice(&(data.len() as u32).to_le_bytes());
    frame.extend_from_slice(data);
    frame
}

fn enqueue(queue: &mut VecDeque<Vec<u8>>, frame: Vec<u8>) -> Result<()> {
    ensure!(
        queue.len() + (frame.len() - 1).div_ceil(CHUNK) <= MAX_OUTGOING,
        "Phaze USB response queue full"
    );
    for chunk in frame[1..].chunks(CHUNK) {
        let mut out = Vec::with_capacity(1 + chunk.len());
        out.push(15);
        out.extend_from_slice(chunk);
        queue.push_back(out);
    }
    Ok(())
}

fn response<const N: usize>(kind: u8, request: &[u8]) -> [u8; N] {
    let mut reply = [0; N];
    reply[0] = 15;
    reply[5..9].copy_from_slice(&((N - 1 - PREFIX) as u32).to_le_bytes());
    reply[9] = kind;
    reply[14..38].copy_from_slice(&request[5..29]);
    reply[38..42].copy_from_slice(&((N - REPLY_HEADER) as u32).to_le_bytes());
    reply
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
    fn usb_hello_uses_the_probe_name_only_for_the_observed_version() -> Result<()> {
        let mut hello = [0; PREFIX + HEADER + 66];
        hello[4..8].copy_from_slice(&99u32.to_le_bytes());
        hello[13..37].fill(0x21);
        hello[37..41].copy_from_slice(&66u32.to_le_bytes());
        let mut probe = Probe::default();
        probe.control(true);
        probe.replied();
        probe.receive(&hello, |_, _| {})?;
        assert!(probe.take_hello().is_none());
        hello[41..43].copy_from_slice(&USB_VERSION);
        hello[43..].fill(b'X');
        probe.receive(&hello, |_, _| {})?;
        let reply = probe.take_hello().unwrap();
        assert_eq!((reply[0], reply[9]), (15, 1));
        assert_eq!(le(&reply[5..9]), 163);
        assert_eq!(le(&reply[38..42]), 130);
        assert_eq!(&reply[14..38], &hello[13..37]);
        assert_eq!(&reply[42..44], &USB_VERSION);
        assert_eq!(&reply[44..44 + PROBE_NAME.len()], PROBE_NAME);
        assert!(reply[44 + PROBE_NAME.len()..].iter().all(|&byte| byte == 0));
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
    fn devices_are_announced_opened_polled_and_disconnected() -> Result<()> {
        let mut usb = Probe::new(true, true);
        usb.control(true);
        usb.replied();
        let mut req = [0; HEADER];
        req[0] = 9;
        device_request(&mut usb.devices, &mut usb.outgoing, &req, &[])?;
        let offers: Vec<_> = usb.outgoing.drain(..).collect();
        assert_eq!(
            offers.iter().map(|f| f[9]).collect::<Vec<_>>(),
            [16, 250, 16, 250]
        );
        for (device, neutral) in [(MOUSE_ID, 8usize), (KEYBOARD_ID, 29)] {
            req[25..29].copy_from_slice(&device.to_le_bytes());
            req[0] = 25;
            device_request(&mut usb.devices, &mut usb.outgoing, &req, &[])?;
            let first = usb.outgoing.front().unwrap();
            assert_eq!(first[9], 39);
            assert_eq!(le(&first[38..42]) as usize, OPEN_PAYLOAD);
            usb.outgoing.clear();
            req[0] = 18;
            device_request(
                &mut usb.devices,
                &mut usb.outgoing,
                &req,
                &0x81u32.to_le_bytes(),
            )?;
            let reply = usb.next_device_frame().unwrap();
            assert_eq!(reply[9], 19);
            assert_eq!(&reply[42..46], &[0; 4]);
            assert_eq!(reply.len() - 46, neutral);
            device_request(
                &mut usb.devices,
                &mut usb.outgoing,
                &req,
                &0x81u32.to_le_bytes(),
            )?;
        }
        let frames = usb.disconnect();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames.last().unwrap(), &LINK_DOWN);
        assert!(usb.next_device_frame().is_none());
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
