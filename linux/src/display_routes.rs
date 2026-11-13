//! Linux compressed-stream routing and bounded lifecycle; no windows or renderer.
use crate::playback::Playback;
use anyhow::{Result, ensure};
use eframe::egui;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
pub struct Displays {
    slots: Mutex<[Option<Arc<Channel>>; crate::display_stream::STREAMS - 1]>,
    pub owner: AtomicUsize,
    pub hevc: std::sync::atomic::AtomicBool,
    release: std::sync::atomic::AtomicBool,
}
pub struct Channel {
    pub playback: Arc<Playback>,
    pub inspector: Arc<Mutex<crate::video_stream::Inspector>>,
    ingress: Mutex<crate::bud_ingress::VideoState>,
    opened: std::time::Instant,
}
pub fn viewport(stream: usize) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("display-stream", stream))
}
impl Displays {
    pub fn flags(&self) -> i32 {
        8 | i32::from(self.hevc.load(Ordering::Acquire))
    }
    pub fn get(&self, stream: usize) -> Option<Arc<Channel>> {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(stream.checked_sub(1)?)?
            .clone()
    }
    pub fn open(self: &Arc<Self>, stream: usize, ctx: &egui::Context) -> Result<()> {
        ensure!(
            (1..crate::display_stream::STREAMS).contains(&stream),
            "Invalid display stream"
        );
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(slots[stream - 1].is_none(), "Display is already open");
        let playback = Playback::new(ctx.clone());
        playback.attach_displays(self, stream);
        playback.set_hevc(self.hevc.load(Ordering::Acquire));
        let mut inspector = crate::video_stream::Inspector::default();
        inspector.configure(crate::video_stream::VideoProtocol {
            version: 1,
            message_size: 11,
            version_offset: 0,
            flag_offset: 4,
        });
        inspector.enable_hevc(playback.hevc());
        slots[stream - 1] = Some(Arc::new(Channel {
            opened: std::time::Instant::now(),
            playback: playback.clone(),
            inspector: Arc::new(Mutex::new(inspector)),
            ingress: Mutex::new(crate::bud_ingress::VideoState::new(
                playback.hevc(),
                false,
                false,
            )),
        }));
        Ok(())
    }
    pub fn close(&self, stream: usize) {
        if let Some(index) = stream.checked_sub(1) {
            let channel = self
                .slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(index)
                .and_then(Option::take);
            if let Some(channel) = channel {
                if self
                    .owner
                    .compare_exchange(stream + 1, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    self.release.store(true, Ordering::Release);
                }
                channel.playback.stop();
                channel.playback.repaint();
            }
        }
    }
    pub fn clear(&self) {
        for stream in 1..crate::display_stream::STREAMS {
            self.close(stream);
        }
    }
    pub fn poll(
        &self,
        primary: &Playback,
        mut send: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        if self.release.swap(false, Ordering::AcqRel) {
            send(&crate::control::header(24, 0, 0, 0))?;
        }
        for stream in 1..crate::display_stream::STREAMS {
            let Some(channel) = self.get(stream) else {
                continue;
            };
            channel.playback.bind_input_consumer();
            channel.playback.poll_gpu();
            if primary.stopped()
                || channel.playback.stopped()
                || (channel.opened.elapsed() >= std::time::Duration::from_secs(20)
                    && channel.playback.video_snapshot().frames_decoded == 0)
            {
                send(&crate::display_stream::enable(stream, false)?)?;
                self.close(stream);
            } else {
                match channel.playback.take_keyframe_request() {
                    Ok(Some(_)) => {
                        channel
                            .ingress
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .wait_for_keyframe();
                        send(&crate::control::header(13, stream as i32, self.flags(), 1))?;
                    }
                    Err(_) => {
                        channel.playback.stop();
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    pub fn bud(&self, source: &mut evoparseq_bud::bud::MediaSource) -> Result<bool> {
        let mut pending = false;
        for stream in 1..crate::display_stream::STREAMS {
            let channel = self.get(stream);
            for index in 0..crate::bud_ingress::PUMP_BUDGET {
                if source
                    .with_secondary_video(stream, |bytes| {
                        if let Some(channel) = &channel {
                            if channel
                                .ingress
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .receive(bytes, channel.playback.as_ref())
                                .is_err()
                            {
                                channel.playback.stop();
                            }
                        }
                        Ok(())
                    })?
                    .is_none()
                {
                    break;
                }
                pending |= index + 1 == crate::bud_ingress::PUMP_BUDGET;
            }
        }
        Ok(pending)
    }
}
impl Drop for Displays {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_bounded_screens_are_independent_and_never_stop_the_primary() {
        let displays = Arc::new(Displays::default());
        let ctx = egui::Context::default();
        let primary = Playback::new(ctx.clone());
        primary.attach_displays(&displays, 0);
        assert!(displays.open(0, &ctx).is_err());
        assert!(displays.open(3, &ctx).is_err());
        displays.open(1, &ctx).unwrap();
        displays.open(2, &ctx).unwrap();
        assert!(displays.open(1, &ctx).is_err());
        let second = displays.get(1).unwrap();
        displays.close(1);
        assert!(second.playback.stopped());
        assert!(!displays.get(2).unwrap().playback.stopped());
        assert!(!primary.stopped());
        displays.clear();
        assert!(displays.get(2).is_none());
    }
}
