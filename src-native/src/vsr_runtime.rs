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

pub const DLL_HASH: &str = "c3d88eea5ff7a548edefa66414cf6e77464d0947277c904f324dd23abf58a1ed";
const ARCHIVE_HASH: &str = "abf4f34e2b5a618e355b0d5a0365d8ecc3db4396e756e4c850a867e1ae2ed69e";
const ARCHIVE_SIZE: u64 = 63_493_768;
const DLL_SIZE: u64 = 19_140_144;
const URL: &str = "https://api.ngc.nvidia.com/v2/models/nvidia/multimedia/dlpp/versions/1.5/files/RTX_Video_SDK_v1.1.0.zip";
pub const PROMPT: &str = "Download the optional NVIDIA VSR runtime from NVIDIA and the EvoParseq NGX adapter from GitHub?\n\nThe download is about 61 MB. The checked NVIDIA runtime DLL (about 19 MB) and a version-pinned EvoParseq NGX adapter DLL will be stored under %LOCALAPPDATA%/EvoParseq/vsr/1.1.0 for your Windows user (no administrator rights required). Sign-in opens after setup finishes.\n\nVSR requires a compatible RTX GPU/driver, is off by default and may increase latency. The NVIDIA SDK license is included in the application package. Choosing No keeps ordinary GPU video available.";

const BRIDGE_MAX_SIZE: u64 = 16 * 1024 * 1024;
// This digest is injected by the release build after compiling the isolated DLL.
const BRIDGE_HASH: Option<&str> = option_env!("EVOPARSEQ_NGX_BRIDGE_SHA256");
pub const RESTART_HINT: &str =
    " Restart the app to download the approved NVIDIA VSR runtime before sign-in.";

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
fn bridge_path(root: &Path) -> PathBuf {
    // Allow different versions of EvoParseq to run concurrently; DLLs are
    // immutable by digest and Windows keeps loaded modules locked.
    root.join(format!(
        "evoparseq-ngx-bridge-{}.dll",
        BRIDGE_HASH.unwrap_or("unavailable")
    ))
}

pub fn validated_bridge_path() -> Result<PathBuf> {
    let root = validated_directory()?;
    Ok(bridge_path(&root))
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

/// Accept the final HTTPS response body regardless of whether NVIDIA served it
/// directly or reqwest already followed its redirects. The immutable archive
/// size and SHA-256 remain mandatory before any extraction.
fn save_checked_archive(body: impl Read, path: &Path, size: u64, expected: &str) -> Result<()> {
    let mut file = File::create(path)?;
    let count = std::io::copy(&mut body.take(size + 1), &mut file)?;
    drop(file);
    ensure!(count == size, "NVIDIA SDK download size mismatch");
    verify(path, size, expected)
}

/// The NGC API sometimes sends a redirect or a 200 metadata response with a
/// Location header instead of archive bytes. Never interpret that body as ZIP.
/// An ordinary 200 with no Location is a direct archive response.
fn archive_download_location(
    origin: &reqwest::Url,
    status: reqwest::StatusCode,
    location: Option<&str>,
) -> Result<Option<reqwest::Url>> {
    ensure!(
        status.is_success() || status.is_redirection(),
        "NVIDIA download service rejected the request"
    );
    if let Some(value) = location {
        let url = origin.join(value)?;
        ensure!(url.scheme() == "https", "NVIDIA download must use HTTPS");
        return Ok(Some(url));
    }
    ensure!(
        status.is_success(),
        "NVIDIA download location missing from redirect response"
    );
    Ok(None)
}
pub fn validated_directory() -> Result<PathBuf> {
    let root = directory()?;
    verify(&root.join("nvngx_vsr.dll"), DLL_SIZE, DLL_HASH)
        .context("VSR runtime missing or invalid; approve its download in Settings and restart")?;
    verify_bridge(&bridge_path(&root))?;
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

/// Download only the version-pinned GitHub release asset for our own bridge.
/// The checksum itself is compiled into the EXE, not fetched from a mutable manifest.
fn install_bridge(root: &Path, client: &reqwest::blocking::Client) -> Result<()> {
    let expected = BRIDGE_HASH.context("This build has no NVIDIA NGX adapter digest")?;
    ensure!(
        expected.len() == 64 && expected.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid NGX adapter digest"
    );
    let version = env!("CARGO_PKG_VERSION");
    let url = format!(
        "https://github.com/vampywiz17/EvoParseq/releases/download/v{version}/EvoParseq-NGX-Bridge.dll"
    );
    let response = client
        .get(url)
        .send()
        .context("Cannot download the optional NGX adapter from the matching EvoParseq release")?
        .error_for_status()
        .context("The matching NGX adapter release asset is unavailable")?;
    let temp = Temporary(root.join("ngx-bridge.tmp"));
    let mut file = File::create(&temp.0)?;
    let length = std::io::copy(&mut response.take(BRIDGE_MAX_SIZE + 1), &mut file)?;
    drop(file);
    ensure!(
        length > 0 && length <= BRIDGE_MAX_SIZE,
        "NGX adapter exceeds size limit"
    );
    verify_bridge(&temp.0)?;
    let target = bridge_path(root);
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    std::fs::rename(&temp.0, target)?;
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
    // The bridge still needs to be fetched by install() when it is missing.
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

pub fn install() -> Result<()> {
    let root = directory()?;
    std::fs::create_dir_all(&root)?;
    let temporary = Temporary(root.join("sdk-download.tmp"));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" || attempt.previous().len() > 5 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    // First verify the separately built adapter. Never load an unverified DLL.
    if verify_bridge(&bridge_path(&root)).is_err() {
        install_bridge(&root, &client)?;
    }
    // A previously installed, hash-verified NVIDIA runtime remains usable.
    if verify(&root.join("nvngx_vsr.dll"), DLL_SIZE, DLL_HASH).is_ok() {
        return Ok(());
    }
    // Discover NGC's archive response WITHOUT auto-following redirects: doing
    // so loses the Location header that NGC sometimes uses as metadata.
    let discovery = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let initial = discovery
        .get(URL)
        .send()
        .context("Cannot contact NVIDIA download service")?;
    let location = initial
        .headers()
        .get(reqwest::header::LOCATION)
        .map(|value| value.to_str())
        .transpose()?;
    let target = archive_download_location(initial.url(), initial.status(), location)?;
    let response = if let Some(url) = target {
        client
            .get(url)
            .send()
            .context("Cannot fetch NVIDIA's HTTPS archive location")?
    } else {
        initial
    };
    let response = response
        .error_for_status()
        .context("NVIDIA archive download was rejected")?;
    ensure!(
        response.url().scheme() == "https",
        "NVIDIA download must use HTTPS"
    );
    save_checked_archive(response, &temporary.0, ARCHIVE_SIZE, ARCHIVE_HASH)?;
    // These offsets are valid ONLY after checking the complete immutable SDK
    // 1.1.0 archive hash above. No filenames from an archive become filesystem paths.
    extract(
        &temporary.0,
        &root.join("NVIDIA-RTX-VIDEO-LICENSE.pdf"),
        62_433_254,
        131_214,
        136_535,
    )?;
    let dll = Temporary(root.join("runtime.tmp"));
    extract(&temporary.0, &dll.0, 37_964_664, 14_455_982, DLL_SIZE)?;
    verify(&dll.0, DLL_SIZE, DLL_HASH)?;
    std::fs::rename(&dll.0, root.join("nvngx_vsr.dll"))?;
    crate::event_log::record(
        "INFO",
        "vsr_runtime_installed",
        serde_json::json!({"sdk_version":"1.1.0","sha256":DLL_HASH}),
    );
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
    fn nvidia_download_discovery_handles_metadata_redirect_and_direct_body() {
        let origin = reqwest::Url::parse(URL).unwrap();
        assert!(
            archive_download_location(&origin, reqwest::StatusCode::OK, None)
                .unwrap()
                .is_none()
        );
        let redirect = "https://download.nvidia.com/rtx-video-sdk.zip";
        assert_eq!(
            archive_download_location(&origin, reqwest::StatusCode::FOUND, Some(redirect))
                .unwrap()
                .unwrap()
                .as_str(),
            redirect,
        );
        assert_eq!(
            archive_download_location(&origin, reqwest::StatusCode::OK, Some(redirect))
                .unwrap()
                .unwrap()
                .as_str(),
            redirect,
        );
        assert!(archive_download_location(&origin, reqwest::StatusCode::FOUND, None).is_err());
        assert!(archive_download_location(
            &origin,
            reqwest::StatusCode::FOUND,
            Some("http://other.example/zip")
        )
        .is_err());
    }

    #[test]
    fn pinned_download_accepts_final_response_body_without_location_header() {
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
    fn pinned_download_rejects_tampering_and_oversized_body() {
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
    fn optional_bridge_filename_is_version_isolated_by_digest() {
        let cache = Path::new("evoparseq-test-cache");
        let adapter = bridge_path(cache);
        let expected = BRIDGE_HASH.unwrap_or("unavailable");
        assert_eq!(
            adapter,
            cache.join(format!("evoparseq-ngx-bridge-{expected}.dll"))
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
