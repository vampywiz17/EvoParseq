//! Authenticated, optional NGX adapter updates. Never called on the video worker.
//! The DLL has a stable filename; authenticity is established from a signed
//! manifest and checked bytes, not from the filename or a mutable JSON digest.
use anyhow::{bail, ensure, Context, Result};
use base64::Engine as _;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const DLL_NAME: &str = "EvoParseq-NGX-Bridge.dll";
// Per-channel metadata selects an immutable content-addressed DLL. The DLL
// basename stays constant; the hash belongs to the cache directory and signed
// metadata, not to its filename.
const MANIFEST_NAME: &str = "bridge-manifest";
const ABI: u32 = 3;
const MAX_MANIFEST: u64 = 16 * 1024;
const MAX_BRIDGE: u64 = 16 * 1024 * 1024;
const DEV_SOURCE: &str =
    "https://raw.githubusercontent.com/vampywiz17/EvoParseq/dev/ngx-bridge/channels/dev.json";
const STABLE_SOURCE: &str =
    "https://raw.githubusercontent.com/vampywiz17/EvoParseq/main/ngx-bridge/channels/stable.json";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: String,
    signature: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    schema: u32,
    channel: String,
    sequence: u64,
    version: String,
    abi: u32,
    sha256: String,
    size: u64,
    url: String,
}

fn public_key() -> Result<Option<Vec<u8>>> {
    let Some(key) = option_env!("EVOPARSEQ_NGX_MANIFEST_PUBLIC_KEY_B64") else {
        return Ok(None);
    };
    let key = base64::engine::general_purpose::STANDARD
        .decode(key)
        .context("NGX manifest public key is not valid base64")?;
    ensure!(key.len() == 32, "Invalid NGX manifest Ed25519 key length");
    Ok(Some(key))
}
pub const fn enabled() -> bool {
    option_env!("EVOPARSEQ_NGX_MANIFEST_PUBLIC_KEY_B64").is_some()
}
fn channel() -> &'static str {
    match option_env!("EVOPARSEQ_NGX_BRIDGE_CHANNEL") {
        Some("stable") => "stable",
        _ => "dev",
    }
}
fn source() -> &'static str {
    if channel() == "stable" {
        STABLE_SOURCE
    } else {
        DEV_SOURCE
    }
}
fn validate_payload(value: &Payload) -> Result<()> {
    ensure!(
        value.schema == 1 && value.abi == ABI,
        "Unsupported NGX adapter manifest ABI"
    );
    ensure!(value.channel == channel(), "NGX manifest channel mismatch");
    ensure!(value.sequence > 0, "NGX manifest sequence must be positive");
    ensure!(
        !value.version.is_empty()
            && value.version.len() <= 64
            && value
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')),
        "Invalid NGX adapter version"
    );
    ensure!(
        value.size > 0 && value.size <= MAX_BRIDGE,
        "NGX bridge exceeds size limit"
    );
    ensure!(
        value.sha256.len() == 64 && value.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid NGX bridge SHA-256 digest"
    );
    let url = reqwest::Url::parse(&value.url)?;
    ensure!(
        url.scheme() == "https" && url.host_str() == Some("github.com") && url.port().is_none(),
        "NGX bridge must come from GitHub HTTPS"
    );
    let segments: Vec<_> = url
        .path_segments()
        .context("Invalid NGX bridge URL")?
        .collect();
    ensure!(
        segments.len() == 6
            && segments[..4] == ["vampywiz17", "EvoParseq", "releases", "download"]
            && !segments[4].is_empty()
            && segments[5] == DLL_NAME,
        "Invalid NGX bridge release asset path"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "NGX bridge URL contains a query or fragment"
    );
    Ok(())
}
fn validate_manifest(bytes: &[u8], key: &[u8]) -> Result<Payload> {
    ensure!(
        bytes.len() <= MAX_MANIFEST as usize,
        "NGX manifest too large"
    );
    let envelope: Envelope =
        serde_json::from_slice(bytes).context("Invalid NGX manifest envelope")?;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(&envelope.payload)
        .context("Invalid NGX manifest payload")?;
    ensure!(
        payload.len() <= MAX_MANIFEST as usize,
        "NGX manifest payload too large"
    );
    let signature = base64::engine::general_purpose::STANDARD
        .decode(&envelope.signature)
        .context("Invalid NGX manifest signature")?;
    ensure!(
        signature.len() == 64,
        "NGX manifest signature has wrong size"
    );
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&payload, &signature)
        .map_err(|_| anyhow::anyhow!("NGX manifest Ed25519 signature invalid"))?;
    let manifest: Payload =
        serde_json::from_slice(&payload).context("Invalid NGX manifest payload JSON")?;
    validate_payload(&manifest)?;
    Ok(manifest)
}
fn digest(path: &Path, expected: &str, expected_size: u64) -> Result<()> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() == expected_size,
        "NGX bridge file size mismatch"
    );
    let mut hasher = Sha256::new();
    std::io::copy(&mut file.take(MAX_BRIDGE + 1), &mut hasher)?;
    ensure!(
        format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected),
        "NGX bridge SHA-256 mismatch"
    );
    Ok(())
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(format!("{MANIFEST_NAME}-{}.json", channel()))
}
fn valid_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn cache_path(root: &Path, hash: &str) -> Result<PathBuf> {
    ensure!(valid_sha256(hash), "Invalid NGX bridge cache digest");
    Ok(root
        .join("bridge-cache")
        .join(hash.to_ascii_lowercase())
        .join(DLL_NAME))
}
fn cached(root: &Path, key: &[u8]) -> Result<Option<Payload>> {
    let path = manifest_path(root);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    Ok(Some(validate_manifest(&bytes, key)?))
}
pub fn verified_path(root: &Path, legacy_hash: Option<&str>) -> Result<PathBuf> {
    if let Some(key) = public_key()? {
        if let Ok(Some(manifest)) = cached(root, &key) {
            let path = cache_path(root, &manifest.sha256)?;
            if digest(&path, &manifest.sha256, manifest.size).is_ok() {
                return Ok(path);
            }
        }
    }
    // Old unsigned/bootstrap builds cannot select arbitrary manifests or
    // overwrite the signed cache. Their own compiled digest remains the
    // immutable trust anchor and allows multiple EXEs to coexist offline.
    if let Some(hash) = legacy_hash {
        let path = cache_path(root, hash)?;
        if digest_legacy(&path, hash).is_ok() {
            return Ok(path);
        }
        let previous = root.join(format!("evoparseq-ngx-bridge-{hash}.dll"));
        if digest_legacy(&previous, hash).is_ok() {
            return Ok(previous);
        }
        let old_stable = root.join(DLL_NAME);
        if digest_legacy(&old_stable, hash).is_ok() {
            return Ok(old_stable);
        }
    }
    bail!("Verified NVIDIA NGX bridge is unavailable")
}
pub fn verified(root: &Path, legacy_hash: Option<&str>) -> Result<()> {
    verified_path(root, legacy_hash).map(|_| ())
}
fn digest_legacy(path: &Path, expected: &str) -> Result<()> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() > 0 && file.metadata()?.len() <= MAX_BRIDGE,
        "Legacy NGX bridge size invalid"
    );
    let mut hasher = Sha256::new();
    std::io::copy(&mut file.take(MAX_BRIDGE + 1), &mut hasher)?;
    ensure!(
        format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected),
        "Legacy NGX bridge hash mismatch"
    );
    Ok(())
}

pub fn migrate_legacy(root: &Path, legacy_hash: Option<&str>) -> Result<()> {
    let Some(hash) = legacy_hash else { return Ok(()); };
    let dest = cache_path(root, hash)?;
    if digest_legacy(&dest, hash).is_ok() {
        return Ok(());
    }
    // Never touch the already authenticated bridge. This only migrates a
    // verified old install into an isolated cache directory.
    let legacy = root.join(format!("evoparseq-ngx-bridge-{hash}.dll"));
    let previous = root.join(DLL_NAME);
    let source = if digest_legacy(&legacy, hash).is_ok() {
        &legacy
    } else if digest_legacy(&previous, hash).is_ok() {
        &previous
    } else {
        return Ok(());
    };
    std::fs::create_dir_all(dest.parent().context("Missing NGX bridge cache folder")?)?;
    let (stage, mut output) = unique_stage(dest.parent().unwrap(), "bridge-migration")?;
    std::io::copy(&mut File::open(source)?, &mut output)?;
    drop(output);
    digest_legacy(&stage.path, hash)?;
    commit_immutable(&stage.path, &dest, hash, None)?;
    Ok(())
}
fn replace(source: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        std::fs::rename(source, target)?;
    } else {
        // Windows does not permit replacing a DLL another process has loaded.
        // ReplaceFileW retains the old file if replacement cannot be committed.
        #[cfg(windows)]
        unsafe {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" {
                fn ReplaceFileW(
                    dst: *const u16,
                    src: *const u16,
                    backup: *const u16,
                    flags: u32,
                    exclude: *mut std::ffi::c_void,
                    reserved: *mut std::ffi::c_void,
                ) -> i32;
            }
            let dest: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
            let src: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
            if ReplaceFileW(
                dest.as_ptr(),
                src.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(source, target)?;
    }
    Ok(())
}

fn download(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    let timeout = if limit == MAX_MANIFEST { 5 } else { 30 };
    let response = client
        .get(url)
        .timeout(Duration::from_secs(timeout))
        .send()?
        .error_for_status()?;
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "NGX metadata/download too large");
    Ok(bytes)
}

/// Unique, exclusively created staging files cannot be truncated by a second
/// EvoParseq process, even when two instances start at the same millisecond.
pub struct Stage {
    pub path: PathBuf,
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn unique_stage(root: &Path, name: &str) -> Result<(Stage, File)> {
    for _ in 0..16 {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce)?;
        let path = root.join(format!(".{name}-{}-{:016x}.tmp",
            std::process::id(), u64::from_le_bytes(nonce)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((Stage { path }, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Unable to reserve a unique NGX staging file")
}
pub fn unique_stage_for_bootstrap(root: &Path) -> Result<(Stage, File)> {
    unique_stage(root, "bridge-bootstrap")
}
pub fn commit_bootstrap(stage: &Path, target: &Path, digest: &str) -> Result<()> {
    commit_immutable(stage, target, digest, None)
}
fn commit_immutable(stage: &Path, target: &Path, hash: &str, size: Option<u64>) -> Result<()> {
    if let Some(size) = size {
        digest(stage, hash, size)?;
        if digest(target, hash, size).is_ok() {
            return Ok(());
        }
    } else {
        digest_legacy(stage, hash)?;
        if digest_legacy(target, hash).is_ok() {
            return Ok(());
        }
    }
    // A newly downloaded digest gets a brand-new directory. Never overwrite
    // a loaded or signed DLL; this is both downgrade-safe and Win32-lock-safe.
    ensure!(!target.exists(), "Invalid NGX cache entry requires manual repair");
    std::fs::rename(stage, target)?;
    Ok(())
}
fn needs_download(path: &Path, manifest: &Payload) -> bool {
    digest(path, &manifest.sha256, manifest.size).is_err()
}

/// Use an OS-owned mutex, not a persistent lock-file that a crash could leave
/// behind. It serializes per-user updates across simultaneous process starts.
#[cfg(windows)]
struct UpdateLock(*mut std::ffi::c_void);
#[cfg(windows)]
impl UpdateLock {
    fn acquire() -> Result<Self> {
        #[link(name = "kernel32")]
        extern "system" {
            fn CreateMutexW(attributes: *mut std::ffi::c_void,
                initial_owner: i32, name: *const u16) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void,
                milliseconds: u32) -> u32;
        }
        let name: Vec<u16> = "Local\\EvoParseq-NGX-Bridge-Update-v1"
            .encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
        ensure!(!handle.is_null(), "Cannot create NGX bridge update mutex");
        let guard = Self(handle);
        let status = unsafe { WaitForSingleObject(handle, 60_000) };
        ensure!(status == 0 || status == 0x80,
            "NGX bridge updater is already running or could not acquire lock");
        Ok(guard)
    }
}
#[cfg(windows)]
impl Drop for UpdateLock {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        extern "system" {
            fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// Fetch a signed manifest, reuse an existing matching digest or place a new
/// immutable DLL before updating the channel's signed selection metadata.
pub fn refresh(root: &Path, client: &reqwest::blocking::Client,
    legacy_hash: Option<&str>) -> Result<()> {
    #[cfg(windows)]
    let _lock = UpdateLock::acquire()?;
    migrate_legacy(root, legacy_hash)?;
    let Some(key) = public_key()? else {
        return verified(root, legacy_hash)
            .context("No NGX signing key configured; using compiled bootstrap digest");
    };
    let current = cached(root, &key).ok().flatten();
    let data = match download(client, source(), MAX_MANIFEST) {
        Ok(data) => data,
        Err(error) => return verified(root, legacy_hash).context(error),
    };
    let manifest = match validate_manifest(&data, &key) {
        Ok(manifest) => manifest,
        Err(error) => return verified(root, legacy_hash).context(error),
    };
    if current.as_ref().is_some_and(|old| old.sequence > manifest.sequence) {
        return verified(root, legacy_hash);
    }
    let target = cache_path(root, &manifest.sha256)?;
    if needs_download(&target, &manifest) {
        let bytes = match download(client, &manifest.url, MAX_BRIDGE) {
            Ok(bytes) => bytes,
            Err(error) => return verified(root, legacy_hash).context(error),
        };
        ensure!(bytes.len() as u64 == manifest.size,
            "Downloaded NGX bridge size mismatch");
        let parent = target.parent().context("Missing NGX bridge cache folder")?;
        std::fs::create_dir_all(parent)?;
        let (stage, mut file) = unique_stage(parent, "bridge-download")?;
        file.write_all(&bytes)?;
        drop(file);
        if let Err(error) = commit_immutable(
            &stage.path, &target, &manifest.sha256, Some(manifest.size)) {
            return verified(root, legacy_hash).context(error);
        }
    }
    digest(&target, &manifest.sha256, manifest.size)?;
    // Signed metadata is committed only when its DLL is already verified.
    let (stage, mut file) = unique_stage(root, "bridge-manifest")?;
    file.write_all(&data)?;
    drop(file);
    replace(&stage.path, &manifest_path(root))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    #[test]
    fn unchanged_bridge_hash_never_requires_another_download() {
        let file =
            std::env::temp_dir().join(format!("evoparseq-bridge-unchanged-{}", std::process::id()));
        let bytes = b"synthetic bridge bytes, not NVIDIA code";
        std::fs::write(&file, bytes).unwrap();
        let manifest = Payload {
            schema: 1,
            channel: channel().into(),
            sequence: 1,
            version: "test".into(),
            abi: ABI,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size: bytes.len() as u64,
            url: "https://github.com/vampywiz17/EvoParseq/releases/download/v1/EvoParseq-NGX-Bridge.dll".into(),
        };
        assert!(!needs_download(&file, &manifest));
        let mut newer = manifest.clone();
        newer.sha256 = "a".repeat(64);
        assert!(needs_download(&file, &newer));
        std::fs::remove_file(file).unwrap();
    }
    #[test]
    fn envelope_requires_signature_matching_exact_payload() {
        let signer = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
        let payload = serde_json::to_vec(&serde_json::json!({
            "schema": 1,
            "channel": channel(),
            "sequence": 1,
            "version": "1.2.3",
            "abi": 3,
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "size": 326144,
            "url": "https://github.com/vampywiz17/EvoParseq/releases/download/v0.9.9-dev.2/EvoParseq-NGX-Bridge.dll"
        })).unwrap();
        let encoded = serde_json::json!({
            "payload":base64::engine::general_purpose::STANDARD.encode(&payload),
            "signature":base64::engine::general_purpose::STANDARD.encode(signer.sign(&payload).as_ref())
        });
        let bytes = serde_json::to_vec(&encoded).unwrap();
        let key = signer.public_key().as_ref();
        assert!(validate_manifest(&bytes, key).is_ok());
        let mut bad = encoded;
        bad["payload"] =
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b"forged"));
        assert!(validate_manifest(&serde_json::to_vec(&bad).unwrap(), key).is_err());
    }
    #[test]
    fn mismatched_channel_and_hosts_are_rejected() {
        let good = Payload {
            schema:1, channel:channel().into(),sequence:1,version:"1".into(),abi:ABI,
            sha256:"a".repeat(64),size:200,url:"https://github.com/vampywiz17/EvoParseq/releases/download/v1/EvoParseq-NGX-Bridge.dll".into()
        };
        assert!(validate_payload(&good).is_ok());
        let mut bad = good.clone();
        bad.channel = "wrong".into();
        assert!(validate_payload(&bad).is_err());
        bad = good.clone();
        bad.url = "https://example.org/a.dll".into();
        assert!(validate_payload(&bad).is_err());
        bad = good.clone();
        bad.abi = 4;
        assert!(validate_payload(&bad).is_err());
    }
}
