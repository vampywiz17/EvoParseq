//! Adapter shared by the Wayland and Android session loops.
use crate::{
    audio_stream::DecodedPcm,
    desktop_input::{Action, Engine, Event},
    phaze_session::{self, Stream},
    video_stream::PacketInfo,
};
use anyhow::Result;
use evoparseq_phaze::{AudioCodec, ClientControl, Input, Keyboard, Output, Sink, VideoConfig};
use std::time::{Duration, Instant};

pub trait Platform {
    fn stage(&mut self, _stage: &'static str) {}
    fn outputs(&mut self, _outputs: &[Output]) {}
    fn stream_request_sent(&mut self, _output: u32, _config: VideoConfig) {}
    fn video(&mut self, encoded: &[u8], info: PacketInfo) -> Result<()>;
    fn reset_video(&mut self);
    fn take_recovery(&mut self) -> bool;
    fn poll(
        &mut self,
        session: &mut State,
        output: Option<u32>,
        input: &mut Vec<Input>,
        controls: &mut Vec<ClientControl>,
    ) -> Result<bool>;
    fn cursor_visibility(&mut self, _visible: bool) {}
    fn clipboard(&mut self, _text: &str) {}
    fn control_sent(&mut self, _control: &ClientControl, _sent: bool) {}
    fn peer_state(&mut self, _local_id: u32, _state: evoparseq_phaze::PeerState) {}
}

pub struct State {
    pub stream: Stream,
    pub received: u64,
    pub sent: u64,
    pub rtt: Option<Duration>,
    pub audio_bytes: u64,
    pub route: &'static str,
    pub carrier: &'static str,
}

pub struct Client<P> {
    pub platform: P,
    state: State,
    pcm: DecodedPcm,
    raw_audio: bool,
}

impl<P: Platform> Client<P> {
    pub fn new(platform: P, config: VideoConfig, pcm: DecodedPcm, raw_audio: bool) -> Self {
        Self {
            platform,
            pcm,
            raw_audio,
            state: State {
                stream: Stream::new(config),
                received: 0,
                sent: 0,
                rtt: None,
                audio_bytes: 0,
                route: "Direct",
                carrier: "direct UDP",
            },
        }
    }

    pub async fn connect(
        &mut self,
        token: &str,
        peer: &str,
        servers: Option<Vec<webrtc::ice_transport::ice_server::RTCIceServer>>,
        roots: impl Fn() -> Vec<Vec<u8>>,
    ) -> Result<()> {
        let remote = evoparseq_phaze::Session::from_token(token)?;
        if let Some(servers) = servers {
            phaze_session::connect_relay(
                &remote,
                peer,
                self,
                servers,
                roots,
                |client, name, allocated| {
                    client.state.carrier = name;
                    if allocated {
                        client.state.route = "TURN";
                    }
                },
            )
            .await
        } else {
            evoparseq_phaze::connect(&remote, peer, self).await
        }
    }
}

impl<P: Platform> Sink for Client<P> {
    fn raw_audio(&self) -> bool {
        self.raw_audio
    }
    fn video_config(&self) -> VideoConfig {
        self.state.stream.config()
    }
    fn stream_request(&mut self) -> Option<(u32, VideoConfig)> {
        self.state.stream.take_request()
    }
    fn take_video_recovery(&mut self) -> bool {
        self.platform.take_recovery()
    }
    fn stream_request_sent(&mut self, output: u32, config: VideoConfig) {
        self.state.stream.request_sent(output, config);
        self.platform.stream_request_sent(output, config);
    }
    fn bandwidth_feedback_started(&mut self, lower_rate: f64, upper_rate: f64) {
        phaze_session::log_bandwidth_feedback(lower_rate, upper_rate);
    }
    fn stage(&mut self, stage: &'static str) {
        self.platform.stage(stage);
    }
    fn outputs(&mut self, outputs: &[Output]) {
        self.platform.outputs(outputs);
    }
    fn video(&mut self, encoded: &[u8], reset: bool) -> Result<()> {
        if reset {
            self.platform.reset_video();
        }
        if let Some(info) = self.state.stream.inspect(encoded, reset) {
            self.platform.video(encoded, info)?;
        }
        Ok(())
    }
    fn audio(&mut self, pcm: &[i16], codec: AudioCodec, payload_bytes: usize) -> Result<()> {
        self.state.audio_bytes = self.state.audio_bytes.saturating_add(payload_bytes as u64);
        self.pcm.submit(pcm, codec.label());
        Ok(())
    }
    fn traffic(&mut self, received: u64, sent: u64) {
        self.state.received = received;
        self.state.sent = sent;
    }
    fn rtt(&mut self, elapsed: Option<Duration>) {
        self.state.rtt = elapsed;
    }
    fn poll(
        &mut self,
        output: Option<u32>,
        input: &mut Vec<Input>,
        controls: &mut Vec<ClientControl>,
    ) -> Result<bool> {
        self.platform.poll(&mut self.state, output, input, controls)
    }
    fn cursor_visibility(&mut self, visible: bool) {
        self.platform.cursor_visibility(visible);
    }
    fn cursor_output_changed(&mut self) {
        self.platform.cursor_visibility(true);
    }
    fn clipboard_text(&mut self, text: &str) {
        self.platform.clipboard(text);
    }
    fn control_sent(&mut self, control: &ClientControl, sent: bool) {
        self.platform.control_sent(control, sent);
    }
    fn peer_state(&mut self, local_id: u32, state: evoparseq_phaze::PeerState) {
        self.platform.peer_state(local_id, state);
    }
}

#[derive(Default)]
pub struct DesktopInput {
    pub engine: Engine,
    keys: Keyboard,
}

impl DesktopInput {
    pub fn event(&mut self, event: Event, size: (u32, u32), input: &mut Vec<Input>) -> Result<()> {
        self.engine.geometry(size);
        let keys = &mut self.keys;
        self.engine.event_action(event, Instant::now(), |action| {
            translate(action, keys, size, input);
            Ok(())
        })
    }

    #[cfg(target_os = "android")]
    pub fn tick(&mut self, size: (u32, u32), input: &mut Vec<Input>) -> Result<()> {
        let keys = &mut self.keys;
        self.engine.tick_action(Instant::now(), |action| {
            translate(action, keys, size, input);
            Ok(())
        })
    }
}

fn translate(action: Action, keys: &mut Keyboard, size: (u32, u32), commands: &mut Vec<Input>) {
    let normal = |value: i32, extent: u32| {
        (i64::from(value) * 65535 / i64::from(extent.saturating_sub(1).max(1))).clamp(0, 65535)
            as i32
    };
    match action {
        Action::Key(usage, mods, down) => commands.extend(keys.key(usage as u16, mods, down)),
        Action::Motion(x, y, true) => commands.push(Input::Motion {
            x,
            y,
            relative: true,
        }),
        Action::Motion(x, y, false) => commands.push(Input::Motion {
            x: normal(x, size.0),
            y: normal(y, size.1),
            relative: false,
        }),
        Action::Button(button, down, x, y, relative) => {
            if down && !relative {
                let position = Input::Motion {
                    x: normal(x, size.0),
                    y: normal(y, size.1),
                    relative: false,
                };
                if commands.last() != Some(&position) {
                    commands.push(position);
                }
            }
            commands.push(Input::Button {
                button: button as u8,
                down,
                x,
                y,
            });
        }
        Action::Scroll(x, y) => commands.push(Input::Scroll {
            x,
            y: y.saturating_neg(),
        }),
        Action::Release => {
            keys.reset();
            commands.push(Input::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_phaze_preserves_physical_mouse_buttons_and_positions_click_first() -> Result<()> {
        let mut desktop = DesktopInput::default();
        let mut commands = Vec::new();
        let size = (101, 101);
        desktop.event(Event::Focus(true), size, &mut commands)?;
        for (mask, button) in [(2, 2), (4, 3)] {
            commands.clear();
            let pointer = crate::desktop_input::Pointer {
                x: 50.,
                y: 25.,
                relative: false,
                moved: true,
                viewport: size,
                buttons: 0,
                wheel: (0, 0),
            };
            desktop.event(Event::Pointer(pointer), size, &mut commands)?;
            commands.clear();
            desktop.event(
                Event::Pointer(crate::desktop_input::Pointer {
                    buttons: mask,
                    moved: false,
                    ..pointer
                }),
                size,
                &mut commands,
            )?;
            desktop.event(
                Event::Pointer(crate::desktop_input::Pointer {
                    moved: false,
                    ..pointer
                }),
                size,
                &mut commands,
            )?;
            assert_eq!(
                commands,
                [
                    Input::Motion {
                        x: 32767,
                        y: 16383,
                        relative: false
                    },
                    Input::Button {
                        button,
                        x: 50,
                        y: 25,
                        down: true
                    },
                    Input::Button {
                        button,
                        x: 50,
                        y: 25,
                        down: false
                    },
                ]
            );
        }
        Ok(())
    }
}
