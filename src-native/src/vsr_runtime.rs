//! Optional vendor runtime installation. Never called from the video worker.
//! Only the exact pinned NVIDIA archive is accepted. Its two needed entries
//! are fixed deflate streams; this is deliberately not a general ZIP extractor.
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

#[path = "vsr_bridge_update.rs"]
mod bridge_update;
#[path = "vsr_browser_import.rs"]
mod browser_import;

pub const fn updates_enabled() -> bool {
    bridge_update::enabled()
}
pub const fn bridge_supported() -> bool {
    bridge_update::enabled() || option_env!("EVOPARSEQ_NGX_BRIDGE_SHA256").is_some()
}

pub const DLL_HASH: &str = "c3d88eea5ff7a548edefa66414cf6e77464d0947277c904f324dd23abf58a1ed";
const ARCHIVE_HASH: &str = "abf4f34e2b5a618e355b0d5a0365d8ecc3db4396e756e4c850a867e1ae2ed69e";
const ARCHIVE_SIZE: u64 = 63_493_768;
const DLL_SIZE: u64 = 19_140_144;
const URL: &str = "https://api.ngc.nvidia.com/v2/models/nvidia/multimedia/dlpp/versions/1.5/files/RTX_Video_SDK_v1.1.0.zip";
pub const PROMPT: &str = "Enable optional NVIDIA VSR components?\n\nDownloads will open only in your default browser, never inside EvoParseq. After downloading, select the ZIP/DLL with the file picker: EvoParseq verifies the exact SDK SHA-256 or signed bridge manifest and installs it automatically in your per-user cache. No manual file copying or administrator rights are needed.\n\nVSR requires a compatible RTX GPU/driver, is off by default and may increase latency. Choosing No keeps ordinary GPU video available.";

const BRIDGE_MAX_SIZE: u64 = 16 * 1024 * 1024;
// Older builds pin their bootstrapping adapter; signed manifests can update
// independently without recompiling the EXE.
const BRIDGE_HASH: Option<&str> = option_env!("EVOPARSEQ_NGX_BRIDGE_SHA256");
pub const RESTART_HINT: &str =
    " Restart the app to choose the NVIDIA VSR runtime download in your browser.";

fn directory_under(local: &Path) -> PathBuf {
    local.join("EvoParseq").join("vsr").join("1.1.0")
}
fn local_directory() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .context("Windows local application data folder unavailable")?;
    let local = PathBuf::from(local);
    ensure!(
        local.is_absolute(),
        "Windows local application data folder must be absolute"
    );
    Ok(local)
}
pub fn directory() -> Result<PathBuf> {
    Ok(directory_under(&local_directory()?))
}
pub fn validated_bridge_path() -> Result<PathBuf> {
    let root = validated_directory()?;
    bridge_update::verified_path(&root, BRIDGE_HASH)
}

pub fn present() -> bool {
    validated_directory().is_ok()
}
fn hash(mut file: &File) -> Result<String> {
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}
fn verify(path: &Path, size: u64, expected: &str) -> Result<()> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() == size && hash(&file)? == expected,
        "NVIDIA runtime checksum mismatch"
    );
    Ok(())
}

/// Accept an explicitly user-selected local SDK ZIP. Its exact immutable
/// size and SHA-256 are mandatory before any extraction.
fn save_checked_archive(body: impl Read, path: &Path, size: u64, expected: &str) -> Result<()> {
    let mut file = File::create(path)?;
    let count = std::io::copy(&mut body.take(size + 1), &mut file)?;
    drop(file);
    ensure!(count == size, "NVIDIA SDK ZIP size mismatch");
    verify(path, size, expected)
}

pub fn validated_directory() -> Result<PathBuf> {
    let root = directory()?;
    verify(&root.join("nvngx_vsr.dll"), DLL_SIZE, DLL_HASH)
        .context("VSR runtime missing or invalid; approve its download in Settings and restart")?;
    bridge_update::verified(&root, BRIDGE_HASH)?;
    Ok(std::fs::canonicalize(root)?)
}
/// Verify the exact release adapter built for this executable before LoadLibraryExW.
fn verify_bridge(path: &Path) -> Result<()> {
    let expected = BRIDGE_HASH.context(
        "This build has no pinned NVIDIA NGX bridge; build through the Windows release CI",
    )?;
    verify_bridge_with_hash(path, expected)
}

/// Keep the byte-level verification testable without loading NVIDIA code or
/// contacting the live release service. Production always passes the pinned
/// hash embedded into EvoParseq.exe.
fn verify_bridge_with_hash(path: &Path, expected: &str) -> Result<()> {
    ensure!(
        expected.len() == 64 && expected.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid compile-time NGX bridge digest"
    );
    let file = File::open(path)
        .context("NVIDIA NGX adapter not installed; approve its download in Settings")?;
    ensure!(
        file.metadata()?.len() > 0 && file.metadata()?.len() <= BRIDGE_MAX_SIZE,
        "Invalid NVIDIA NGX adapter size"
    );
    ensure!(
        hash(&file)?.eq_ignore_ascii_case(expected),
        "NVIDIA NGX adapter checksum mismatch"
    );
    Ok(())
}

/// Reuse only our previously pinned runtime, not arbitrary driver/application DLLs.
pub fn reuse_previous(data: &Path) -> Result<bool> {
    let root = directory()?;
    if validated_directory().is_ok() {
        return Ok(true);
    }
    // The existing NVIDIA runtime can be reused even if the new bridge is absent.
    // Never recopy it while only the optional bridge is missing.
    if verify(&root.join("nvngx_vsr.dll"), DLL_SIZE, DLL_HASH).is_err() {
        let local = local_directory()?;
        let candidates = [
            data.join("vsr/1.1.0"),
            local.join("EvoParseq/rtx-video").join(DLL_HASH),
        ];
        reuse_from(&root, &candidates)?;
    }
    // Migrate only this EXE's hash-verified adapter into its own immutable
    // cache entry. Never overwrite another build's signed or pinned DLL.
    bridge_update::migrate_legacy(&root, BRIDGE_HASH)?;
    Ok(validated_directory().is_ok())
}
fn reuse_from(root: &Path, candidates: &[PathBuf]) -> Result<bool> {
    for old in candidates {
        let source = old.join("nvngx_vsr.dll");
        if !source.is_file() {
            continue;
        }
        verify(&source, DLL_SIZE, DLL_HASH)?;
        std::fs::create_dir_all(root)?;
        let temporary = Temporary(root.join("runtime.tmp"));
        std::fs::copy(&source, &temporary.0)?;
        verify(&temporary.0, DLL_SIZE, DLL_HASH)?;
        std::fs::rename(&temporary.0, root.join("nvngx_vsr.dll"))?;
        let license = old.join("NVIDIA-RTX-VIDEO-LICENSE.pdf");
        if license.is_file() {
            let _ = std::fs::copy(license, root.join("NVIDIA-RTX-VIDEO-LICENSE.pdf"));
        }
        // Leave the old cache intact: another running/older EXE may still use it.
        return Ok(true);
    }
    Ok(false)
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

enum BridgeRequest {
    Signed(bridge_update::UpdateOffer),
    Pinned { url: String, sha256: &'static str },
}

fn bridge_request(root: &Path, client: &reqwest::blocking::Client) -> Result<Option<BridgeRequest>> {
    if bridge_update::enabled() {
        match bridge_update::check_manifest(root, client) {
            Ok(Some(offer)) => return Ok(Some(BridgeRequest::Signed(offer))),
            Ok(None) => (),
            Err(error) => {
                // Missing/offline manifest must never break the existing
                // pinned adapter or require a binary network request.
                crate::event_log::record(
                    "WARN", "vsr_manifest_check_failed",
                    serde_json::json!({"reason":error.to_string()}),
                );
            }
        }
    }
    if bridge_update::verified(root, BRIDGE_HASH).is_ok() {
        return Ok(None);
    }
    let Some(hash) = BRIDGE_HASH else {
        anyhow::bail!("No signed NGX manifest and no pinned fallback DLL are available");
    };
    ensure!(
        hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid compiled NGX adapter digest"
    );
    let version = env!("CARGO_PKG_VERSION");
    let url = format!(
        "https://github.com/vampywiz17/EvoParseq/releases/download/v{version}/EvoParseq-NGX-Bridge.dll"
    );
    Ok(Some(BridgeRequest::Pinned { url, sha256: hash }))
}

fn install_sdk_from_browser(root: &Path, archive: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    // Verify the *copied* bytes before extracting fixed offsets, even if
    // another process modified the user's Downloads directory meanwhile.
    let (stage, output) = bridge_update::unique_stage_for_bootstrap(root)?;
    // Close our exclusive reservation before reopening the same filename.
    drop(output);
    save_checked_archive(File::open(archive)?, &stage.path, ARCHIVE_SIZE, ARCHIVE_HASH)?;
    let (dll_stage, dll_file) = bridge_update::unique_stage_for_bootstrap(root)?;
    drop(dll_file);
    extract(&stage.path, &dll_stage.path, 37_964_664, 14_455_982, DLL_SIZE)?;
    verify(&dll_stage.path, DLL_SIZE, DLL_HASH)?;
    let destination = root.join("nvngx_vsr.dll");
    if !destination.is_file() {
        std::fs::rename(&dll_stage.path, &destination)?;
    } else if verify(&destination, DLL_SIZE, DLL_HASH).is_err() {
        // A loaded but corrupt DLL cannot be replaced on Windows; leave the
        // existing file untouched if rename/replace is not permitted.
        std::fs::remove_file(&destination)?;
        std::fs::rename(&dll_stage.path, &destination)?;
    }
    if !root.join("NVIDIA-RTX-VIDEO-LICENSE.pdf").is_file() {
        extract(
            &stage.path,
            &root.join("NVIDIA-RTX-VIDEO-LICENSE.pdf"),
            62_433_254,
            131_214,
            136_535,
        )?;
    }
    crate::event_log::record(
        "INFO", "vsr_runtime_installed",
        serde_json::json!({"sdk_version":"1.1.0","sha256":DLL_HASH,"source":"browser_import"}),
    );
    Ok(())
}

/// Browser-based acquisition: the app downloads signed JSON metadata only.
/// All SDK ZIP and NGX DLL bytes come from a user-selected local file.
pub fn install() -> Result<()> {
    let root = directory()?;
    std::fs::create_dir_all(&root)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .https_only(true)
        .build()?;
    // Check signed JSON only. No HTTP request for executable contents.
    let bridge = bridge_request(&root, &client)?;
    if verify(&root.join("nvngx_vsr.dll"), DLL_SIZE, DLL_HASH).is_err() {
        let Some(path) = browser_import::choose(URL, "zip")? else {
            return Ok(()); // User deferred download; normal video still works.
        };
        install_sdk_from_browser(&root, &path)?;
    }
    if let Some(bridge) = bridge {
        let (url, signed) = match &bridge {
            BridgeRequest::Signed(offer) => (&offer.browser_url, true),
            BridgeRequest::Pinned { url, .. } => (url, false),
        };
        let Some(path) = browser_import::choose(url, "dll")? else {
            return Ok(());
        };
        match &bridge {
            BridgeRequest::Signed(offer) => bridge_update::import_signed(&root, offer, &path)?,
            BridgeRequest::Pinned { sha256, .. } => {
                bridge_update::import_pinned(&root, &path, sha256)?
            }
        }
        crate::event_log::record(
            "INFO", "vsr_bridge_imported",
            serde_json::json!({"source":"browser_import","signed":signed}),
        );
    }
    Ok(())
}
fn extract(archive: &Path, output: &Path, offset: u64, compressed: u64, size: u64) -> Result<()> {
    let mut input = File::open(archive)?;
    input.seek(SeekFrom::Start(offset))?;
    let decoder = flate2::read::DeflateDecoder::new(input.take(compressed));
    let mut file = File::create(output)?;
    ensure!(
        std::io::copy(&mut decoder.take(size + 1), &mut file)? == size,
        "NVIDIA runtime extraction size mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vendor_sdk_browser_address_is_https() {
        let url = reqwest::Url::parse(URL).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("api.ngc.nvidia.com"));
    }

    #[test]
    fn selected_sdk_archive_copy_has_to_match_exact_hash() {
        let path = std::env::temp_dir().join(format!(
            "evoparseq-ngx-direct-body-{}.bin",
            std::process::id()
        ));
        let _cleanup = Temporary(path.clone());
        let body = b"already followed HTTPS redirect";
        let digest = format!("{:x}", Sha256::digest(body));
        save_checked_archive(
            std::io::Cursor::new(body),
            &path,
            body.len() as u64,
            &digest,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
    }

    #[test]
    fn selected_sdk_archive_rejects_tampering_and_oversized_body() {
        let path = std::env::temp_dir().join(format!(
            "evoparseq-ngx-invalid-body-{}.bin",
            std::process::id()
        ));
        let _cleanup = Temporary(path.clone());
        let digest = format!("{:x}", Sha256::digest(b"correct"));
        assert!(save_checked_archive(std::io::Cursor::new(b"changed"), &path, 7, &digest).is_err());
        assert!(save_checked_archive(std::io::Cursor::new(b"extra!"), &path, 5, &digest).is_err());
    }

    #[test]
    fn invalid_previous_cache_is_not_migrated() {
        let fixture =
            std::env::temp_dir().join(format!("evoparseq-vsr-migrate-{}", std::process::id()));
        let old = fixture.join("portable/vsr/1.1.0");
        let target = directory_under(&fixture.join("local-app-data"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("nvngx_vsr.dll"), b"not NVIDIA code").unwrap();
        assert!(reuse_from(&target, std::slice::from_ref(&old)).is_err());
        assert!(!target.exists());
        assert!(old.join("nvngx_vsr.dll").exists());
        std::fs::remove_dir_all(fixture).unwrap();
    }
    #[test]
    fn mismatched_vendor_runtime_is_rejected_before_loading() {
        let path =
            std::env::temp_dir().join(format!("evoparseq-vsr-corrupt-{}", std::process::id()));
        std::fs::write(&path, b"not NVIDIA code").unwrap();
        assert!(verify(&path, 15, DLL_HASH).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn optional_bridge_hash_validation_rejects_tampering() {
        let fixture =
            std::env::temp_dir().join(format!("evoparseq-ngx-verify-{}", std::process::id()));
        std::fs::create_dir_all(&fixture).unwrap();
        let dll = fixture.join("evoparseq-ngx-bridge.dll");
        let sample = b"synthetic adapter fixture; no NVIDIA SDK content";
        std::fs::write(&dll, sample).unwrap();
        let digest = format!("{:x}", Sha256::digest(sample));

        // Real bytes must match the expected 64-character digest exactly.
        assert!(verify_bridge_with_hash(&dll, &digest).is_ok());
        assert!(verify_bridge_with_hash(&dll, &digest.to_uppercase()).is_ok());
        assert!(verify_bridge_with_hash(&dll, "not-a-digest").is_err());
        assert!(verify_bridge_with_hash(&dll, &"g".repeat(64)).is_err());
        assert!(verify_bridge_with_hash(&dll, &"0".repeat(64)).is_err());

        std::fs::write(&dll, b"altered fixture").unwrap();
        assert!(verify_bridge_with_hash(&dll, &digest).is_err());
        std::fs::write(&dll, b"").unwrap();
        assert!(verify_bridge_with_hash(&dll, &digest).is_err());

        // A sparse oversized file must fail before its content is hashed.
        File::create(&dll)
            .unwrap()
            .set_len(BRIDGE_MAX_SIZE + 1)
            .unwrap();
        assert!(verify_bridge_with_hash(&dll, &digest).is_err());
        std::fs::remove_file(&dll).unwrap();
        assert!(verify_bridge_with_hash(&dll, &digest).is_err());
        std::fs::remove_dir_all(&fixture).unwrap();
    }

    #[test]
    fn optional_bridge_uses_stable_basename_in_isolated_digest_directory() {
        let cache = Path::new("evoparseq-test-cache");
        let digest = "a".repeat(64);
        let adapter = bridge_update::cache_path(cache, &digest).unwrap();
        assert_eq!(adapter.file_name().unwrap(), "EvoParseq-NGX-Bridge.dll");
        assert_eq!(
            adapter.parent().unwrap().file_name().unwrap(),
            digest.as_str()
        );
        assert!(adapter.starts_with(cache));
    }

    #[test]
    fn pinned_archive_stream_offsets_reproduce_the_official_runtime() {
        let Some(sdk) = std::env::var_os("EVOPARSEQ_RTX_VIDEO_SDK") else {
            return;
        };
        let archive = PathBuf::from(sdk)
            .parent()
            .unwrap()
            .join("RTX_Video_SDK_v1.1.0.zip");
        verify(&archive, ARCHIVE_SIZE, ARCHIVE_HASH).unwrap();
        let path =
            std::env::temp_dir().join(format!("evoparseq-vsr-extract-{}", std::process::id()));
        extract(&archive, &path, 37_964_664, 14_455_982, DLL_SIZE).unwrap();
        verify(&path, DLL_SIZE, DLL_HASH).unwrap();
        let migrated = path.with_extension("cache");
        let previous = path.with_extension("old");
        std::fs::create_dir_all(&previous).unwrap();
        std::fs::rename(&path, previous.join("nvngx_vsr.dll")).unwrap();
        assert!(reuse_from(&migrated, std::slice::from_ref(&previous)).unwrap());
        verify(&migrated.join("nvngx_vsr.dll"), DLL_SIZE, DLL_HASH).unwrap();
        assert!(previous.join("nvngx_vsr.dll").exists());
        std::fs::remove_dir_all(previous).unwrap();
        std::fs::remove_dir_all(migrated).unwrap();
    }
}
