//! Authenticated, optional NGX adapter updates. Never called on the video worker.
//! The DLL has a stable filename; authenticity is established from a signed
//! manifest and checked bytes, not from the filename or a mutable JSON digest.
use anyhow::{bail, ensure, Context, Result};
use base64::Engine as _;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::{Path, PathBuf}};

pub const DLL_NAME: &str = "EvoParseq-NGX-Bridge.dll";
const MANIFEST_NAME: &str = "bridge-manifest.json";
const ABI: u32 = 3;
const MAX_MANIFEST: u64 = 16 * 1024;
const MAX_BRIDGE: u64 = 16 * 1024 * 1024;
const DEV_SOURCE: &str = "https://raw.githubusercontent.com/vampywiz17/EvoParseq/dev/ngx-bridge/channels/dev.json";
const STABLE_SOURCE: &str = "https://raw.githubusercontent.com/vampywiz17/EvoParseq/main/ngx-bridge/channels/stable.json";

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
pub fn enabled() -> bool {
    option_env!("EVOPARSEQ_NGX_MANIFEST_PUBLIC_KEY_B64").is_some()
}
fn channel() -> &'static str {
    match option_env!("EVOPARSEQ_NGX_BRIDGE_CHANNEL") {
        Some("stable") => "stable",
        _ => "dev",
    }
}
fn source() -> &'static str {
    if channel() == "stable" { STABLE_SOURCE } else { DEV_SOURCE }
}
fn validate_payload(value: &Payload) -> Result<()> {
    ensure!(value.schema == 1 && value.abi == ABI, "Unsupported NGX adapter manifest ABI");
    ensure!(value.channel == channel(), "NGX manifest channel mismatch");
    ensure!(value.sequence > 0, "NGX manifest sequence must be positive");
    ensure!(!value.version.is_empty() && value.version.len() <= 64 &&
        value.version.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')),
        "Invalid NGX adapter version");
    ensure!(value.size > 0 && value.size <= MAX_BRIDGE, "NGX bridge exceeds size limit");
    ensure!(value.sha256.len() == 64 &&
        value.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid NGX bridge SHA-256 digest");
    let url = reqwest::Url::parse(&value.url)?;
    ensure!(url.scheme() == "https" && url.host_str() == Some("github.com") &&
        url.port().is_none(), "NGX bridge must come from GitHub HTTPS");
    let segments: Vec<_> = url.path_segments().context("Invalid NGX bridge URL")?.collect();
    ensure!(segments.len() == 6 && segments[..4] == ["vampywiz17","EvoParseq","releases","download"] &&
        !segments[4].is_empty() && segments[5] == DLL_NAME, "Invalid NGX bridge release asset path");
    ensure!(url.query().is_none() && url.fragment().is_none(), "NGX bridge URL contains a query or fragment");
    Ok(())
}
fn validate_manifest(bytes: &[u8], key: &[u8]) -> Result<Payload> {
    ensure!(bytes.len() <= MAX_MANIFEST as usize, "NGX manifest too large");
    let envelope: Envelope = serde_json::from_slice(bytes)
        .context("Invalid NGX manifest envelope")?;
    let payload = base64::engine::general_purpose::STANDARD.decode(&envelope.payload)
        .context("Invalid NGX manifest payload")?;
    ensure!(payload.len() <= MAX_MANIFEST as usize, "NGX manifest payload too large");
    let signature = base64::engine::general_purpose::STANDARD.decode(&envelope.signature)
        .context("Invalid NGX manifest signature")?;
    ensure!(signature.len() == 64, "NGX manifest signature has wrong size");
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&payload, &signature)
        .map_err(|_| anyhow::anyhow!("NGX manifest Ed25519 signature invalid"))?;
    let manifest: Payload = serde_json::from_slice(&payload)
        .context("Invalid NGX manifest payload JSON")?;
    validate_payload(&manifest)?;
    Ok(manifest)
}
fn digest(path: &Path, expected: &str, expected_size: u64) -> Result<()> {
    let file = File::open(path)?;
    ensure!(file.metadata()?.len() == expected_size, "NGX bridge file size mismatch");
    let mut hasher = Sha256::new();
    std::io::copy(&mut file.take(MAX_BRIDGE + 1), &mut hasher)?;
    ensure!(format!("{:x}",hasher.finalize()).eq_ignore_ascii_case(expected),
        "NGX bridge SHA-256 mismatch");
    Ok(())
}
fn cached(root: &Path, key: &[u8]) -> Result<Option<Payload>> {
    let path = root.join(MANIFEST_NAME);
    if !path.is_file() { return Ok(None); }
    let bytes = std::fs::read(path)?;
    Ok(Some(validate_manifest(&bytes, key)?))
}
pub fn verified(root: &Path, legacy_hash: Option<&str>) -> Result<()> {
    let target = root.join(DLL_NAME);
    if let Some(key) = public_key()? {
        if let Ok(Some(manifest)) = cached(root, &key) {
            if digest(&target, &manifest.sha256, manifest.size).is_ok() {
                return Ok(());
            }
        }
    }
    if let Some(hash) = legacy_hash {
        if target.is_file() && digest_legacy(&target, hash).is_ok() {
            return Ok(());
        }
    }
    bail!("Verified NVIDIA NGX bridge is unavailable")
}
fn digest_legacy(path: &Path, expected: &str) -> Result<()> {
    let file = File::open(path)?;
    ensure!(file.metadata()?.len() > 0 && file.metadata()?.len() <= MAX_BRIDGE,
        "Legacy NGX bridge size invalid");
    let mut hasher = Sha256::new();
    std::io::copy(&mut file.take(MAX_BRIDGE + 1), &mut hasher)?;
    ensure!(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected),
        "Legacy NGX bridge hash mismatch");
    Ok(())
}
pub fn migrate_legacy(root: &Path, legacy_hash: Option<&str>) -> Result<()> {
    let Some(hash) = legacy_hash else { return Ok(()); };
    let dest = root.join(DLL_NAME);
    if digest_legacy(&dest, hash).is_ok() { return Ok(()); }
    let legacy = root.join(format!("evoparseq-ngx-bridge-{hash}.dll"));
    if !legacy.is_file() { return Ok(()); }
    digest_legacy(&legacy, hash)?;
    let stage = root.join("bridge-migration.tmp");
    std::fs::copy(&legacy, &stage)?;
    if let Err(error) = replace(&stage, &dest) {
        let _ = std::fs::remove_file(&stage);
        return Err(error);
    }
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
                fn ReplaceFileW(dst: *const u16, src: *const u16,
                    backup: *const u16, flags: u32, exclude: *mut std::ffi::c_void,
                    reserved: *mut std::ffi::c_void) -> i32;
            }
            let dest: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
            let src: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
            if ReplaceFileW(dest.as_ptr(), src.as_ptr(), std::ptr::null(), 0,
                std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(source, target)?;
    }
    Ok(())
}
fn download(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = client.get(url).send()?.error_for_status()?;
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "NGX metadata/download too large");
    Ok(bytes)
}
/// Check signed channel data at startup. A new release is downloaded only if
/// the local DLL digest differs. The manifest is persisted after DLL verification.
pub fn refresh(root: &Path, client: &reqwest::blocking::Client, legacy_hash: Option<&str>) -> Result<()> {
    // Do not overwrite a verified, already installed signed adapter with an
    // older build merely because an EXE also carries a bootstrap digest.
    if verified(root, legacy_hash).is_err() {
        migrate_legacy(root, legacy_hash)?;
    }
    let Some(key) = public_key()? else {
        return verified(root, legacy_hash).context("No manifest signing key is configured in this build");
    };
    // Retain previously verified versions when offline or a manifest has not
    // been published. Never load bytes from unsigned or tampered metadata.
    let cached = cached(root, &key).ok().flatten();
    let incoming = match download(client, source(), MAX_MANIFEST) {
        Ok(data) => match validate_manifest(&data, &key) {
            Ok(value) => Some((data, value)),
            Err(error) => {
                if verified(root, legacy_hash).is_ok() { return Ok(()); }
                return Err(error);
            },
        },
        Err(error) => {
            if verified(root, legacy_hash).is_ok() { return Ok(()); }
            return Err(error);
        },
    };
    let (manifest_bytes, manifest) = incoming.context("No signed NGX manifest")?;
    if cached.as_ref().is_some_and(|old| old.sequence > manifest.sequence) {
        return verified(root, legacy_hash);
    }
    let target = root.join(DLL_NAME);
    if digest(&target, &manifest.sha256, manifest.size).is_err() {
        let bytes = download(client, &manifest.url, MAX_BRIDGE)?;
        ensure!(bytes.len() as u64 == manifest.size, "Downloaded NGX bridge size mismatch");
        let stage = root.join("bridge-download.tmp");
        std::fs::write(&stage, &bytes)?;
        if let Err(error) = digest(&stage, &manifest.sha256, manifest.size)
            .and_then(|_| replace(&stage, &target)) {
            let _ = std::fs::remove_file(&stage);
            if verified(root, legacy_hash).is_ok() { return Ok(()); }
            return Err(error);
        }
    }
    // A manifest never becomes trusted until the corresponding DLL matches.
    digest(&target, &manifest.sha256, manifest.size)?;
    let stage = root.join("bridge-manifest.tmp");
    std::fs::write(&stage, &manifest_bytes)?;
    replace(&stage, &root.join(MANIFEST_NAME))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    #[test]
    fn envelope_requires_signature_matching_exact_payload() {
        let signer = Ed25519KeyPair::from_seed_unchecked(&[9;32]).unwrap();
        let payload = br#"{"schema":1,"channel":"dev","sequence":1,"version":"1.2.3","abi":3,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":326144,"url":"https://github.com/vampywiz17/EvoParseq/releases/download/v0.9.9-dev.2/EvoParseq-NGX-Bridge.dll"}"#;
        let encoded = serde_json::json!({
            "payload":base64::engine::general_purpose::STANDARD.encode(payload),
            "signature":base64::engine::general_purpose::STANDARD.encode(signer.sign(payload).as_ref())
        });
        let bytes = serde_json::to_vec(&encoded).unwrap();
        let key = signer.public_key().as_ref();
        assert!(validate_manifest(&bytes, key).is_ok());
        let mut bad = encoded;
        bad["payload"] = serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b"forged"));
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
        bad.abi=4;
        assert!(validate_payload(&bad).is_err());
    }
}
