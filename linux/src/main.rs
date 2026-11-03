#[path = "../../src-native/src/native_core/account.rs"]
mod account;
#[path = "../../src-native/src/event_log.rs"]
mod event_log;
mod video_probe;

use eframe::{egui, egui_wgpu};
use std::sync::mpsc;
use zeroize::Zeroizing;

enum Command {
    SignIn(account::Login),
    Refresh,
    SignOut,
}

enum Reply {
    Hosts(Vec<account::Host>),
    SignedOut,
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
                        for host in &self.hosts {
                            ui.label(&host.name);
                        }
                        if self.hosts.is_empty() {
                            ui.label("No computers listed.");
                        }
                        ui.label("Streaming is not available in this preview.");
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
