use std::collections::VecDeque;

const MAX_SIDE: u32 = 256;
const MAX_IMAGE: usize = 512 * 1024;
const CACHE_ENTRIES: usize = 128;
const CACHE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorFormat {
    Rgba,
    Monochrome,
    Png,
}

pub struct CursorImage<'a> {
    pub format: CursorFormat,
    pub size: (u32, u32),
    pub hotspot: (u32, u32),
    pub bytes: &'a [u8],
}

struct Shape {
    id: u32,
    format: CursorFormat,
    size: (u32, u32),
    hotspot: (u32, u32),
    bytes: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Cursors {
    shapes: VecDeque<Shape>,
    bytes: usize,
    active: Option<u32>,
}

impl Cursors {
    pub fn receive(&mut self, frame: &[u8]) -> Option<CursorImage<'_>> {
        let (&opcode, body) = frame.split_first()?;
        let id = match opcode {
            12 => {
                let (header, bytes) = body.split_at_checked(28)?;
                let [kind, width, height, hot_x, hot_y, _, id] =
                    std::array::from_fn(|i| crate::wire::be(header, i * 4));
                if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
                    return None;
                }
                let pixels = (width * height) as usize;
                let format = match kind {
                    1 if bytes.len() == pixels * 4 => CursorFormat::Rgba,
                    2 if bytes.len() == pixels.div_ceil(8) * 2 => CursorFormat::Monochrome,
                    3 if bytes.len() <= MAX_IMAGE && bytes.starts_with(b"\x89PNG\r\n\x1a\n") => {
                        CursorFormat::Png
                    }
                    _ => return None,
                };
                if let Some(index) = self.shapes.iter().position(|shape| shape.id == id) {
                    self.bytes -= self.shapes.remove(index)?.bytes.len();
                }
                while self.shapes.len() >= CACHE_ENTRIES || self.bytes + bytes.len() > CACHE_BYTES {
                    self.bytes -= self.shapes.pop_front()?.bytes.len();
                }
                self.bytes += bytes.len();
                self.shapes.push_back(Shape {
                    id,
                    format,
                    size: (width, height),
                    hotspot: (hot_x.min(width - 1), hot_y.min(height - 1)),
                    bytes: bytes.to_vec(),
                });
                self.active = None;
                id
            }
            19 if body.len() == 4 => crate::wire::be(body, 0),
            _ => return None,
        };
        if self.active == Some(id) {
            return None;
        }
        let shape = self.shapes.iter().find(|shape| shape.id == id)?;
        self.active = Some(id);
        Some(CursorImage {
            format: shape.format,
            size: shape.size,
            hotspot: shape.hotspot,
            bytes: &shape.bytes,
        })
    }
}

#[cfg(test)]
#[test]
fn cursor_update_and_cached_reference_preserve_shape_and_hotspot() {
    let mut cursors = Cursors::default();
    let full = |id, format, bytes: &[u8]| {
        let mut packet = vec![12];
        for field in [format, 2u32, 3, 1, 2, 10_000, id] {
            packet.extend(field.to_be_bytes());
        }
        packet.extend(bytes);
        packet
    };
    let first = full(7, 2, &[0x80, 0x40]);
    let second = full(8, 1, &[0xff; 24]);
    assert!(cursors.receive(&first[..first.len() - 1]).is_none());
    let mono = cursors.receive(&first).unwrap();
    assert_eq!(mono.format, CursorFormat::Monochrome);
    assert_eq!(mono.hotspot, (1, 2));
    assert_eq!(cursors.receive(&second).unwrap().format, CursorFormat::Rgba);
    let cached = [19, 0, 0, 0, 7];
    assert_eq!(cursors.receive(&cached).unwrap().bytes, [0x80, 0x40]);
    assert!(cursors.receive(&cached).is_none());
    assert!(cursors.receive(&[19, 0, 0, 0, 9]).is_none());
}
