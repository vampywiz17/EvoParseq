//! The pointer message, opcode 9.
//!
//! Thirteen bytes of header then twenty-one of body, and optionally an image
//! after that. Two forms share the layout: one carries the picture, the other
//! names a picture the far side already has.
//!
//! **Three states travel here and none of them implies another**
//! (docs/05-host.md section 8.1). Hidden is what an application asked for and
//! is what relative mode is derived from; suppressed is the pointer withheld
//! because somebody is using touch, and is not relative; whether a pointer is
//! being drawn at all is a third thing again.

use crate::control::{Control, op};
use crate::error::{Error, Result};

/// Body length, after the thirteen-byte header.
pub const BODY_LEN: usize = 21;

/// The bits one update carries.
///
/// **A set, not an enumeration.** Several are set at once and the far side
/// reads each independently, so the type models a set of bits rather than a
/// choice between them. Naming them here is what keeps the values in one
/// place: anything that spells `0x0002` for itself agrees with the encoder
/// only until one of them changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u16);

impl Flags {
    /// Nothing set.
    pub const NONE: Self = Self(0);
    /// An image travels with this message.
    pub const IMAGE: Self = Self(0x0002);
    /// No image: the far side already holds one, named by its checksum.
    pub const CACHED: Self = Self(0x0010);
    /// The far side must forget every image it holds before reading this one.
    pub const FORGET: Self = Self(0x0020);
    /// Motion should be sent as deltas rather than positions.
    pub const RELATIVE: Self = Self(0x0100);
    /// The pointer was hidden by an application. **Relative mode is derived
    /// from this and from nothing else**, but the two are separate bits and
    /// the far side reads them separately.
    pub const HIDDEN: Self = Self(0x0200);
    /// The pointer is withheld because input is arriving by touch. **Not
    /// relative**, and folding it into relative traps a pointer that was never
    /// taken over.
    pub const SUPPRESSED: Self = Self(0x0800);

    /// The value as it goes on the wire.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Read a set off the wire.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// True when every bit of `other` is set here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for Flags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for Flags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// What to say about the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Update {
    /// Which stream this pointer belongs to. A host with one stream sends
    /// zero; the field exists because a pointer belongs to the picture it is
    /// drawn on, not to the session.
    pub stream: u8,
    /// **Relative to the stream's own corner, not the desktop's.** A pointer
    /// outside the stream is not this stream's to report at all.
    pub x: u16,
    pub y: u16,
    /// Where inside the image the pointer actually points.
    pub hot_x: u16,
    pub hot_y: u16,
    pub hidden: bool,
    pub relative: bool,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Message<'a> {
    pub update: Update,
    pub flags: Flags,
    /// The picture's size, when one travels; zero otherwise.
    pub width: u16,
    pub height: u16,
    /// The checksum a cached picture is named by; zero when none was.
    pub checksum: u32,
    /// The picture itself, empty unless the image bit is set.
    pub image: &'a [u8],
}

fn be16(body: &[u8], at: usize) -> Result<u16> {
    body.get(at..at + 2)
        .and_then(|s| <[u8; 2]>::try_from(s).ok())
        .map(u16::from_be_bytes)
        .ok_or(Error::ShortPacket)
}

/// Read an update off a control message.
///
/// **The two forms are told apart by their bits, read independently.** With
/// the image bit the hotspot is in the body and the picture follows it, as
/// many bytes as the declared size; with the cached bit the hotspot is in the
/// header's arguments and the checksum sits where the hotspot would have
/// been. A body shorter than the fixed part is refused; a declared image
/// longer than what arrived is taken at what arrived.
pub fn parse<'a>(message: &Control<'a>) -> Result<Message<'a>> {
    if message.opcode != op::CURSOR {
        return Err(Error::ShortPacket);
    }
    let body = message.body;
    let size = body
        .get(3..7)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
        .map(u32::from_be_bytes)
        .ok_or(Error::ShortPacket)?;
    let width = be16(body, 7)?;
    let height = be16(body, 9)?;
    let x = be16(body, 11)?;
    let y = be16(body, 13)?;
    let at15 = be16(body, 15)?;
    let at17 = be16(body, 17)?;
    let flags = Flags::from_bits(be16(body, 19)?);
    let (hot_x, hot_y, checksum) = if flags.contains(Flags::CACHED) {
        (
            u16::try_from(message.a0 & 0xFFFF).unwrap_or(0),
            u16::try_from(message.a1 & 0xFFFF).unwrap_or(0),
            u32::from(at15) | (u32::from(at17) << 16),
        )
    } else {
        (at15, at17, 0)
    };
    let image = if flags.contains(Flags::IMAGE) {
        let rest = body.get(BODY_LEN..).unwrap_or(&[]);
        rest.get(..usize::try_from(size).unwrap_or(usize::MAX))
            .unwrap_or(rest)
    } else {
        &[]
    };
    Ok(Message {
        update: Update {
            stream: u8::try_from(message.a2 & 0xFF).unwrap_or(0),
            x,
            y,
            hot_x,
            hot_y,
            hidden: flags.contains(Flags::HIDDEN),
            relative: flags.contains(Flags::RELATIVE),
            suppressed: flags.contains(Flags::SUPPRESSED),
        },
        flags,
        width,
        height,
        checksum,
        image,
    })
}
