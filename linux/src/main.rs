#[path = "../../src-native/src/about.rs"]
mod about;
#[path = "../../src-native/src/native_core/account.rs"]
pub(crate) mod account;
#[path = "../../src-native/src/native_core/signaling.rs"]
mod account_signaling;
#[path = "../../src-native/src/app_style.rs"]
mod app_style;
#[path = "../../src-native/src/attempt.rs"]
pub mod attempt;
mod audio_device;
#[path = "../../src-native/src/audio_layout.rs"]
pub mod audio_layout;
mod audio_pipewire;
#[path = "../../src-native/src/audio_recovery.rs"]
mod audio_recovery;
#[path = "../../src-native/src/audio_stream.rs"]
pub mod audio_stream;
#[path = "../../src-native/src/bud_ingress.rs"]
mod bud_ingress;
#[path = "../../src-native/src/bud_routes.rs"]
mod bud_routes;
#[path = "../../src-native/src/codec_initialization.rs"]
pub mod codec_initialization;
mod connection;
#[path = "../../src-native/src/connection_settings.rs"]
mod connection_settings;
#[path = "../../src-native/src/native_core/display_timing.rs"]
mod display_timing;
#[path = "../../src-native/src/native_core/host_displays.rs"]
mod host_displays;
#[path = "../../src-native/src/native_core/host_video.rs"]
mod host_video;
mod information_linux;
#[path = "../../src-native/src/network_settings.rs"]
mod network_settings;
#[path = "../../src-native/src/native_core/profile.rs"]
pub(crate) mod profile;
#[path = "../../src-native/src/release_flatpak.rs"]
mod release_flatpak;
#[path = "../../src-native/src/release_http.rs"]
mod release_http;
mod resources_linux;
mod secure_storage;
#[path = "../../src-native/src/session_controls.rs"]
mod session_controls;
#[path = "../../src-native/src/session_preferences.rs"]
pub mod session_preferences;
mod settings_linux;
#[path = "../../src-native/src/native_core/sign_in.rs"]
pub(crate) mod sign_in;
#[path = "../../src-native/src/stats.rs"]
mod stats;
#[path = "../../src-native/src/stats_recent.rs"]
mod stats_recent;
#[path = "../../src-native/src/stats_view.rs"]
mod stats_view;
mod telemetry;
#[path = "../../src-native/src/turn_configuration.rs"]
mod turn_configuration;
mod native_core {
    pub(crate) use crate::profile;
}
#[path = "../../src-native/src/clipboard_sync.rs"]
mod clipboard_sync;
#[path = "../../src-native/src/control.rs"]
pub mod control;
mod cursor_theme;
#[path = "../../src-native/src/desktop_input.rs"]
pub mod desktop_input;
mod desktop_linux;
mod desktop_ui;
mod display_routes;
#[path = "../../src-native/src/display_stream.rs"]
mod display_stream;
#[path = "../../src-native/src/event_log.rs"]
mod event_log;
#[path = "../../src-native/src/frame_timing.rs"]
pub mod frame_timing;
#[path = "../../src-native/src/gamepad.rs"]
pub mod gamepad;
mod gamepad_linux;
#[path = "../../src-native/src/hid_policy.rs"]
mod hid_policy;
#[path = "../../src-native/src/input_handoff.rs"]
pub mod input_handoff;
#[path = "../../src-native/src/input_state.rs"]
mod input_state;
mod launcher;
#[path = "../../src-native/src/media_ingress.rs"]
pub mod media_ingress;
mod multi_display;
mod native_loop;
#[path = "../../src-native/src/native_wire.rs"]
pub mod native_wire;
#[cfg(test)]
mod network_tests;
#[path = "../../src-native/src/native_core/picker.rs"]
mod picker;
mod playback;
#[path = "../../src-native/src/rtc_configuration.rs"]
pub mod rtc_configuration;
mod session_overlay;
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
mod wayland_keys;
mod window_mode;

use eframe::{egui, egui_wgpu};
use std::sync::{Arc, mpsc};
use zeroize::Zeroizing;

enum Command {
    Restore,
    SignIn(account::Login, bool),
    Refresh,
    SignOut,
    Stream(
        String,
        connection::Mode,
        Arc<playback::Playback>,
        Arc<telemetry::Session>,
    ),
}

enum Reply {
    Hosts {
        hosts: Vec<account::Host>,
        notice: Option<String>,
    },
    SignedOut,
    StreamEnded(String),
    Error {
        signed_in: bool,
        message: String,
    },
}

struct Desktop {
    commands: mpsc::Sender<Command>,
    replies: mpsc::Receiver<Reply>,
    settings: Arc<connection_settings::Manager>,
    remember_login: bool,
    email: String,
    password: Zeroizing<String>,
    tfa: Zeroizing<String>,
    picker: picker::Picker,
    peer: String,
    transport: connection::Mode,
    logo: egui::TextureHandle,
    desktop_input: desktop_linux::DesktopInput,
    shortcuts: Option<wayland_keys::Shortcuts>,
    signed_in: bool,
    busy: bool,
    status_text: &'static str,
    error: Option<String>,
    show_video: bool,
    ui_state: desktop_ui::State,
    badge: session_overlay::Badge,
    fitted_stream: Option<(u32, u32)>,
    video_context: Option<Arc<video_vulkan::Context>>,
    video_gpus: Vec<connection_settings::GpuPreference>,
    video_error: Option<String>,
    output_format: wgpu::TextureFormat,
    decoder: Option<video_vulkan::Decoder>,
    playback: Option<Arc<playback::Playback>>,
    telemetry: Option<Arc<telemetry::Session>>,
    metrics: telemetry::View,
    screens: [Option<multi_display::Surface>; display_stream::STREAMS - 1],
}

fn account_worker(
    commands: mpsc::Receiver<Command>,
    replies: mpsc::Sender<Reply>,
    ui: egui::Context,
    settings: Arc<connection_settings::Manager>,
) {
    let mut form = sign_in::SignIn::new(None, settings.view().0.native_core.email, false);
    for command in commands {
        let mut notice = None;
        let result = match command {
            Command::Restore => settings.view().0.native_core.session().map(|saved| {
                form.session = saved.map(Arc::new);
            }).map_err(|_| anyhow::anyhow!("Could not unlock the saved sign-in. Sign in again or unlock your desktop keyring.")),
            Command::SignIn(login, remember) => account::login(&login).map(|new_session| {
                form.email = login.email.trim().into();
                form.remember_login = remember;
                form.accept(Arc::new(new_session), settings.as_ref(), |reason| {
                    notice = Some(match reason {
                        sign_in::PersistenceNotice::ProtectedSessionNotSaved => "Signed in, but the desktop keyring could not save the protected session. This sign-in will stay in memory.".into(),
                        _ => reason.text().into(),
                    });
                });
            }),
            Command::Refresh => Ok(()),
            Command::Stream(peer, mode, playback, telemetry) => {
                let _ = settings.update_native(|profile| profile.last_peer = peer.clone());
                let result = form.session
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Sign in first."))
                    .and_then(|session| connection::run_stream(session, &peer, mode, &playback, &telemetry, &settings));
                let reply = match result {
                    Ok(message) => Reply::StreamEnded(message),
                    Err(error) => Reply::Error {
                        signed_in: form.session.is_some(),
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
                // A failed file transaction must not claim that a saved login was removed.
                match form.prepare_logout(settings.as_ref()) {
                    Ok(()) => {
                        if let Some(session) = form.session.take() { session.revoke(); }
                        if replies.send(Reply::SignedOut).is_err() { break; }
                    }
                    Err(error) => {
                        if replies.send(Reply::Error { signed_in: form.session.is_some(), message: error.to_string() }).is_err() { break; }
                    }
                }
                ui.request_repaint();
                continue;
            }
        };
        let result = result.and_then(|()| {
            account::hosts(
                form.session
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Sign in first."))?,
            )
        });
        let reply = match result {
            Ok(hosts) => Reply::Hosts { hosts, notice },
            Err(error) => {
                if error.is::<account::ExpiredSession>() {
                    form.expire_saved(settings.as_ref());
                }
                Reply::Error {
                    signed_in: form.session.is_some(),
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
        self.status_text = match &command {
            Command::Restore | Command::SignIn(..) => app_style::desktop::SIGNING_IN,
            Command::Refresh => app_style::desktop::REFRESHING,
            Command::SignOut => app_style::desktop::SIGNING_OUT,
            Command::Stream(..) => app_style::desktop::CHOOSE,
        };
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
    fn on_exit(&mut self) {
        self.shortcuts.take();
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if raw.viewport_id != egui::ViewportId::ROOT {
            return;
        }
        if let Some(playback) = &self.playback {
            playback.controller_focus(
                raw.focused && !self.show_video && !self.badge.open && !self.ui_state.stats,
            );
        }
        if raw.events.iter().any(|event| matches!(event, egui::Event::PointerButton {pos,pressed:true,..} if pos.y < 24.)) {
            if let Some(playback) = &self.playback {
                if self.desktop_input.enabled { self.desktop_input.release(ctx, playback); }
            }
            return;
        }
        if self.show_video {
            if let Some(playback) = &self.playback {
                if self.desktop_input.enabled {
                    self.desktop_input.release(ctx, playback);
                }
            }
            return;
        }
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
                self.fitted_stream = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));

                playback.stop();
            }
            if let Some(telemetry) = &self.telemetry {
                telemetry.displays.clear();
            }
            self.screens = Default::default();
            self.decoder = None;
            self.telemetry = None;
            self.metrics = Default::default();
            match reply {
                Reply::Hosts { hosts, notice } => {
                    self.error = notice;
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
                    self.status_text = if self.picker.hosts().is_empty() {
                        app_style::desktop::EMPTY
                    } else {
                        app_style::desktop::CHOOSE
                    };
                }
                Reply::SignedOut => {
                    self.remember_login = false;
                    self.picker = Default::default();
                    self.signed_in = false;
                    self.status_text = app_style::desktop::SIGNED_OUT;
                    self.error = None;
                }
                Reply::StreamEnded(message) => {
                    eprintln!("{message}");
                    self.badge.open = false;
                }
                Reply::Error { signed_in, message } => {
                    self.signed_in = signed_in;
                    let message = video_timing
                        .map_or_else(|| message.clone(), |timing| format!("{message}\n{timing}"));
                    eprintln!("{message}");
                    self.error = Some(message);
                    if !signed_in {
                        self.remember_login = false;
                        self.picker = Default::default();
                    }
                }
            }
        }

        window_mode::apply(
            ctx,
            self.playback.is_some() && !self.show_video,
            self.show_video,
        );
        if let (Some(playback), Some(decoder)) = (&self.playback, &mut self.decoder) {
            match multi_display::decode(playback, decoder, ctx.input(|i| i.focused)) {
                Err(error) => {
                    playback.stop();
                    let message = format!(
                        "Vulkan video stopped after {} pictures: {error:#}",
                        decoder.decoded
                    );
                    event_log::record(
                        "ERROR",
                        "video_backend_failed",
                        serde_json::json!({"backend":"Vulkan Video","decoded_pictures":decoder.decoded,
                            "vulkan_result": error.downcast_ref::<ash::vk::Result>().map(|result| result.as_raw())}),
                    );
                    eprintln!("{message}");
                    self.error = Some(message);
                }
                Ok(consumed) if consumed > 0 => ctx.request_repaint(),
                Ok(_) => {}
            }
        }
        if let Some(shortcuts) = &mut self.shortcuts {
            let requested = self.playback.as_ref().is_some_and(|playback| {
                self.desktop_input.enabled
                    && ctx.input(|i| i.focused)
                    && matches!(
                        playback
                            .immersive
                            .load(std::sync::atomic::Ordering::Acquire),
                        1 | 2
                    )
            });
            shortcuts.update(requested);
        }
        if let (Some(telemetry), Some(playback)) = (&self.telemetry, &self.playback) {
            telemetry
                .bus
                .visible
                .store(self.ui_state.stats, std::sync::atomic::Ordering::Relaxed);
            telemetry
                .bus
                .overlay_visible
                .store(self.badge.open, std::sync::atomic::Ordering::Relaxed);
            self.metrics.refresh(
                telemetry,
                playback,
                self.decoder.as_ref(),
                self.ui_state.stats,
                self.desktop_input.refresh_hz(),
            );
        }
        self.menu_bar(ctx);
        self.additional_displays(ctx);
        self.statistics(ctx);
        self.information(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().fill(if self.playback.is_some() && !self.show_video {
                    egui::Color32::BLACK
                } else {
                    launcher::color(app_style::BACKGROUND)
                }),
            )
            .show(ctx, |ui| {
                if self.show_video {
                    self.launcher(ui, ctx);
                } else if let Some(playback) = self.playback.clone() {
                    let area = self.decoder.as_ref().and_then(|decoder| decoder.show(ui));
                    self.desktop_input.video = area.map(|(rect, _)| rect);
                    if let Some((_, source)) = area {
                        playback.set_input_geometry(source);
                        multi_display::fit(ctx, source, &mut self.fitted_stream);
                    }
                    self.desktop_input.cursor(ctx, &playback);
                    self.session_overlay(ctx, &playback);
                } else {
                    self.launcher(ui, ctx);
                }
            });
        window_mode::apply(
            ctx,
            self.playback.is_some() && !self.show_video,
            self.show_video,
        );
        if let Some(playback) = &self.playback {
            playback.finish_clipboard(
                self.desktop_input.enabled
                    && ctx.input(|i| i.focused)
                    && !self.show_video
                    && !self.badge.open
                    && !self.ui_state.stats,
            );
        }
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
    native_loop::report_framework_errors();
    cursor_theme::prepare();
    if std::env::args_os()
        .skip(1)
        .any(|arg| arg == "--probe-video")
    {
        for line in video_probe::report()? {
            println!("{line}");
        }
        return Ok(());
    }

    let settings = settings_linux::open()?;
    let (saved, settings_error) = settings.view();
    event_log::initialize(
        settings.directory().join("client-events.jsonl"),
        saved.event_logging,
    )?;
    evoparseq_socket_qos::observe(|transport, status| {
        let s = status.linux;
        event_log::record(
            if status.applied { "INFO" } else { "WARN" },
            "linux_socket_qos",
            serde_json::json!({"transport": transport, "requested": status.requested,
                "applied": status.applied, "ipv4_dscp": s.ipv4_dscp,
                "ipv6_dscp": s.ipv6_dscp, "socket_priority": s.priority,
                "ipv4_error": s.ipv4_error, "ipv6_error": s.ipv6_error,
                "priority_error": s.priority_error, "os_error": status.error}),
        );
    });
    let _log_guard = event_log::Guard;
    let restore = !saved.native_core.encrypted_session.is_empty();
    let mut setup = egui_wgpu::WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(&setup.instance_descriptor);
    let video_gpus = video_vulkan::adapters(&instance);
    let (wgpu_setup, video_context, video_error) =
        match video_vulkan::create(&instance, saved.video_gpu.as_ref()) {
            Ok((setup, context)) => (egui_wgpu::WgpuSetup::Existing(setup), Some(context), None),
            Err(error) => (setup.into(), None, Some(error.to_string())),
        };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("EvoParseq")
            .with_decorations(true)
            .with_resizable(false)
            .with_maximize_button(false)
            .with_app_id("io.github.vampywiz17.EvoParseq")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!(
                "../../assets/evoparseq.png"
            ))?)
            .with_inner_size([
                app_style::desktop::WINDOW[0] as f32,
                app_style::desktop::WINDOW[1] as f32 + 24.,
            ]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup,
            runtime_frame_latency: video_context
                .as_ref()
                .map(|context| context.frame_latency.clone()),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut worker = None;
    let keyboard = Arc::new(native_loop::KeyboardBridge::default());
    let event_loop =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build()?;
    let result = {
        let app =
            eframe::create_native(
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
                    let logo = cc.egui_ctx.load_texture(
                        "EvoParseq logo",
                        image,
                        egui::TextureOptions::LINEAR,
                    );

                    let (commands, incoming) = mpsc::channel();
                    let (outgoing, replies) = mpsc::channel();
                    let context = cc.egui_ctx.clone();
                    let worker_settings = settings.clone();
                    worker = Some(std::thread::Builder::new().name("account".into()).spawn(
                        move || account_worker(incoming, outgoing, context, worker_settings),
                    )?);
                    if restore {
                        commands.send(Command::Restore)?;
                    }
                    let output_format = cc
                        .wgpu_render_state
                        .as_ref()
                        .map(|state| state.target_format)
                        .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
                    Ok(Box::new(Desktop {
                        video_context,
                        video_gpus,
                        video_error,
                        output_format,
                        decoder: None,
                        playback: None,
                        telemetry: None,
                        metrics: Default::default(),
                        screens: Default::default(),
                        commands,
                        replies,
                        settings: settings.clone(),
                        remember_login: restore,
                        email: saved.native_core.email.clone(),
                        password: Zeroizing::default(),
                        tfa: Zeroizing::default(),
                        picker: Default::default(),
                        peer: saved.native_core.last_peer.clone(),
                        transport: if saved.bud {
                            connection::Mode::Bud
                        } else {
                            connection::Mode::WebRtc
                        },
                        logo,
                        desktop_input: desktop_linux::DesktopInput::new(keyboard.clone()),
                        shortcuts: wayland_keys::Shortcuts::new(cc).ok(),
                        signed_in: false,
                        busy: restore,
                        status_text: if restore {
                            app_style::desktop::SIGNING_IN
                        } else {
                            app_style::LOGIN_NOTICE
                        },
                        error: settings_error.clone(),
                        show_video: false,
                        ui_state: desktop_ui::State {
                            prefer_hevc: saved.prefer_hevc,
                            ..Default::default()
                        },
                        badge: Default::default(),
                        fitted_stream: None,
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
