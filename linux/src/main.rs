#[path = "../../src-native/src/native_core/account.rs"]
mod account;
#[path = "../../src-native/src/native_core/signaling.rs"]
mod account_signaling;
#[path = "../../src-native/src/app_style.rs"]
mod app_style;
#[path = "../../src-native/src/attempt.rs"]
pub mod attempt;
#[path = "../../src-native/src/audio_layout.rs"]
pub mod audio_layout;
mod audio_pipewire;
#[path = "../../src-native/src/audio_stream.rs"]
pub mod audio_stream;
#[path = "../../src-native/src/bud_ingress.rs"]
mod bud_ingress;
#[path = "../../src-native/src/codec_initialization.rs"]
pub mod codec_initialization;
mod connection;
#[path = "../../src-native/src/control.rs"]
pub mod control;
#[path = "../../src-native/src/desktop_input.rs"]
pub mod desktop_input;
mod desktop_linux;
#[path = "../../src-native/src/event_log.rs"]
mod event_log;
#[path = "../../src-native/src/frame_timing.rs"]
pub mod frame_timing;
#[path = "../../src-native/src/gamepad.rs"]
pub mod gamepad;
#[path = "../../src-native/src/input_handoff.rs"]
pub mod input_handoff;
mod launcher;
#[path = "../../src-native/src/media_ingress.rs"]
pub mod media_ingress;
mod native_loop;
#[path = "../../src-native/src/native_wire.rs"]
pub mod native_wire;
#[path = "../../src-native/src/native_core/picker.rs"]
mod picker;
mod playback;
#[path = "../../src-native/src/rtc_configuration.rs"]
pub mod rtc_configuration;
#[path = "../../src-native/src/signaling.rs"]
pub mod signaling;
#[path = "../../src-native/src/transport.rs"]
pub mod transport;
#[path = "../../src-native/src/video_output.rs"]
pub mod video_output;
mod video_probe;
mod video_profile;
#[path = "../../src-native/src/video_stream.rs"]
pub mod video_stream;
mod video_vulkan;
#[path = "../../src-native/src/viewport.rs"]
pub mod viewport;

use eframe::{egui, egui_wgpu};
use std::sync::{Arc, mpsc};
use zeroize::Zeroizing;

enum Command {
    SignIn(account::Login),
    Refresh,
    SignOut,
    Stream(String, connection::Mode, Arc<playback::Playback>),
}

enum Reply {
    Hosts(Vec<account::Host>),
    SignedOut,
    StreamEnded(String),
    Error { signed_in: bool, message: String },
}

struct Desktop {
    commands: mpsc::Sender<Command>,
    replies: mpsc::Receiver<Reply>,
    email: String,
    password: Zeroizing<String>,
    tfa: Zeroizing<String>,
    picker: picker::Picker,
    peer: String,
    transport: connection::Mode,
    logo: egui::TextureHandle,
    desktop_input: desktop_linux::DesktopInput,
    signed_in: bool,
    busy: bool,
    error: Option<String>,
    session_result: Option<String>,
    show_video: bool,
    video: Vec<String>,
    renderer: String,
    video_context: Option<Arc<video_vulkan::Context>>,
    video_error: Option<String>,
    output_format: wgpu::TextureFormat,
    decoder: Option<video_vulkan::Decoder>,
    playback: Option<Arc<playback::Playback>>,
}

fn account_worker(
    commands: mpsc::Receiver<Command>,
    replies: mpsc::Sender<Reply>,
    ui: egui::Context,
) {
    let mut session = None;
    for command in commands {
        let result = match command {
            Command::SignIn(login) => account::login(&login).map(|new_session| {
                session = Some(new_session);
            }),
            Command::Refresh => Ok(()),
            Command::Stream(peer, mode, playback) => {
                let result = session
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Sign in first."))
                    .and_then(|session| connection::run_stream(session, &peer, mode, &playback));
                let reply = match result {
                    Ok(message) => Reply::StreamEnded(message),
                    Err(error) => Reply::Error {
                        signed_in: session.is_some(),
                        message: error.to_string(),
                    },
                };
                if replies.send(reply).is_err() {
                    break;
                }
                ui.request_repaint();
                continue;
            }
            Command::SignOut => {
                session = None;
                if replies.send(Reply::SignedOut).is_err() {
                    break;
                }
                ui.request_repaint();
                continue;
            }
        };
        let result = result.and_then(|()| {
            account::hosts(
                session
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Sign in first."))?,
            )
        });
        let reply = match result {
            Ok(hosts) => Reply::Hosts(hosts),
            Err(error) => {
                if error.is::<account::ExpiredSession>() {
                    session = None;
                }
                Reply::Error {
                    signed_in: session.is_some(),
                    message: error.to_string(),
                }
            }
        };
        if replies.send(reply).is_err() {
            break;
        }
        ui.request_repaint();
    }
}

impl Desktop {
    fn send(&mut self, command: Command) {
        match self.commands.send(command) {
            Ok(()) => {
                self.busy = true;
                self.error = None;
                self.session_result = None;
            }
            Err(_) => self.error = Some("Account worker is unavailable.".into()),
        }
    }
}

impl eframe::App for Desktop {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(playback) = &self.playback {
            self.desktop_input.events(ctx, raw, playback);
        }
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        while let Ok(reply) = self.replies.try_recv() {
            let video_timing = self
                .decoder
                .as_ref()
                .map(|decoder| decoder.timing_summary());
            self.busy = false;
            if let Some(playback) = self.playback.take() {
                self.desktop_input.release(ctx, &playback);
                self.desktop_input.video = None;
                playback.stop();
            }
            self.decoder = None;
            match reply {
                Reply::Hosts(hosts) => {
                    let selected = self
                        .picker
                        .hosts()
                        .iter()
                        .position(|host| host.peer == self.peer);
                    let peer = self.peer.clone();
                    if let Some(change) =
                        self.picker
                            .replace_hosts(hosts, picker::Refresh::Manual, || {
                                picker::ListPosition {
                                    selected,
                                    last_peer: peer,
                                    top: 0,
                                }
                            })
                    {
                        if let Some(peer) = change.peer {
                            self.peer = peer;
                        }
                    }
                    self.signed_in = true;
                }
                Reply::SignedOut => {
                    self.picker = Default::default();
                    self.signed_in = false;
                    self.error = None;
                }
                Reply::StreamEnded(message) => self.session_result = Some(message),
                Reply::Error { signed_in, message } => {
                    self.signed_in = signed_in;
                    let message = video_timing
                        .map_or_else(|| message.clone(), |timing| format!("{message}\n{timing}"));
                    eprintln!("{message}");
                    self.error = Some(message);
                    if !signed_in {
                        self.picker = Default::default();
                    }
                }
            }
        }

        if let (Some(playback), Some(decoder)) = (&self.playback, &mut self.decoder) {
            match playback
                .resume_if_ready(ctx.input(|input| input.focused))
                .and_then(|_| playback.decode_pending(|bytes| decoder.submit(bytes)))
            {
                Err(error) => {
                    playback.stop();
                    let message = format!(
                        "Vulkan video stopped after {} pictures: {error:#}",
                        decoder.decoded
                    );
                    eprintln!("{message}");
                    self.error = Some(message);
                }
                Ok(consumed) if consumed > 0 => ctx.request_repaint(),
                Ok(_) => {}
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(if self.playback.is_some() { egui::Color32::BLACK } else { launcher::color(app_style::BACKGROUND) })).show(ctx, |ui| {
            if let Some(playback) = self.playback.clone() {
                let area = self.decoder.as_ref().and_then(|decoder| decoder.show(ui));
                self.desktop_input.video = area.map(|(rect,_)| rect);
                if let Some((_,source)) = area { playback.set_input_geometry(source); }
                self.desktop_input.cursor(ctx, &playback);
                self.desktop_input.controls = None;
                if !self.desktop_input.enabled || self.error.is_some() {
                    let area = egui::Area::new(egui::Id::new("stream-controls")).fixed_pos(egui::pos2(16.,16.)).order(egui::Order::Foreground).show(ctx, |ui| {
                        egui::Frame::new().fill(launcher::color(app_style::BACKGROUND)).inner_margin(16.).corner_radius(6).show(ui, |ui| {
                            ui.set_max_width(480.);
                            ui.horizontal(|ui| {
                                if ui.add_enabled(playback.input_ready() && self.desktop_input.video.is_some(),egui::Button::new("Enable input")).clicked() { self.desktop_input.enable(&playback); }
                                if ui.button("Disconnect").clicked() { self.desktop_input.release(ctx,&playback); playback.stop(); }
                                if ui.button("Full screen").clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)))); }
                            });
                            ui.label(format!("{} / H.264 / Vulkan Video",self.transport.label()));
                            ui.small("Click the video to control. Ctrl+Shift+M releases input and opens controls.");
                            if let Some(status) = playback.status() { ui.label(status); }
                            if let Some(error) = &self.error { ui.colored_label(egui::Color32::LIGHT_RED,error); }
                        });
                    });
                    self.desktop_input.controls = Some(area.response.rect);
                }
            } else { self.launcher(ui,ctx); }
        });
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(playback) = &self.playback {
            playback.stop();
        }
    }
}

fn main() -> anyhow::Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--probe-video")
    {
        for line in video_probe::report()? {
            println!("{line}");
        }
        return Ok(());
    }

    let video =
        video_probe::report().unwrap_or_else(|error| vec![format!("Probe failed: {error}")]);
    let mut setup = egui_wgpu::WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    let (wgpu_setup, video_context, video_error) = match video_vulkan::create() {
        Ok((setup, context)) => (egui_wgpu::WgpuSetup::Existing(setup), Some(context), None),
        Err(error) => (setup.into(), None, Some(error.to_string())),
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("EvoParseq")
            .with_decorations(true)
            .with_app_id("io.github.vampywiz17.EvoParseq")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!(
                "../../assets/evoparseq.png"
            ))?)
            .with_min_inner_size([720.0, 540.0])
            .with_inner_size([780.0, 600.0]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut worker = None;
    let keyboard = Arc::new(native_loop::KeyboardBridge::default());
    let event_loop =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build()?;
    let result = {
        let app = eframe::create_native(
            "io.github.vampywiz17.EvoParseq",
            options,
            Box::new(|cc| {
                launcher::style(&cc.egui_ctx);
                let icon = eframe::icon_data::from_png_bytes(include_bytes!(
                    "../../assets/evoparseq.png"
                ))?;
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [icon.width as usize, icon.height as usize],
                    &icon.rgba,
                );
                let logo =
                    cc.egui_ctx
                        .load_texture("EvoParseq logo", image, egui::TextureOptions::LINEAR);

                let (commands, incoming) = mpsc::channel();
                let (outgoing, replies) = mpsc::channel();
                let context = cc.egui_ctx.clone();
                worker = Some(
                    std::thread::Builder::new()
                        .name("account".into())
                        .spawn(move || account_worker(incoming, outgoing, context))?,
                );
                let renderer = cc
                    .wgpu_render_state
                    .as_ref()
                    .map(|state| state.adapter.get_info().name)
                    .unwrap_or_else(|| "Unknown".into());
                let output_format = cc
                    .wgpu_render_state
                    .as_ref()
                    .map(|state| state.target_format)
                    .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
                Ok(Box::new(Desktop {
                    video_context,
                    video_error,
                    output_format,
                    decoder: None,
                    playback: None,
                    commands,
                    replies,
                    renderer,
                    video,
                    email: String::new(),
                    password: Zeroizing::default(),
                    tfa: Zeroizing::default(),
                    picker: Default::default(),
                    peer: String::new(),
                    transport: connection::Mode::WebRtc,
                    logo,
                    desktop_input: desktop_linux::DesktopInput::new(keyboard.clone()),
                    signed_in: false,
                    busy: false,
                    error: None,
                    session_result: None,
                    show_video: false,
                }))
            }),
            &event_loop,
        );
        let mut app = native_loop::Application::new(app, keyboard.clone());
        let result = event_loop.run_app(&mut app);
        drop(app);
        result
    };
    // The window has closed, so revoking the short-lived session cannot block the UI.
    if let Some(worker) = worker {
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("Account worker stopped unexpectedly"))?;
    }
    result.map_err(|error| anyhow::anyhow!("{error}"))
}
