//! Bounded host cursor pixels, independent of window and video backends.
use anyhow::{bail, Result};
use evoparseq_host_interop::{CursorFormat, CursorImage};

pub(crate) const MAX_SIDE: u32 = 256;
const MAX_ENCODED: usize = 1024 * 1024;

pub(crate) struct Pixels {
    pub size: (u32, u32),
    pub hotspot: (u32, u32),
    pub rgba: Vec<u8>,
}

pub(crate) fn png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    if bytes.len() > MAX_ENCODED {
        bail!("encoded cursor exceeds limit");
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(2 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok((image.into_raw(), width, height))
}

impl Pixels {
    pub fn decode(image: CursorImage<'_>) -> Result<Self> {
        let (width, height) = image.size;
        if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
            bail!("cursor dimensions exceed limit");
        }
        let count = (width * height) as usize;
        let rgba = match image.format {
            CursorFormat::Png => {
                let (bytes, w, h) = png(image.bytes)?;
                if (w, h) != image.size {
                    bail!("cursor PNG dimensions mismatch");
                }
                bytes
            }
            CursorFormat::Rgba => {
                if image.bytes.len() != count * 4 {
                    bail!("cursor pixel length mismatch");
                }
                image.bytes.to_vec()
            }
            CursorFormat::Monochrome => {
                let mask = count.div_ceil(8);
                if image.bytes.len() != mask * 2 {
                    bail!("cursor mask length mismatch");
                }
                (0..count)
                    .flat_map(|i| {
                        let and = image.bytes[i / 8] & (0x80 >> (i % 8)) != 0;
                        let xor = image.bytes[mask + i / 8] & (0x80 >> (i % 8)) != 0;
                        // RGBA cannot invert the underlying desktop. Represent
                        // inverse pixels as white; never read back the video.
                        let value = if xor { 255 } else { 0 };
                        [value, value, value, if and && !xor { 0 } else { 255 }]
                    })
                    .collect()
            }
        };
        Ok(Self {
            size: image.size,
            hotspot: (
                image.hotspot.0.min(width - 1),
                image.hotspot.1.min(height - 1),
            ),
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decode(format: CursorFormat, size: (u32, u32), bytes: &[u8]) -> Result<Pixels> {
        Pixels::decode(CursorImage {
            format,
            size,
            hotspot: (99, 99),
            bytes,
        })
    }
    #[test]
    fn linux_cursor_pixels_validate_dimensions_lengths_and_hotspot() {
        for size in [(0, 1), (1, 0), (257, 1), (1, 257)] {
            assert!(decode(CursorFormat::Rgba, size, &[]).is_err());
        }
        assert!(decode(CursorFormat::Rgba, (1, 1), &[1; 3]).is_err());
        let p = decode(CursorFormat::Rgba, (1, 1), &[1, 2, 3, 4]).unwrap();
        assert_eq!(p.rgba, [1, 2, 3, 4]);
        assert_eq!(p.hotspot, (0, 0));
        assert_eq!(p.size, (1, 1));
        let p = Pixels::decode(CursorImage {
            format: CursorFormat::Rgba,
            size: (2, 2),
            hotspot: (1, 0),
            bytes: &[0; 16],
        })
        .unwrap();
        assert_eq!(p.hotspot, (1, 0));
    }
    #[test]
    fn linux_cursor_monochrome_preserves_masks_and_partial_bytes() {
        // black, white, transparent, inverse; masks are MSB first.
        let p = decode(CursorFormat::Monochrome, (4, 1), &[0x30, 0x50]).unwrap();
        assert_eq!(
            p.rgba,
            [0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255]
        );
        assert!(decode(CursorFormat::Monochrome, (9, 1), &[0; 3]).is_err());
        let p = decode(CursorFormat::Monochrome, (9, 1), &[0xff, 0x80, 0, 0]).unwrap();
        assert_eq!(p.rgba.len(), 36);
        assert!(p.rgba.chunks_exact(4).all(|p| p[3] == 0));
    }
    #[test]
    fn linux_cursor_png_is_bounded_and_matches_declared_geometry() {
        use image::ImageEncoder;
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[7, 8, 9, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let p = decode(CursorFormat::Png, (1, 1), &bytes).unwrap();
        assert_eq!(p.rgba, [7, 8, 9, 255]);
        assert!(decode(CursorFormat::Png, (2, 1), &bytes).is_err());
        assert!(decode(CursorFormat::Png, (1, 1), b"not PNG").is_err());
        assert!(png(&vec![0; MAX_ENCODED + 1]).is_err());
        let mut oversized = Vec::new();
        image::codecs::png::PngEncoder::new(&mut oversized)
            .write_image(&vec![0; 257 * 4], 257, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(png(&oversized).is_err());
    }
}
