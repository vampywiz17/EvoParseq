//! OS-free native HID normalization and verification gate. Only an explicitly
//! selected BUD transport may enable this policy; release is always permitted.
pub fn absolute_motion(
    frame: &[u8],
    viewport: Option<crate::viewport::Viewport>,
) -> anyhow::Result<Option<bytes::Bytes>> {
    if frame.len() != 13 || frame[12] != 3 || frame[..4] != [0; 4] {
        return Ok(None);
    }
    let viewport = viewport.ok_or(crate::control::AbsoluteMouseUnavailable)?;
    anyhow::ensure!(
        viewport.rect().is_some(),
        "absolute HID requires a valid video viewport"
    );
    let axis = |offset: usize, size: u32| {
        let point = i32::from_be_bytes(frame[offset..offset + 4].try_into().unwrap());
        let span = i64::from(size.saturating_sub(1));
        if span == 0 {
            0
        } else {
            ((i64::from(point).clamp(0, span) * 32767 + span / 2) / span) as i32
        }
    };
    Ok(Some(crate::control::header(
        3,
        2,
        axis(4, viewport.source.0),
        axis(8, viewport.source.1),
    )))
}

#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub struct Router {
    pub mouse: bool,
    pub keyboard: bool,
    pub(crate) relative: bool,
}
#[cfg(any(target_os = "linux", test))]
impl Router {
    pub fn send(
        &mut self,
        packet: &[u8],
        source: (u32, u32),
        primary: bool,
        active: [(bool, u32); 2],
        mut send: impl FnMut(&[u8]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if packet.len() < 13 {
            return send(packet);
        }
        let kind = packet[12];
        if kind == 24 {
            self.relative = false;
            return send(packet);
        }
        if kind == 0 && self.keyboard && !active[1].0 {
            return Ok(());
        }
        if self.mouse && matches!(kind, 1 | 2 | 3) {
            if !primary || !active[0].0 {
                return Ok(());
            }
            if kind == 3 {
                let motion = i32::from_be_bytes(packet[..4].try_into().unwrap());
                if motion == 0 || motion == 1 {
                    let relative = motion == 1;
                    if self.relative != relative {
                        send(&crate::control::header(3, 3, i32::from(relative), 0))?;
                        self.relative = relative;
                    }
                }
                if let Some(absolute) = absolute_motion(
                    packet,
                    Some(crate::viewport::Viewport {
                        source,
                        client: source,
                    }),
                )? {
                    return send(&absolute);
                }
            }
        }
        send(packet)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hid_primary_and_driver_are_required_but_release_is_never_blocked() {
        let mut router = Router {
            mouse: true,
            keyboard: true,
            ..Default::default()
        };
        let mut frames = Vec::new();
        let mouse = crate::control::header(3, 0, 1919, 1079);
        router
            .send(&mouse, (1920, 1080), false, [(true, 0); 2], |f| {
                frames.push(f.to_vec());
                Ok(())
            })
            .unwrap();
        assert!(frames.is_empty());
        router
            .send(&mouse, (1920, 1080), true, [(true, 0); 2], |f| {
                frames.push(f.to_vec());
                Ok(())
            })
            .unwrap();
        assert_eq!(frames[0], crate::control::header(3, 2, 32767, 32767));
        router
            .send(
                &crate::control::header(24, 0, 0, 0),
                (0, 0),
                false,
                [(false, 1); 2],
                |f| {
                    frames.push(f.to_vec());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(frames.last().unwrap()[12], 24);
    }
}
