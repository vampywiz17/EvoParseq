#[path = "../../src-native/src/native_core/account.rs"]
mod account;
#[path = "../../src-native/src/native_core/signaling.rs"]
mod account_signaling;
#[path = "../../src-native/src/attempt.rs"]
pub mod attempt;
#[path = "../../src-native/src/audio_layout.rs"]
pub mod audio_layout;
mod audio_pipewire;
#[path = "../../src-native/src/audio_stream.rs"]
pub mod audio_stream;
#[path = "../../src-native/src/codec_initialization.rs"]
pub mod codec_initialization;
mod connection_probe;
#[path = "../../src-native/src/control.rs"]
pub mod control;
#[path = "../../src-native/src/event_log.rs"]
mod event_log;
#[path = "../../src-native/src/frame_timing.rs"]
pub mod frame_timing;
#[path = "../../src-native/src/media_ingress.rs"]
pub mod media_ingress;
#[path = "../../src-native/src/native_wire.rs"]
pub mod native_wire;
#[path = "../../src-native/src/rtc_configuration.rs"]
pub mod rtc_configuration;
#[path = "../../src-native/src/signaling.rs"]
pub mod signaling;
#[path = "../../src-native/src/transport.rs"]
pub mod transport;
#[path = "../../src-native/src/video_output.rs"]
pub mod video_output;
mod video_probe;
#[path = "../../src-native/src/video_stream.rs"]
pub mod video_stream;

use eframe::{egui, egui_wgpu};
use std::sync::mpsc;
use zeroize::Zeroizing;

enum Command {
    SignIn(account::Login),
    Refresh,
    SignOut,
    Probe(String, connection_probe::Mode),
}

enum Reply {
    Hosts(Vec<account::Host>),
    SignedOut,
    ProbePassed(String),
    Error { signed_in: bool, message: String },
}

struct Desktop {
    commands: mpsc::Sender<Command>,
    replies: mpsc::Receiver<Reply>,
    email: String,
    password: Zeroizing<String>,
    tfa: Zeroizing<String>,
    hosts: Vec<account::Host>,
    signed_in: bool,
    busy: bool,
    error: Option<String>,
    probe_result: Option<String>,
    show_video: bool,
    video: Vec<String>,
    renderer: String,
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
            Command::Probe(peer, mode) => {
                let reply = match session
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Sign in first."))
                    .and_then(|session| connection_probe::run(session, &peer, mode))
                {
                    Ok(message) => Reply::ProbePassed(message),
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
                self.probe_result = None;
            }
            Err(_) => self.error = Some("Account worker is unavailable.".into()),
        }
    }
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        while let Ok(reply) = self.replies.try_recv() {
            self.busy = false;
            match reply {
                Reply::Hosts(hosts) => {
                    self.hosts = hosts;
                    self.signed_in = true;
                }
                Reply::SignedOut => {
                    self.hosts.clear();
                    self.signed_in = false;
                    self.error = None;
                }
                Reply::ProbePassed(message) => self.probe_result = Some(message),
                Reply::Error { signed_in, message } => {
                    self.signed_in = signed_in;
                    self.error = Some(message);
                    if !signed_in {
                        self.hosts.clear();
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("EvoParseq");
                ui.label("Linux preview");
                if ui
                    .button(if self.show_video {
                        "Back"
                    } else {
                        "Video capabilities"
                    })
                    .clicked()
                {
                    self.show_video = !self.show_video;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.show_video {
                    ui.heading("Vulkan Video");
                    ui.label(format!("UI adapter: {}", self.renderer));
                    ui.label("Driver-reported capabilities · playback validation pending");
                    for line in &self.video {
                        ui.label(line);
                    }
                    return;
                }
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.add_enabled_ui(!self.busy, |ui| {
                    if self.signed_in {
                        ui.heading("Computers");
                        ui.horizontal(|ui| {
                            if ui.button("Refresh").clicked() {
                                self.send(Command::Refresh);
                            }
                            if ui.button("Sign out").clicked() {
                                self.send(Command::SignOut);
                            }
                        });
                        let mut probe = None;
                        for host in &self.hosts {
                            ui.horizontal(|ui| {
                                ui.label(&host.name);
                                for mode in [connection_probe::Mode::WebRtc, connection_probe::Mode::Bud] {
                                    if ui.button(format!("Test {}", mode.label())).clicked() {
                                        probe = Some((host.peer.clone(), mode));
                                    }
                                }
                            });
                        }
                        if let Some((peer, mode)) = probe {
                            self.send(Command::Probe(peer, mode));
                        }
                        if let Some(result) = &self.probe_result {
                            ui.label(result);
                        }
                        if self.hosts.is_empty() {
                            ui.label("No computers listed.");
                        }
                        ui.label("Play audio on the host during the test. Video playback is not available in this preview.");
                    } else {
                        ui.heading("Sign in to Parsec");
                        ui.label("Email");
                        ui.text_edit_singleline(&mut self.email);
                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(&mut *self.password).password(true));
                        ui.label("Two-factor code (optional)");
                        ui.add(egui::TextEdit::singleline(&mut *self.tfa).password(true));
                        if ui.button("Sign in").clicked() {
                            let login = account::Login {
                                email: self.email.clone(),
                                password: std::mem::take(&mut self.password),
                                tfa: std::mem::take(&mut self.tfa),
                            };
                            self.send(Command::SignIn(login));
                        }
                    }
                });
                if self.busy {
                    ui.spinner();
                }
            });
        });
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
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("EvoParseq")
            .with_inner_size([780.0, 500.0]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: setup.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut worker = None;
    let result = eframe::run_native(
        "io.github.vampywiz17.EvoParseq",
        options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = egui::Color32::from_rgb(25, 29, 35);
            visuals.selection.bg_fill = egui::Color32::from_rgb(25, 100, 170);
            cc.egui_ctx.set_visuals(visuals);

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
            Ok(Box::new(Desktop {
                commands,
                replies,
                renderer,
                video,
                email: String::new(),
                password: Zeroizing::default(),
                tfa: Zeroizing::default(),
                hosts: Vec::new(),
                signed_in: false,
                busy: false,
                error: None,
                probe_result: None,
                show_video: false,
            }))
        }),
    );
    // The window has closed, so revoking the short-lived session cannot block the UI.
    if let Some(worker) = worker {
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("Account worker stopped unexpectedly"))?;
    }
    result.map_err(|error| anyhow::anyhow!("{error}"))
}
