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
    prepare_vsr_runtime(&settings);
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

// Keep these decisions independent of Windows GPU enumeration and network
// access so first-run, corrupted-settings and cached-startup behavior can be
// tested on a GitHub runner without an NVIDIA device.
fn vsr_preparation_allowed(bridge_supported: bool, settings_valid: bool) -> bool {
    bridge_supported && settings_valid
}

fn vsr_update_needed(cache_verified: bool, signed_updates: bool) -> bool {
    !cache_verified || signed_updates
}

fn prepare_vsr_runtime(settings: &crate::connection_settings::Manager) {
    use crate::connection_settings::VsrRuntimeConsent;
    if !vsr_preparation_allowed(
        crate::vsr_runtime::bridge_supported(),
        settings.view().1.is_none(),
    ) {
        return;
    }
    let data = settings.directory();
    let reused = crate::vsr_runtime::reuse_previous(&data).unwrap_or(false);
    // Reuse the verified cached DLL without network traffic if signed
    // updates are unavailable; signed builds must recheck their manifest.
    if !vsr_update_needed(reused, crate::vsr_runtime::updates_enabled()) {
        return;
    }
    if !crate::gpu_windows::enumerate()
        .is_ok_and(|gpus| gpus.iter().any(|g| g.preference.vendor_id == 0x10de))
    {
        return;
    }
    let mut preferences = settings.view().0;
    if preferences.vsr_runtime_consent == VsrRuntimeConsent::Ask {
        let text: Vec<u16> = crate::vsr_runtime::PROMPT
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = "EvoParseq - optional NVIDIA VSR"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let accepted = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_YESNO
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_DEFBUTTON2
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONQUESTION,
            )
        } == windows_sys::Win32::UI::WindowsAndMessaging::IDYES;
        preferences.vsr_runtime_consent = if accepted {
            VsrRuntimeConsent::Allowed
        } else {
            VsrRuntimeConsent::Declined
        };
        if settings
            .update_vsr_consent(preferences.vsr_runtime_consent)
            .is_err()
        {
            crate::event_log::record(
                "WARN",
                "vsr_runtime_consent_save_failed",
                serde_json::json!({}),
            );
            unsafe {
                ui::message_box(std::ptr::null_mut(), "The VSR download permission could not be saved beside settings.json. No download was started; normal GPU video remains available.");
            }
            return;
        }
    }
    if preferences.vsr_runtime_consent == VsrRuntimeConsent::Allowed {
        if let Err(error) = crate::vsr_runtime::install() {
            crate::event_log::record(
                "WARN",
                "vsr_runtime_install_failed",
                serde_json::json!({"reason":error.to_string()}),
            );
            // Optional adapter updates never produce a blocking modal on
            // startup. Failure leaves the verified cached bridge in place, if
            // available, and ordinary GPU video remains usable.
        }
    }
}

#[cfg(test)]
mod vsr_runtime_guard_tests {
    #[test]
    fn startup_does_not_attempt_optional_vsr_without_bridge_or_valid_settings() {
        assert!(!super::vsr_preparation_allowed(false, false));
        assert!(!super::vsr_preparation_allowed(false, true));
        assert!(!super::vsr_preparation_allowed(true, false));
        assert!(super::vsr_preparation_allowed(true, true));
    }

    #[test]
    fn startup_reuses_verified_bridge_without_redundant_download() {
        // A legacy/pinned EXE with the DLL already cached must skip the
        // startup download even if a different EXE version is installed.
        assert!(!super::vsr_update_needed(true, false));
        // A manifest-aware build must still check for a new signed release,
        // but the updater compares the SHA-256 before downloading a DLL.
        assert!(super::vsr_update_needed(true, true));
        // A first-run install has to be attempted regardless of channel.
        assert!(super::vsr_update_needed(false, false));
        assert!(super::vsr_update_needed(false, true));
    }

    #[test]
    fn corrupt_settings_never_start_optional_nvidia_download() {
        let fixture = std::env::temp_dir().join(format!(
            "evoparseq-vsr-invalid-settings-{}.json",
            std::process::id()
        ));
        let invalid = b"{not-valid-json";
        std::fs::write(&fixture, invalid).unwrap();
        let settings = crate::connection_settings::Manager::open(fixture.clone());
        assert!(settings.view().1.is_some());

        // Even an RTX-enabled release must return before GPU enumeration,
        // displaying any consent dialog or starting a vendor download.
        super::prepare_vsr_runtime(&settings);
        assert_eq!(std::fs::read(&fixture).unwrap().as_slice(), &invalid[..]);
        std::fs::remove_file(fixture).unwrap();
    }
}
