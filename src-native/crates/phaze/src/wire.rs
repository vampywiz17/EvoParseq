use anyhow::{bail, ensure, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
type Received = (Vec<Vec<u8>>, Option<Vec<u8>>);
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
fn be(b: &[u8], i: usize) -> u32 {
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
pub fn request(output: u32) -> Vec<u8> {
    let mut b = vec![3];
    for x in [output, 0, 10, 30, 4096, 4096] {
        b.extend(x.to_be_bytes())
    }
    b
}
pub fn hello() -> Vec<u8> {
    let mut b = vec![18];
    b.extend([0; 8]);
    b
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
    next: u64,
    highest: Option<u64>,
    serial: u64,
    packets: BTreeMap<u64, Vec<u8>>,
    seen: BTreeSet<u64>,
    tx_next: u64,
    outgoing: BTreeMap<u64, Vec<u8>>,
    buffered: usize,
}
impl Reliable {
    pub fn frame(&mut self, payload: &[u8]) -> Result<Vec<Vec<u8>>> {
        ensure!(payload.len() <= MAX_FRAME, "Outgoing frame too large");
        ensure!(
            self.outgoing.len() + payload.len().div_ceil(1170) <= 64,
            "Outgoing reliable window exceeded"
        );
        let count = payload.len().div_ceil(1170);
        ensure!(
            count > 0 && count <= u16::MAX as usize,
            "Invalid fragment count"
        );
        let mut out = Vec::new();
        for chunk in payload.chunks(1170) {
            let mut b = vec![1, 0];
            b.extend((count as u16).to_le_bytes());
            b.extend(self.tx_next.to_le_bytes());
            b.extend(((chunk.len() + 14) as u16).to_le_bytes());
            b.extend(chunk);
            self.outgoing.insert(self.tx_next, b.clone());
            self.tx_next += 1;
            out.push(b)
        }
        Ok(out)
    }
    pub fn pending(&self) -> impl Iterator<Item = &[u8]> {
        self.outgoing.values().map(Vec::as_slice)
    }
    pub fn receive(&mut self, b: &[u8]) -> Result<Received> {
        if b.is_empty() {
            bail!("Empty reliable packet")
        }
        if b[0] == 3 {
            ensure!(b.len() == 34, "Invalid acknowledgment length");
            let frontier = u64::from_le_bytes(b[10..18].try_into().unwrap());
            let anchor = u64::from_le_bytes(b[18..26].try_into().unwrap());
            let bitmap = u64::from_le_bytes(b[26..34].try_into().unwrap());
            ensure!(
                frontier <= self.tx_next && (self.tx_next == 0 || anchor < self.tx_next),
                "Acknowledgment outside sent window"
            );
            self.outgoing.retain(|seq, _| {
                *seq >= frontier
                    && !anchor
                        .checked_sub(*seq)
                        .is_some_and(|d| d < 64 && bitmap & (1 << d) != 0)
            });
            return Ok((Vec::new(), None));
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
        self.highest = Some(self.highest.map_or(seq, |x| x.max(seq)));
        self.seen.insert(seq);
        if seq >= self.next && !self.packets.contains_key(&seq) {
            ensure!(self.buffered + len <= MAX_FRAME, "Reassembly memory limit");
            self.buffered += len;
            self.packets.insert(seq, b[..len].to_vec());
        }
        let mut frames = Vec::new();
        while let Some(first) = self.packets.get(&self.next) {
            let count = u16::from_le_bytes(first[2..4].try_into().unwrap()) as u64;
            if first[0] == 5 {
                self.buffered -= self.packets.remove(&self.next).unwrap().len();
                self.next += 1;
                continue;
            }
            if !(0..count).all(|i| self.packets.contains_key(&(self.next + i))) {
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
            self.next += count;
            frames.push(frame);
        }
        let mut frontier = self.next;
        while self.packets.contains_key(&frontier) {
            frontier += 1
        }
        let anchor = self.highest.unwrap();
        let mut bitmap = 0u64;
        for d in 0..64 {
            if let Some(s) = anchor.checked_sub(d) {
                if self.seen.contains(&s) {
                    bitmap |= 1 << d
                }
            }
        }
        self.seen.retain(|s| anchor.saturating_sub(*s) < 64);
        let mut ack = vec![3, 0];
        ack.extend(self.serial.to_le_bytes());
        self.serial += 1;
        ack.extend(frontier.to_le_bytes());
        ack.extend(anchor.to_le_bytes());
        ack.extend(bitmap.to_le_bytes());
        Ok((frames, Some(ack)))
    }
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
    ensure!(hello() == vec![18, 0, 0, 0, 0, 0, 0, 0, 0], "Hello test");
    ensure!(request(7).len() == 25, "Request test");
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
