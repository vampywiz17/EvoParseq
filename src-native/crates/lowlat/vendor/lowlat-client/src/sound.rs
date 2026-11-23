//! Bounded compressed-audio packet pool. The application owns decoding.
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use lowlat_common::pool::{self, Pool};
use lowlat_common::spsc::Ring;
use lowlat_core::audio;

/// Packets waiting for the application: 640 ms at the host's 20 ms.
pub const SLOTS: usize = 32;
/// The longest sound packet: the uncompressed ceiling plus its header.
pub const PACKET_BYTES: usize = audio::PCM_PAYLOAD_MAX + audio::AUDIO_HEADER_LEN;
/// Received sound packets, on their way to the application.
///
/// One producer (the session's thread) and one consumer. Shaped as the
/// access units are: the pool holds the bytes, the ring carries slot
/// indices, the word wakes the consumer.
#[derive(Debug, Clone)]
pub struct Packets {
    pool: Arc<Pool>,
    ring: Arc<Ring<u32, SLOTS>>,
    word: Arc<AtomicU32>,
}

impl Default for Packets {
    fn default() -> Self {
        Self::new()
    }
}

impl Packets {
    pub fn new() -> Self {
        Self {
            pool: Arc::new(Pool::new(SLOTS, PACKET_BYTES)),
            ring: Arc::new(Ring::new()),
            word: Arc::new(AtomicU32::new(0)),
        }
    }

    /// A slot to write the next packet into, or `None` while every one is
    /// still held: the producer's.
    pub(crate) fn writer(&self) -> Option<pool::Writer<'_>> {
        self.pool.acquire()
    }

    /// Hand a filled slot over, stamped with when it arrived; `false` when
    /// the ring refused, which cannot happen while it is as deep as the pool.
    pub(crate) fn publish(&self, writer: pool::Writer<'_>, arrived_ms: u32) -> bool {
        if writer.publish(arrived_ms, &[&self.ring]) == 0 {
            return false;
        }
        self.word.fetch_add(1, Ordering::Release);
        lowlat_common::wait::notify_one(&self.word);
        true
    }

    /// Wait up to `timeout` for a packet to be handed over, or for a wake.
    /// The consumer's; it rechecks after.
    pub fn wait(&self, timeout: Duration) {
        let word = self.word.load(Ordering::Acquire);
        if self.ring.is_empty() {
            lowlat_common::wait::wait(&self.word, word, timeout);
        }
    }

    /// Wake the consumer without a packet: teardown.
    pub fn wake(&self) {
        self.word.fetch_add(1, Ordering::Release);
        lowlat_common::wait::notify_all(&self.word);
    }

    /// The next packet, in order, or `None` while nothing waits.
    fn take(&self) -> Option<Packet<'_>> {
        let index = self.ring.pop()?;
        let frame = self.pool.claim(index)?;
        Some(Packet { frame })
    }

    pub(crate) fn take_encoded(&self) -> Option<EncodedAudio<'_>> {
        self.take().map(|packet| EncodedAudio { packet })
    }

    pub fn clear(&self) {
        while self.take().is_some() {}
    }

    /// Packets handed over and not yet taken.
    pub fn queued(&self) -> usize {
        self.ring.len()
    }
}

/// One packet, header included, as it came off the wire.
#[derive(Debug)]
struct Packet<'a> {
    frame: pool::Frame<'a>,
}

/// Application-owned compressed audio lease; no SDK audio decoding or PCM copy.
pub struct EncodedAudio<'a> {
    packet: Packet<'a>,
}

impl EncodedAudio<'_> {
    pub fn bytes(&self) -> &[u8] {
        self.packet.bytes()
    }
}

impl Packet<'_> {
    fn bytes(&self) -> &[u8] {
        self.frame.bytes()
    }
}

/// Exclusive application audio consumer, detached from the session command lock.
pub struct EncodedAudioSource(Packets);
impl EncodedAudioSource {
    pub(crate) fn new(packets: Packets) -> Self {
        Self(packets)
    }
    pub fn take(&mut self) -> Option<EncodedAudio<'_>> {
        self.0.take_encoded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_audio_lease_keeps_bytes_and_releases_pool_slot() {
        let packets = Packets::new();
        for n in 0..SLOTS {
            let mut writer = packets.writer().unwrap();
            assert!(writer.fill(&[n as u8; 4]));
            assert!(packets.publish(writer, n as u32));
        }
        assert!(packets.writer().is_none());
        let mut source = EncodedAudioSource::new(packets.clone());
        let first = source.take().unwrap();
        assert_eq!(first.bytes(), &[0; 4]);
        assert!(packets.writer().is_none());
        drop(first);
        assert!(packets.writer().is_some());
        drop(source);
        packets.clear();
        assert_eq!(packets.queued(), 0);
        for _ in 0..SLOTS {
            assert!(packets.writer().is_some());
        }
    }
}
