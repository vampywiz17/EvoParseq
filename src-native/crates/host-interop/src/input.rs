//! Independent Phaze input serialization. Physical keys use PC set-1 scans;
//! modifiers and button numbers follow the public libmatoya vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Key {
        usage: u16,
        scan: u16,
        modifiers: u16,
        down: bool,
    },
    Motion {
        x: i32,
        y: i32,
        relative: bool,
    },
    Button {
        button: u8,
        x: i32,
        y: i32,
        down: bool,
    },
    Scroll {
        x: i32,
        y: i32,
    },
    Release,
}
/// The session's existing input batch, with a destination for each event.
#[derive(Default)]
pub struct InputBatch {
    pub(crate) items: Vec<(Option<u32>, Input)>,
    output: Option<u32>,
}

impl InputBatch {
    pub fn target(&mut self, output: Option<u32>) {
        self.output = output;
    }

    pub fn push(&mut self, command: Input) {
        self.items.push((self.output, command));
    }

    pub fn insert(&mut self, index: usize, command: Input) {
        self.items.insert(index, (self.output, command));
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Input> {
        self.items.iter().map(|(_, command)| command)
    }

    pub fn last(&self) -> Option<&Input> {
        self.items.last().map(|(_, command)| command)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.output = None;
    }
}

impl Extend<Input> for InputBatch {
    fn extend<T: IntoIterator<Item = Input>>(&mut self, commands: T) {
        self.items
            .extend(commands.into_iter().map(|command| (self.output, command)));
    }
}

impl FromIterator<Input> for InputBatch {
    fn from_iter<T: IntoIterator<Item = Input>>(commands: T) -> Self {
        let mut batch = Self::default();
        batch.extend(commands);
        batch
    }
}

impl std::ops::Index<usize> for InputBatch {
    type Output = Input;

    fn index(&self, index: usize) -> &Self::Output {
        &self.items[index].1
    }
}

impl Input {
    pub(crate) fn is_absolute_motion(self) -> bool {
        matches!(
            self,
            Self::Motion {
                relative: false,
                ..
            }
        )
    }

    pub(crate) fn encode(self, output: u32) -> Vec<u8> {
        let (opcode, values) = match self {
            Self::Release => return vec![13],
            Self::Key {
                usage: _,
                scan,
                modifiers,
                down,
            } => (
                8,
                [u32::from(scan), u32::from(modifiers), 0, u32::from(down)],
            ),
            Self::Scroll { x, y } => (9, [x as u32, y as u32, 0, 0]),
            Self::Button { button, x, y, down } => {
                (10, [u32::from(button), x as u32, y as u32, u32::from(down)])
            }
            Self::Motion { x, y, relative } => (11, [x as u32, y as u32, u32::from(relative), 0]),
        };
        let mut frame = Vec::with_capacity(21);
        frame.push(opcode);
        for word in [output].into_iter().chain(values) {
            frame.extend(word.to_be_bytes());
        }
        frame
    }
}

#[derive(Default)]
pub struct Keyboard {
    modifiers: u16,
}
impl Keyboard {
    pub fn reset(&mut self) {
        self.modifiers = 0;
    }
    /// Shared USB physical-key usages, independent of the local text layout.
    pub fn key(&mut self, usage: u16, compact: i32, down: bool) -> Option<Input> {
        let scan = scan(usage)?;
        if (224..=231).contains(&usage) {
            let bit = [4, 1, 16, 64, 8, 2, 32, 128][usize::from(usage - 224)];
            if down {
                self.modifiers |= bit;
            } else {
                self.modifiers &= !bit;
            }
        }
        let mut modifiers = self.modifiers;
        for (source, mask, fallback) in [(1, 3, 1), (2, 12, 4), (4, 48, 16), (8, 192, 64)] {
            if compact & source != 0 && modifiers & mask == 0 {
                modifiers |= fallback;
            }
        }
        if compact & 16 != 0 {
            modifiers |= 512;
        }
        if compact & 32 != 0 {
            modifiers |= 256;
        }
        Some(Input::Key {
            usage,
            scan,
            modifiers,
            down,
        })
    }
}
fn scan(usage: u16) -> Option<u16> {
    if (4..=29).contains(&usage) {
        return Some(
            [
                0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31,
                0x18, 0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
            ][usize::from(usage - 4)],
        );
    }
    if (30..=39).contains(&usage) {
        return Some(usage - 28);
    }
    if (58..=67).contains(&usage) {
        return Some(usage + 1);
    }
    if (104..=114).contains(&usage) {
        return Some(usage - 4);
    }
    Some(match usage {
        40 => 0x1c,
        41 => 1,
        42 => 0xe,
        43 => 0xf,
        44 => 0x39,
        45 => 0xc,
        46 => 0xd,
        47 => 0x1a,
        48 => 0x1b,
        49 => 0x2b,
        51 => 0x27,
        52 => 0x28,
        53 => 0x29,
        54 => 0x33,
        55 => 0x34,
        56 => 0x35,
        57 => 0x3a,
        68 => 0x57,
        69 => 0x58,
        70 => 0x137,
        71 => 0x46,
        72 => 0x45,
        73 => 0x152,
        74 => 0x147,
        75 => 0x149,
        76 => 0x153,
        77 => 0x14f,
        78 => 0x151,
        79 => 0x14d,
        80 => 0x14b,
        81 => 0x150,
        82 => 0x148,
        83 => 0x145,
        84 => 0x135,
        85 => 0x37,
        86 => 0x4a,
        87 => 0x4e,
        88 => 0x11c,
        89 => 0x4f,
        90 => 0x50,
        91 => 0x51,
        92 => 0x4b,
        93 => 0x4c,
        94 => 0x4d,
        95 => 0x47,
        96 => 0x48,
        97 => 0x49,
        98 => 0x52,
        99 => 0x53,
        100 => 0x56,
        101 => 0x15d,
        103 => 0x59,
        115 => 0x76,
        224 => 0x1d,
        225 => 0x2a,
        226 => 0x38,
        227 => 0x15b,
        228 => 0x11d,
        229 => 0x36,
        230 => 0x138,
        231 => 0x15c,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_layout_modifiers_and_reset() {
        let mut keys = Keyboard::default();
        assert_eq!(
            keys.key(228, 0, true),
            Some(Input::Key {
                usage: 0xe4,
                scan: 0x11d,
                modifiers: 8,
                down: true
            })
        );
        assert_eq!(
            keys.key(4, 0, true),
            Some(Input::Key {
                usage: 0x04,
                scan: 0x1e,
                modifiers: 8,
                down: true
            })
        );
        keys.reset();
        assert_eq!(
            keys.key(4, 0, false),
            Some(Input::Key {
                usage: 0x04,
                scan: 0x1e,
                modifiers: 0,
                down: false
            })
        );
        assert!(keys.key(0, 0, true).is_none());
    }
    #[test]
    fn wire_is_big_endian_and_signed_motion_is_preserved() {
        let frame = Input::Motion {
            x: -5,
            y: 9,
            relative: true,
        }
        .encode(7);
        assert_eq!(
            frame,
            [
                vec![11],
                7u32.to_be_bytes().to_vec(),
                (-5i32).to_be_bytes().to_vec(),
                9u32.to_be_bytes().to_vec(),
                1u32.to_be_bytes().to_vec(),
                vec![0; 4]
            ]
            .concat()
        );
        assert_eq!(Input::Release.encode(7), vec![13]);
    }
}

#[cfg(test)]
#[test]
fn display_drag_keeps_press_move_and_release_on_their_outputs() {
    let mut batch = InputBatch::default();
    batch.target(Some(11));
    batch.push(Input::Button {
        button: 1,
        x: 2,
        y: 4,
        down: true,
    });
    batch.target(Some(29));
    batch.push(Input::Motion {
        x: 300,
        y: 200,
        relative: false,
    });
    batch.push(Input::Button {
        button: 1,
        x: 20,
        y: 10,
        down: false,
    });
    let frames: Vec<_> = batch
        .items
        .iter()
        .map(|(output, command)| command.encode(output.unwrap()))
        .collect();
    assert_eq!((frames[0][0], frames[0][4], frames[0][20]), (10, 11, 1));
    assert_eq!((frames[1][0], frames[1][4]), (11, 29));
    assert_eq!((frames[2][0], frames[2][4], frames[2][20]), (10, 29, 0));
    batch.clear();
    batch.push(Input::Release);
    assert_eq!(batch.items, [(None, Input::Release)]);
}
