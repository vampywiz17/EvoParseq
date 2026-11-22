//! EvoParseq native account and session entry point.
//! The production media, carrier, route retry and HID workers are reused.
mod account;
mod controls;
mod display_timing;
mod display_windows;
pub(crate) mod host_displays;
mod host_video;
mod input;
pub(crate) mod multi_display;
mod phaze;
mod picker;
mod pointer;
pub(crate) mod profile;
mod session;
mod session_badge;
pub(crate) mod session_model;
mod session_platform;
pub(crate) mod session_ui;
mod session_video;
mod sign_in;
mod signaling;
mod transport;
pub(crate) mod ui;

use anyhow::{Context, Result};

pub fn main() {
    if let Err(error) = run() {
        // Every error crossing this entry is an application-authored message;
        // raw HTTP/WebSocket errors must never display their credential URL.
        unsafe {
            ui::message_box(std::ptr::null_mut(), &error.to_string());
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!(
            "EvoParseq {} / native Rust core (BUD/WebRTC)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    let root = if args.is_empty() {
        std::env::current_exe()?
            .parent()
            .context("Application folder unavailable")?
            .to_path_buf()
    } else if args.len() == 2 && args[0] == "--data-dir" {
        std::path::PathBuf::from(&args[1])
    } else {
        anyhow::bail!("Unsupported native core argument");
    };
    let settings = crate::connection_settings::Manager::open(root.join("settings.json"));
    crate::event_log::initialize(
        root.join("client-events.jsonl"),
        settings.view().0.event_logging,
    )?;
    let _log = crate::event_log::Guard;
    evoparseq_socket_qos::observe(|transport, status| {
        crate::event_log::record(
            if status.applied { "INFO" } else { "WARN" },
            "turn_tcp_qos",
            serde_json::json!({"transport":transport,"requested":status.requested,"applied":status.applied,
                "configured_dscp":status.configured_dscp,"configured_l2":status.configured_l2,"os_error":status.error}),
        );
    });
    crate::event_log::record(
        "INFO",
        "native_core_started",
        serde_json::json!({"transport":if settings.view().0.bud {"BUD"} else {"WebRTC"}}),
    );
    let mut session = None;
    loop {
        let Some(selection) = ui::choose(settings.clone(), session.take())? else {
            return Ok(());
        };
        let preferences = settings.view().0;
        let requested_main10 = preferences.prefer_hevc && preferences.experimental_ten_bit;
        let policy = transport::Policy {
            bud: preferences.bud,
        };
        let result = (|| {
            if selection.session.service() == account::Service::Phaze {
                return phaze::run(&selection.session, &selection.peer, settings.clone());
            }
            let socket = signaling::Socket::open(&selection.session)?;
            let platform = session_platform::Windows::create(settings.clone())?;
            session::run(
                &selection.peer,
                &socket,
                &platform,
                settings.clone(),
                policy,
            )
        })();
        if let Err(error) = result {
            crate::event_log::record(
                "ERROR",
                "native_core_failed",
                serde_json::json!({"stage":"signaling_or_session"}),
            );
            unsafe {
                let message = error
                    .downcast_ref::<transport::HostStatus>()
                    .map(|status| status.user_message(requested_main10))
                    .unwrap_or_else(|| error.to_string());
                ui::message_box(std::ptr::null_mut(), &message);
            }
        }
        session = Some(selection.session);
    }
}
