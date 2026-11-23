//! Cursor cache only, from lowlat e548d48 (MIT). PNG decoding stays in EvoParseq.
use lowlat_core::crc32;

/// Pictures the cache holds, which is the number a host sends before it
/// tells the far side to forget them all.
pub const CAPACITY: usize = 100;

/// The widest and tallest picture the reader takes.
pub const MAX_SIDE: u32 = 512;

/// Decoded bytes at the ceiling: 512 square, four bytes a pixel, 1 MiB.
pub const MAX_DECODED: usize = (MAX_SIDE as usize) * (MAX_SIDE as usize) * 4;

/// The longest PNG the cache keeps or the reader looks at. A picture at the
/// ceiling stored uncompressed is the decoded size plus a few bytes per
/// block; anything past that cannot be a picture the reader would accept.
pub const MAX_PNG: usize = MAX_DECODED + 64 * 1024;

/// What the cache holds in all, past which it starts over.
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// One picture a host may name again.
#[derive(Debug, Clone)]
struct Entry {
    checksum: u32,
    width: u16,
    height: u16,
    hot_x: u16,
    hot_y: u16,
    png: Vec<u8>,
}

/// The size and hotspot a name resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
}

/// Pictures by checksum, as the host expects this client to keep them.
#[derive(Debug, Default)]
pub struct Cache {
    entries: Vec<Entry>,
    bytes: usize,
}

impl Cache {
    /// Keep a picture the host just sent, keyed by the checksum it will name
    /// it by. Returns that checksum.
    ///
    /// **Full means start over, not evict one.** The host counts what it
    /// sent and tells this side to forget at its own capacity; a cache that
    /// evicted one at a time would drop a picture the host still names.
    pub fn insert(&mut self, shape: Shape, png: &[u8]) -> u32 {
        let checksum = crc32::of(png);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.checksum == checksum) {
            entry.width = shape.width;
            entry.height = shape.height;
            entry.hot_x = shape.hot_x;
            entry.hot_y = shape.hot_y;
            return checksum;
        }
        if self.entries.len() >= CAPACITY || self.bytes.saturating_add(png.len()) > MAX_CACHE_BYTES
        {
            lowlat_common::log_warn!(
                "client: cursor cache full, forgetting all, entries={} bytes={}",
                self.entries.len(),
                self.bytes
            );
            self.clear();
        }
        self.bytes = self.bytes.saturating_add(png.len());
        self.entries.push(Entry {
            checksum,
            width: shape.width,
            height: shape.height,
            hot_x: shape.hot_x,
            hot_y: shape.hot_y,
            png: png.to_vec(),
        });
        checksum
    }

    /// The picture a name resolves to, with the shape stored beside it.
    pub fn get(&self, checksum: u32) -> Option<(Shape, &[u8])> {
        let entry = self.entries.iter().find(|e| e.checksum == checksum)?;
        Some((
            Shape {
                width: entry.width,
                height: entry.height,
                hot_x: entry.hot_x,
                hot_y: entry.hot_y,
            },
            &entry.png,
        ))
    }

    /// Forget every picture: the host's forget bit, or a full cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
