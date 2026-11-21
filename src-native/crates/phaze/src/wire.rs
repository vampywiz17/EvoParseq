use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
type Received = (Vec<Vec<u8>>, Option<Vec<u8>>);
/// Protected peer heartbeat: separate from ordinary reliable/application frames.
pub const HEARTBEAT: [u8; 2] = [0xcd, 3];
pub enum Envelope<'a> {
    Close,
    Reliable(&'a [u8]),
    Heartbeat,
}
pub fn envelope(bytes: &[u8]) -> Option<Envelope<'_>> {
    match bytes {
        [0xcd, 1, ..] => Some(Envelope::Close),
        [0xcd, 2, payload @ ..] => Some(Envelope::Reliable(payload)),
        [0xcd, 3, ..] => Some(Envelope::Heartbeat),
        _ => None,
    }
}
const MAX_FRAME: usize = 16 * 1024 * 1024;
#[derive(Serialize, Debug)]
pub struct Output {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub active: bool,
    pub primary: bool,
}
pub(crate) fn be(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes(b[i..i + 4].try_into().unwrap())
}
pub fn outputs(b: &[u8]) -> Result<Vec<Output>> {
    ensure!(
        b.len() >= 12 && be(b, 0) as usize == b.len(),
        "Invalid output-list length"
    );
    let n = be(b, 4) as usize;
    ensure!(n <= 12, "Output list exceeds protocol limit");
    let mut p = 12;
    let mut out = Vec::new();
    for _ in 0..n {
        ensure!(p + 8 <= b.len(), "Truncated output entry");
        let id = be(b, p);
        let len = be(b, p + 4) as usize;
        p += 8;
        ensure!(
            len <= b.len() - p && b.len() - p - len >= 54,
            "Truncated output name/tail"
        );
        let name = String::from_utf8_lossy(&b[p..p + len])
            .chars()
            .take(128)
            .collect();
        p += len;
        out.push(Output {
            id,
            name,
            width: be(b, p + 8),
            height: be(b, p + 12),
            active: b[p + 52] != 0,
            primary: b[p + 53] != 0,
        });
        p += 54;
    }
    Ok(out)
}
/// Independent encoding of the six big-endian stream-request words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoConfig {
    pub hevc: bool,
    pub chroma_444: bool,
    pub mbps: u32,
    pub fps: u32,
}
impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            hevc: false,
            chroma_444: false,
            mbps: 10,
            fps: 30,
        }
    }
}
impl VideoConfig {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (1..=100).contains(&self.mbps) && (1..=240).contains(&self.fps),
            "Invalid Phaze stream limits"
        );
        ensure!(!self.chroma_444 || self.hevc, "Phaze 4:4:4 requires HEVC");
        Ok(())
    }
    pub fn request(self, output: u32) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = vec![3];
        let caps = u32::from(self.hevc) | (u32::from(self.chroma_444) << 1);
        for x in [output, caps, self.mbps, self.fps, 4096, 4096] {
            b.extend(x.to_be_bytes());
        }
        Ok(b)
    }
}

pub(crate) fn ack_bandwidth(ack: &[u8]) -> Option<(f64, f64)> {
    if ack.len() != 50 || ack[0] != 4 {
        return None;
    }
    Some((
        f64::from_le_bytes(ack[34..42].try_into().unwrap()),
        f64::from_le_bytes(ack[42..50].try_into().unwrap()),
    ))
}

pub fn hello(raw_audio: bool) -> Vec<u8> {
    let mut b = vec![18];
    b.extend(u32::from(raw_audio).to_be_bytes());
    b.extend(0u32.to_be_bytes()); // Legacy host-driven layout.
    b
}

/// Experimental, explicit client-side display layout. The 0x1c0 option
/// mask matches the vendor client's mirror-layout + client-display mode.
/// Only independently reconstructed wire fields are sent; no native code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualClientDisplay {
    pub width: u32,
    pub height: u32,
}
pub fn parse_virtual_client_display(value: &str) -> Result<VirtualClientDisplay> {
    let (width, height) = value.split_once('x').context("Expected WIDTHxHEIGHT")?;
    let width: u32 = width.parse().context("Invalid virtual display width")?;
    let height: u32 = height.parse().context("Invalid virtual display height")?;
    let display = VirtualClientDisplay { width, height };
    ensure!(
        (320..=8192).contains(&width)
            && (240..=4320).contains(&height)
            && u64::from(width) * u64::from(height) <= 33_177_600,
        "Virtual display dimensions are out of range"
    );
    Ok(display)
}

/// Independently reconstructed from the Phaze 0.14.3 client serializer:
/// 18 | BE options | BE count | (BE id, BE UTF-8 length, name, 11 BE
/// display words, two BE fixed-point scale values, active, primary).
/// The on-wire tail shares the 54-byte format accepted by outputs().
/// Host acceptance and VDD behavior require live validation.
pub fn hello_with_virtual_display(
    raw_audio: bool,
    display: Option<VirtualClientDisplay>,
) -> Result<Vec<u8>> {
    let mut b = hello(raw_audio);
    let Some(display) = display else {
        return Ok(b);
    };
    ensure!(
        (320..=8192).contains(&display.width)
            && (240..=4320).contains(&display.height)
            && u64::from(display.width) * u64::from(display.height) <= 33_177_600,
        "Virtual display dimensions are out of range"
    );
    const NAME: &[u8] = b"EvoParseq Virtual";
    const CLIENT_LAYOUT_FLAGS: u32 = 0x100 | 0xc0;
    let flags = u32::from(raw_audio) | CLIENT_LAYOUT_FLAGS;
    b[1..5].copy_from_slice(&flags.to_be_bytes());
    b[5..9].copy_from_slice(&1u32.to_be_bytes());
    b.extend(0x4556_5031u32.to_be_bytes()); // Stable, synthetic client display ID.
    b.extend((NAME.len() as u32).to_be_bytes());
    b.extend(NAME);
    // x, y, logical width/height, rotation, pixel width/height,
    // refresh numerator/denominator, reserved display capabilities.
    for word in [
        0,
        0,
        display.width,
        display.height,
        0,
        display.width,
        display.height,
        60,
        1,
        0,
        0,
    ] {
        b.extend(word.to_be_bytes());
    }
    // The vendor serializer scales both normalized floating values by 10,000.
    b.extend(10_000u32.to_be_bytes());
    b.extend(10_000u32.to_be_bytes());
    b.extend([1, 1]); // Active, primary.
    Ok(b)
}

pub(crate) fn video_request(
    previous: Option<u32>,
    output: u32,
    config: VideoConfig,
    recovering: bool,
) -> Result<(Option<[u8; 5]>, Vec<u8>)> {
    let start = config.request(output)?;
    let stop = previous
        .filter(|&previous| previous != output || recovering)
        .map(|previous| {
            let mut command = [4; 5];
            command[1..].copy_from_slice(&previous.to_be_bytes());
            command
        });
    Ok((stop, start))
}

#[cfg(test)]
#[test]
fn recovery_reopens_the_selected_output_without_changing_stream_settings() -> Result<()> {
    let config = VideoConfig {
        mbps: 50,
        fps: 60,
        ..Default::default()
    };
    let output = 0x12_34_56_78;
    let start = config.request(output)?;
    assert_eq!(
        video_request(None, output, config, false)?,
        (None, start.clone())
    );
    assert_eq!(
        video_request(Some(output), output, config, false)?,
        (None, start.clone())
    );
    assert_eq!(
        video_request(Some(output), output, config, true)?,
        (Some([4, 0x12, 0x34, 0x56, 0x78]), start.clone())
    );
    assert_eq!(
        video_request(Some(1), output, config, false)?,
        (Some([4, 0, 0, 0, 1]), start)
    );
    assert!(video_request(Some(output), output, VideoConfig { fps: 0, ..config }, true).is_err());
    Ok(())
}
pub fn encoded(b: &[u8]) -> Result<&[u8]> {
    ensure!(b.len() >= 24, "Short video header");
    let n = be(b, 20) as usize;
    ensure!(n <= b.len() - 24, "Compressed length exceeds message");
    Ok(&b[24..24 + n])
}
#[cfg(test)]
fn nal_types(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < b.len() {
        let n = if b[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if b[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };
        if i + n < b.len() {
            out.push(b[i + n] & 31)
        }
        i += n + 1;
    }
    out
}
#[derive(Default)]
pub struct Reliable {
    /// First packet of the next message to deliver.
    next: u64,
    /// First missing packet. This can advance before a fragmented message is complete.
    frontier: u64,
    highest: Option<u64>,
    serial: u64,
    packets: BTreeMap<u64, Vec<u8>>,
    ack_bitmap: u64,
    bandwidth: crate::bandwidth::Bandwidth,
    tx_next: u64,
    outgoing: BTreeMap<u64, Pending>,
    rtt: Option<(Instant, Duration)>,
    buffered: usize,
    input_packets: u64,
    input_retries: u64,
}
struct Pending {
    bytes: Vec<u8>,
    last_sent: Option<Instant>,
    retries: u8,
    input: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputDelivery {
    pub sent: u64,
    /// Acknowledged by the remote transport, independently of OS injection.
    pub acknowledged: u64,
    pub retransmitted: u64,
    pub oldest_unacknowledged_attempt: Option<Duration>,
}

const RETRY_INTERVAL: Duration = Duration::from_millis(250);
const MAX_RETRIES: u8 = 20;
impl Reliable {
    pub fn fits(&self, len: usize, limit: usize) -> bool {
        self.outgoing.len() + len.div_ceil(1170) <= limit
    }

    pub fn frame(&mut self, payload: &[u8]) -> Result<Vec<Vec<u8>>> {
        ensure!(payload.len() <= MAX_FRAME, "Outgoing frame too large");
        ensure!(
            self.fits(payload.len(), 64),
            "Outgoing reliable window exceeded"
        );
        let count = payload.len().div_ceil(1170);
        ensure!(
            count > 0 && count <= u16::MAX as usize,
            "Invalid fragment count"
        );
        let input = matches!(payload.first(), Some(8..=11 | 13));
        let mut out = Vec::new();
        for chunk in payload.chunks(1170) {
            let mut b = vec![1, 0];
            b.extend((count as u16).to_le_bytes());
            b.extend(self.tx_next.to_le_bytes());
            b.extend(((chunk.len() + 14) as u16).to_le_bytes());
            b.extend(chunk);
            self.outgoing.insert(
                self.tx_next,
                Pending {
                    bytes: b.clone(),
                    last_sent: None,
                    retries: 0,
                    input,
                },
            );
            self.tx_next += 1;
            out.push(b)
        }
        if input {
            self.input_packets += count as u64;
        }
        Ok(out)
    }
    pub fn pending(&self) -> impl Iterator<Item = &[u8]> {
        self.outgoing.values().map(|p| p.bytes.as_slice())
    }
    /// Commit timestamps only after the carrier successfully flushed the batch.
    pub fn transmitted(&mut self, started: Instant) {
        for packet in self.outgoing.values_mut() {
            packet.last_sent.get_or_insert(started);
        }
    }
    pub fn retry_due(
        &mut self,
        now: Instant,
        mut send: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        for packet in self.outgoing.values_mut() {
            if !packet
                .last_sent
                .is_some_and(|sent| now.saturating_duration_since(sent) >= RETRY_INTERVAL)
            {
                continue;
            }
            ensure!(
                packet.retries < MAX_RETRIES,
                "Phaze reliable command acknowledgment timed out"
            );
            let retry = packet.retries + 1;
            packet.bytes[1] = retry;
            send(&packet.bytes)?;
            self.input_retries += u64::from(packet.input);
            packet.retries = retry;
            packet.last_sent = Some(now);
        }
        Ok(())
    }
    pub fn input_delivery(&self, now: Instant) -> InputDelivery {
        let mut outstanding = 0;
        let mut oldest = None;
        for packet in self.outgoing.values().filter(|packet| packet.input) {
            outstanding += 1;
            if let Some(sent) = packet.last_sent {
                oldest = Some(oldest.map_or(sent, |old: Instant| old.min(sent)));
            }
        }
        InputDelivery {
            sent: self.input_packets,
            acknowledged: self.input_packets - outstanding,
            retransmitted: self.input_retries,
            oldest_unacknowledged_attempt: oldest.map(|sent| now.saturating_duration_since(sent)),
        }
    }
    pub fn rtt(&self, now: Instant) -> Option<Duration> {
        self.rtt
            .filter(|(at, _)| now.saturating_duration_since(*at) < Duration::from_secs(10))
            .map(|(_, elapsed)| elapsed)
    }
    #[cfg(test)]
    fn receive(&mut self, b: &[u8]) -> Result<Received> {
        self.receive_at(b, Instant::now())
    }
    pub(crate) fn receive_at(&mut self, b: &[u8], now: Instant) -> Result<Received> {
        if b.is_empty() {
            bail!("Empty reliable packet")
        }
        if b[0] == 2 {
            ensure!(b.len() >= 18, "Invalid individual acknowledgment length");
            let seq = u64::from_le_bytes(b[2..10].try_into().unwrap());
            let frontier = u64::from_le_bytes(b[10..18].try_into().unwrap());
            ensure!(
                seq < self.tx_next && frontier <= self.tx_next,
                "Acknowledgment outside sent window"
            );
            // A late duplicate or retransmission ACK covers one packet and
            // the contiguous receive frontier. A replaced transmission is
            // never used for an RTT sample.
            if let Some(packet) = self.outgoing.get(&seq) {
                if let Some(elapsed) = packet
                    .last_sent
                    .filter(|_| b[1] == 0 && packet.retries == 0)
                    .and_then(|sent| now.checked_duration_since(sent))
                {
                    self.rtt = Some((now, elapsed));
                }
                self.outgoing.retain(|&id, _| id != seq && id >= frontier);
            }
            return Ok((Vec::new(), None));
        }
        if matches!(b[0], 3 | 4) {
            ensure!(
                b.len() == if b[0] == 3 { 34 } else { 50 },
                "Invalid acknowledgment length"
            );
            let frontier = u64::from_le_bytes(b[10..18].try_into().unwrap());
            let anchor = u64::from_le_bytes(b[18..26].try_into().unwrap());
            let bitmap = u64::from_le_bytes(b[26..34].try_into().unwrap());
            ensure!(
                frontier <= self.tx_next && (self.tx_next == 0 || anchor < self.tx_next),
                "Acknowledgment outside sent window"
            );
            let mut latest_sent = None;
            self.outgoing.retain(|seq, packet| {
                let acknowledged = *seq < frontier
                    || anchor
                        .checked_sub(*seq)
                        .is_some_and(|d| d < 64 && bitmap & (1 << d) != 0);
                // Karn's rule: an ACK cannot identify which transmission it acknowledges.
                if acknowledged && packet.retries == 0 {
                    latest_sent = latest_sent.max(packet.last_sent);
                }
                !acknowledged
            });
            // One sample per ACK, from its newest newly acknowledged packet.
            // Duplicate ACKs and packets never flushed provide no measurement.
            if let Some(elapsed) = latest_sent.and_then(|sent| now.checked_duration_since(sent)) {
                self.rtt = Some((now, elapsed));
            }
            return Ok((Vec::new(), None));
        }
        if b[0] == 6 {
            let ack = if let Some(highest) = self.highest {
                self.bandwidth.probe(b, highest, now);
                self.bandwidth
                    .take_feedback()
                    .and_then(|sample| self.ack(Some(sample)))
            } else {
                None
            };
            return Ok((Vec::new(), ack));
        }
        if !matches!(b[0], 1 | 5) {
            return Ok((Vec::new(), None));
        }
        ensure!(b.len() >= 14, "Truncated fragment");
        let count = u16::from_le_bytes(b[2..4].try_into().unwrap()) as usize;
        let seq = u64::from_le_bytes(b[4..12].try_into().unwrap());
        let len = u16::from_le_bytes(b[12..14].try_into().unwrap()) as usize;
        ensure!(
            len >= 14 && len <= b.len() && count > 0,
            "Invalid fragment bounds"
        );
        ensure!(
            seq < self.next.saturating_add(16384),
            "Sequence outside bounded receive window"
        );
        let anchor = self.highest.unwrap_or(seq);
        if seq > anchor {
            self.ack_bitmap = self
                .ack_bitmap
                .checked_shl((seq - anchor).min(64) as u32)
                .unwrap_or(0)
                | 1;
            self.highest = Some(seq);
        } else {
            self.highest = Some(anchor);
            if anchor - seq < 64 {
                self.ack_bitmap |= 1 << (anchor - seq);
            }
        }
        let duplicate = seq < self.next || self.packets.contains_key(&seq);
        if !duplicate {
            ensure!(self.buffered + len <= MAX_FRAME, "Reassembly memory limit");
            self.buffered += len;
            self.packets.insert(seq, b[..len].to_vec());
            if b[1] == 0 {
                self.bandwidth.ordinary(seq, b.len(), now);
            }
        }
        if seq == self.frontier {
            while self.packets.contains_key(&self.frontier) {
                self.frontier += 1;
            }
        }
        let mut frames = Vec::new();
        while let Some(first) = self.packets.get(&self.next) {
            let count = u16::from_le_bytes(first[2..4].try_into().unwrap()) as u64;
            if first[0] == 5 {
                self.buffered -= self.packets.remove(&self.next).unwrap().len();
                self.next += 1;
                continue;
            }
            if self.frontier - self.next < count {
                break;
            }
            let mut frame = Vec::new();
            for i in 0..count {
                let part = self.packets.remove(&(self.next + i)).unwrap();
                self.buffered -= part.len();
                ensure!(
                    part[0] == 1
                        && u16::from_le_bytes(part[2..4].try_into().unwrap()) as u64 == count,
                    "Inconsistent fragment group"
                );
                frame.extend(&part[14..]);
            }
            self.bandwidth.complete_message(self.next, count);
            self.next += count;
            frames.push(frame);
        }
        let feedback = self.bandwidth.take_feedback();
        if feedback.is_none() && (duplicate || b[1] != 0) {
            let mut ack = Vec::with_capacity(18);
            ack.extend([2, b[1]]);
            ack.extend(seq.to_le_bytes());
            ack.extend(self.frontier.to_le_bytes());
            return Ok((frames, Some(ack)));
        }
        Ok((frames, self.ack(feedback)))
    }

    fn ack(&mut self, feedback: Option<(f64, f64)>) -> Option<Vec<u8>> {
        let anchor = self.highest?;
        let mut ack = vec![if feedback.is_some() { 4 } else { 3 }, 0];
        ack.extend(self.serial.to_le_bytes());
        self.serial += 1;
        ack.extend(self.frontier.to_le_bytes());
        ack.extend(anchor.to_le_bytes());
        ack.extend(self.ack_bitmap.to_le_bytes());
        if let Some((low, high)) = feedback {
            ack.extend(low.to_le_bytes());
            ack.extend(high.to_le_bytes());
        }
        Some(ack)
    }
}
#[cfg(test)]
#[test]
fn vendor_client_display_handshake_matches_reconstructed_wire_layout() -> Result<()> {
    let display = parse_virtual_client_display("1920x1080")?;
    let hello = hello_with_virtual_display(false, Some(display))?;
    assert_eq!(&hello[0..1], &[18]);
    assert_eq!(be(&hello, 1), 0x1c0);
    assert_eq!(be(&hello, 5), 1);
    assert_eq!(be(&hello, 9), 0x4556_5031);
    let name_len = be(&hello, 13) as usize;
    assert_eq!(&hello[17..17 + name_len], b"EvoParseq Virtual");
    let tail = 17 + name_len;
    assert_eq!(hello.len(), tail + 54);
    assert_eq!(be(&hello, tail), 0); // x
    assert_eq!(be(&hello, tail + 4), 0); // y
    assert_eq!(be(&hello, tail + 8), 1920);
    assert_eq!(be(&hello, tail + 12), 1080);
    assert_eq!(be(&hello, tail + 44), 10_000);
    assert_eq!(be(&hello, tail + 48), 10_000);
    assert_eq!(&hello[tail + 52..], &[1, 1]);
    assert_eq!(
        hello_with_virtual_display(true, None)?,
        hello(true),
        "Disabled experiment must preserve the current handshake"
    );
    assert_eq!(
        be(&hello_with_virtual_display(true, Some(display))?, 1),
        0x1c1
    );
    for invalid in [
        "0x1080",
        "1920x0",
        "1920x1080x60",
        "8192x4320",
        "1920X1080",
        "bad",
    ] {
        assert!(parse_virtual_client_display(invalid).is_err(), "{invalid}");
    }
    assert!(hello_with_virtual_display(
        false,
        Some(VirtualClientDisplay {
            width: 0,
            height: 1080
        })
    )
    .is_err());
    Ok(())
}

#[cfg(test)]
#[test]
fn wire_contracts() -> Result<()> {
    let data: Vec<u8> = (0..3000).map(|i| (i % 251) as u8).collect();
    let mut a = Reliable::default();
    let p = a.frame(&data)?;
    let mut b = Reliable::default();
    ensure!(b.receive(&p[2])?.0.is_empty(), "Reorder test");
    ensure!(b.receive(&p[0])?.0.is_empty(), "Gap test");
    ensure!(b.receive(&p[0])?.0.is_empty(), "Duplicate test");
    ensure!(b.receive(&p[1])?.0 == vec![data], "Reassembly test");
    ensure!(b.receive(&[1]).is_err(), "Truncation test");
    ensure!(
        hello(true) == vec![18, 0, 0, 0, 1, 0, 0, 0, 0],
        "Hello/raw audio test"
    );
    ensure!(
        hello(false) == vec![18, 0, 0, 0, 0, 0, 0, 0, 0],
        "Hello/AAC test"
    );
    ensure!(
        VideoConfig::default().request(7)?.len() == 25,
        "Request test"
    );
    ensure!(outputs(&[0; 12]).is_err(), "Output bounds test");
    ensure!(encoded(&[0; 23]).is_err(), "Video bounds test");
    ensure!(
        nal_types(&[0, 0, 0, 1, 0x67, 9, 0, 0, 1, 0x68]) == vec![7, 8],
        "NAL test"
    );
    Ok(())
}

#[cfg(test)]
#[test]
fn receive_frontier_releases_buffered_frames_and_keeps_ack_window() -> Result<()> {
    let mut sender = Reliable::default();
    let first = sender.frame(&vec![42; 3_000])?;
    let later = sender.frame(b"later")?;
    let mut receiver = Reliable::default();
    let (frames, ack) = receiver.receive(&first[0])?;
    assert!(frames.is_empty());
    assert_eq!(&ack.unwrap()[10..18], &1u64.to_le_bytes());
    for packet in [&later[0], &first[2]] {
        let (frames, ack) = receiver.receive(packet)?;
        assert!(frames.is_empty());
        assert_eq!(&ack.unwrap()[10..18], &1u64.to_le_bytes());
    }
    let (frames, ack) = receiver.receive(&first[1])?;
    assert_eq!(frames, [vec![42; 3_000], b"later".to_vec()]);
    assert_eq!(
        &ack.unwrap()[10..],
        &[4u64, 3, 15].map(u64::to_le_bytes).concat()
    );
    let mut far = later[0].clone();
    far[4..12].copy_from_slice(&67u64.to_le_bytes());
    let (_, ack) = receiver.receive(&far)?;
    assert_eq!(
        &ack.unwrap()[10..],
        &[4u64, 67, 1].map(u64::to_le_bytes).concat()
    );
    let (_, ack) = receiver.receive(&first[0])?;
    let ack = ack.unwrap();
    assert_eq!(&ack[..2], &[2, 0]);
    assert_eq!(&ack[2..], &[0u64, 4].map(u64::to_le_bytes).concat());
    far[4..12].copy_from_slice(&68u64.to_le_bytes());
    let (_, ack) = receiver.receive(&far)?;
    assert_eq!(
        &ack.unwrap()[10..],
        &[4u64, 68, 3].map(u64::to_le_bytes).concat()
    );
    Ok(())
}

#[cfg(test)]
#[test]
fn a_measured_train_extends_ack_without_advancing_sequence_for_probes() -> Result<()> {
    let start = Instant::now();
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let packets = sender.frame(&vec![9; 1170 * 5])?;
    for (index, packet) in packets.iter().enumerate() {
        let (_, ack) =
            receiver.receive_at(packet, start + Duration::from_micros(index as u64 * 200))?;
        assert_eq!(ack.unwrap()[0], 3);
    }
    let mut probe = vec![0; packets[0].len()];
    probe[..5].copy_from_slice(&[6, 1, 7, 5, 0]);
    probe[5..13].copy_from_slice(&4u64.to_le_bytes());
    for i in 1..=7 {
        probe[1] = i;
        let (frames, ack) = receiver.receive_at(
            &probe,
            start + Duration::from_micros((4 + u64::from(i)) * 200),
        )?;
        assert!(frames.is_empty());
        if i == 7 {
            let ack = ack.unwrap();
            assert_eq!(ack.len(), 50);
            assert_eq!(ack[0], 4);
            assert_eq!(&ack[10..18], &5u64.to_le_bytes());
            let (lower_rate, upper_rate) = ack_bandwidth(&ack).unwrap();
            for estimate in [lower_rate, upper_rate] {
                assert!((estimate - 5_920_000.0).abs() < 0.01);
            }
            sender.receive(&ack)?;
            assert!(sender.pending().next().is_none());
        } else {
            assert!(ack.is_none());
        }
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn retries_follow_each_command_during_continuous_input() -> Result<()> {
    let start = Instant::now();
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let mut previous = sender.frame(b"first motion")?.remove(0);
    sender.transmitted(start);
    for step in 1..=u32::from(MAX_RETRIES) + 1 {
        let at = start + RETRY_INTERVAL * step;
        let next = sender.frame(b"next motion")?.remove(0);
        sender.transmitted(at);
        let mut resent = Vec::new();
        sender.retry_due(at, |packet| {
            resent.push(packet.to_vec());
            Ok(())
        })?;
        previous[1] = 1;
        assert_eq!(resent, [previous.clone()]);
        let (_, ack) = receiver.receive(&resent[0])?;
        assert_eq!(ack.as_ref().unwrap()[..2], [2, 1]);
        sender.receive_at(&ack.unwrap(), at)?;
        assert_eq!(sender.pending().count(), 1);
        previous = next;
    }

    let mut at = start + RETRY_INTERVAL * (u32::from(MAX_RETRIES) + 1);
    for _ in 0..MAX_RETRIES {
        at += RETRY_INTERVAL;
        sender.retry_due(at, |_| Ok(()))?;
    }
    assert!(sender.retry_due(at + RETRY_INTERVAL, |_| Ok(())).is_err());
    Ok(())
}

#[cfg(test)]
#[test]
fn individual_acknowledges_a_retry_without_covering_a_gap() -> Result<()> {
    let at = Instant::now();
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let first = sender.frame(b"first")?.remove(0);
    let mut second = sender.frame(b"second")?.remove(0);
    sender.transmitted(at);
    second[1] = 2;
    let (frames, ack) = receiver.receive_at(&second, at)?;
    assert!(frames.is_empty());
    let ack = ack.unwrap();
    assert_eq!(ack, [2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(sender.receive_at(&ack[..17], at).is_err());
    sender.receive_at(&ack, at)?;
    assert_eq!(sender.pending().collect::<Vec<_>>(), [first.as_slice()]);
    assert_eq!(sender.rtt(at), None);
    let (frames, ack) = receiver.receive_at(&first, at)?;
    assert_eq!(frames, [b"first".to_vec(), b"second".to_vec()]);
    sender.receive_at(&ack.unwrap(), at)?;
    assert_eq!(sender.pending().count(), 0);
    Ok(())
}

#[cfg(test)]
#[test]
fn ack_rtt_handles_selective_cumulative_duplicate_and_expired_samples() -> Result<()> {
    let start = Instant::now();
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let first = sender.frame(b"first")?;
    sender.transmitted(start);
    let second = sender.frame(b"second")?;
    sender.transmitted(start + Duration::from_millis(10));
    assert_eq!(sender.rtt(start), None);
    let (_, ack) = receiver.receive(&second[0])?;
    let ack = ack.unwrap();
    sender.receive_at(&ack, start + Duration::from_millis(40))?;
    assert_eq!(
        sender.rtt(start + Duration::from_millis(40)),
        Some(Duration::from_millis(30))
    );
    assert_eq!(sender.pending().count(), 1);
    // A duplicate must neither change the sample nor refresh its expiry.
    sender.receive_at(&ack, start + Duration::from_millis(50))?;
    assert_eq!(
        sender.rtt(start + Duration::from_millis(50)),
        Some(Duration::from_millis(30))
    );
    let (_, ack) = receiver.receive(&first[0])?;
    let mut extended = ack.unwrap();
    extended[0] = 4;
    extended.extend([0; 16]); // Unmapped feedback fields are not RTT measurements.
    sender.receive_at(&extended, start + Duration::from_millis(70))?;
    assert_eq!(
        sender.rtt(start + Duration::from_millis(70)),
        Some(Duration::from_millis(70))
    );
    assert_eq!(sender.pending().count(), 0);
    sender.receive_at(&extended, start + Duration::from_secs(9))?;
    assert_eq!(sender.rtt(start + Duration::from_millis(10070)), None);
    Ok(())
}

#[cfg(test)]
#[test]
fn ack_rtt_ignores_retries_unsent_packets_and_invalid_feedback() -> Result<()> {
    let start = Instant::now();
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let packets = sender.frame(b"retry")?;
    sender.transmitted(start);
    sender.retry_due(start + Duration::from_millis(250), |_| Ok(()))?;
    let (_, ack) = receiver.receive(&packets[0])?;
    sender.receive_at(&ack.unwrap(), start + Duration::from_millis(300))?;
    assert_eq!(sender.rtt(start + Duration::from_millis(300)), None);
    let packets = sender.frame(b"not flushed")?;
    let (_, ack) = receiver.receive(&packets[0])?;
    sender.receive_at(&ack.unwrap(), start)?;
    assert_eq!(sender.rtt(start), None);
    let packets = sender.frame(b"fresh")?;
    sender.transmitted(start);
    let (_, ack) = receiver.receive(&packets[0])?;
    let ack = ack.unwrap();
    assert!(sender.receive_at(&ack[..33], start).is_err());
    let mut invalid = ack.clone();
    invalid[10..18].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(sender.receive_at(&invalid, start).is_err());
    assert_eq!(sender.pending().count(), 1);
    assert_eq!(sender.rtt(start), None);
    sender.receive_at(&ack, start + Duration::from_millis(15))?;
    assert_eq!(
        sender.rtt(start + Duration::from_millis(15)),
        Some(Duration::from_millis(15))
    );
    Ok(())
}

#[cfg(test)]
#[test]
fn ack_rtt_uses_newest_packet_in_a_cumulative_batch() -> Result<()> {
    let start = Instant::now();
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let first = sender.frame(b"first")?;
    sender.transmitted(start);
    let second = sender.frame(b"second")?;
    sender.transmitted(start + Duration::from_millis(20));
    receiver.receive(&first[0])?;
    let (_, ack) = receiver.receive(&second[0])?;
    sender.receive_at(&ack.unwrap(), start + Duration::from_millis(35))?;
    assert_eq!(
        sender.rtt(start + Duration::from_millis(35)),
        Some(Duration::from_millis(15))
    );
    assert_eq!(sender.pending().count(), 0);
    Ok(())
}

#[cfg(test)]
#[test]
fn outgoing_acknowledgments_and_bounded_retry_state() -> Result<()> {
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let packets = sender.frame(&vec![42; 2000])?;
    assert_eq!(sender.pending().count(), 2);
    let (_, ack) = receiver.receive(&packets[1])?;
    sender.receive(&ack.unwrap())?;
    assert_eq!(sender.pending().count(), 1);
    let (frames, ack) = receiver.receive(&packets[0])?;
    assert_eq!(frames, vec![vec![42; 2000]]);
    sender.receive(&ack.unwrap())?;
    assert_eq!(sender.pending().count(), 0);
    assert_eq!(receiver.buffered, 0);
    assert!(sender.receive(&[3]).is_err());
    let mut bad = vec![3; 34];
    bad[10..18].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(sender.receive(&bad).is_err());
    Ok(())
}

#[cfg(test)]
#[test]
fn input_delivery_distinguishes_acknowledged_and_retransmitted_packets() -> Result<()> {
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let start = Instant::now();
    let control = sender.frame(&[18])?;
    sender.frame(&[8])?;
    sender.transmitted(start);
    let (_, ack) = receiver.receive(&control[0])?;
    sender.receive_at(&ack.unwrap(), start)?;
    assert_eq!(sender.input_delivery(start).sent, 1);
    assert_eq!(sender.input_delivery(start).acknowledged, 0);

    let retry = start + RETRY_INTERVAL;
    sender.retry_due(retry, |packet| {
        let (_, ack) = receiver.receive_at(packet, retry)?;
        assert!(ack.is_some());
        Ok(())
    })?;
    let delivery = sender.input_delivery(retry);
    assert_eq!(delivery.retransmitted, 1);
    assert_eq!(delivery.oldest_unacknowledged_attempt, Some(Duration::ZERO));
    let (_, ack) = receiver.receive_at(sender.pending().next().unwrap(), retry)?;
    sender.receive_at(&ack.unwrap(), retry)?;
    let delivery = sender.input_delivery(retry);
    assert_eq!(delivery.acknowledged, 1);
    assert_eq!(delivery.oldest_unacknowledged_attempt, None);
    Ok(())
}

#[cfg(test)]
#[test]
fn protected_heartbeat_is_not_reliable_or_application_data() {
    assert_eq!(HEARTBEAT, [0xcd, 3]);
    assert!(matches!(envelope(&HEARTBEAT), Some(Envelope::Heartbeat)));
    assert!(matches!(envelope(&[0xcd, 1]), Some(Envelope::Close)));
    match envelope(&[0xcd, 2, 1, 9]) {
        Some(Envelope::Reliable(payload)) => assert_eq!(payload, [1, 9]),
        _ => panic!("Reliable envelope was not recognized"),
    }
    for bytes in [&[][..], &[0xcd][..], &[0, 3][..], &[0xcd, 4][..]] {
        assert!(envelope(bytes).is_none());
    }
}

#[cfg(test)]
#[test]
fn raw_audio_checks_encoding_sample_count_and_stereo_alignment() {
    let mut audio = crate::audio::Audio::default();
    let mut packet = vec![0, 0, 0, 0, 2, 0, 128, 255, 127, 1, 0, 255, 255];
    assert_eq!(audio.decode(&packet).unwrap().1, &[-32768, 32767, 1, -1]);
    packet[0] = 1;
    assert!(audio.decode(&packet).is_err());
    packet[0] = 0;
    packet.pop();
    assert!(audio.decode(&packet).is_err());
    assert!(audio.decode(&[0, 255, 255, 255, 255]).is_err());
    assert!(audio.decode(&[]).is_err());
}

#[cfg(test)]
#[test]
fn timing_feedback_also_acknowledges_outgoing_input() -> Result<()> {
    let mut sender = Reliable::default();
    let mut receiver = Reliable::default();
    let packet = sender.frame(&crate::Input::Release.encode(0))?.remove(0);
    let (_, ack) = receiver.receive(&packet)?;
    let mut timing = ack.unwrap();
    timing[0] = 4;
    timing.extend([0; 16]);
    sender.receive(&timing)?;
    assert_eq!(sender.pending().count(), 0);
    timing.pop();
    assert!(sender.receive(&timing).is_err());
    Ok(())
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    #[test]
    fn stream_options_have_bounded_independent_wire_encoding() -> Result<()> {
        let config = VideoConfig {
            hevc: true,
            chroma_444: true,
            mbps: 50,
            fps: 144,
        };
        let packet = config.request(7)?;
        let mut expected = vec![3];
        for word in [7u32, 3, 50, 144, 4096, 4096] {
            expected.extend(word.to_be_bytes());
        }
        assert_eq!(packet, expected);
        assert!(VideoConfig { fps: 0, ..config }.validate().is_err());
        assert!(VideoConfig {
            mbps: 101,
            ..config
        }
        .validate()
        .is_err());
        assert!(VideoConfig {
            hevc: false,
            ..config
        }
        .validate()
        .is_err());
        assert!(VideoConfig {
            hevc: false,
            chroma_444: false,
            ..config
        }
        .validate()
        .is_ok());
        Ok(())
    }
}
