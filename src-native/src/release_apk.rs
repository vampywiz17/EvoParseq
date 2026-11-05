//! Platform-neutral GitHub APK discovery, version policy and bounded download.
//! Android owns package/signature checks and the system installation UI.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};
const REPO: &str = "https://github.com/vampywiz17/EvoParseq";
const API: &str = "https://api.github.com/repos/vampywiz17/EvoParseq/releases?per_page=100";
const LIMIT: u64 = 128 * 1024 * 1024;
#[derive(Clone, Serialize)]
pub struct Candidate {
    pub version: String,
    pub notes: String,
    url: String,
    size: u64,
    digest: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}
fn version(text: &str) -> Result<([u64; 3], Option<u64>)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let (base, dev) = match text.split_once("-dev.") {
        Some((base, n)) => (base, Some(number(n)?)),
        None => (text, None),
    };
    let parts: Vec<_> = base.split('.').collect();
    ensure!(
        parts.len() == 3 && dev != Some(0),
        "Unsupported release version"
    );
    Ok((
        [number(parts[0])?, number(parts[1])?, number(parts[2])?],
        dev,
    ))
}
fn number(s: &str) -> Result<u64> {
    ensure!(
        !s.is_empty()
            && s.bytes().all(|b| b.is_ascii_digit())
            && (s.len() == 1 || !s.starts_with('0')),
        "Invalid release version"
    );
    Ok(s.parse()?)
}
type VersionKey = ([u64; 3], bool, u64);
fn key(v: ([u64; 3], Option<u64>)) -> VersionKey {
    (v.0, v.1.is_none(), v.1.unwrap_or(0))
}
use crate::release_http::hash;

fn select(bytes: &[u8], installed: &str, pre: bool) -> Result<Option<Candidate>> {
    let installed = key(version(installed)?);
    let releases: Vec<Release> = serde_json::from_slice(bytes)?;
    let mut best: Option<(VersionKey, Candidate)> = None;
    for r in releases {
        if r.draft || (r.prerelease && !pre) {
            continue;
        }
        let Ok(v) = version(&r.tag_name) else {
            continue;
        };
        if v.1.is_some() && (!pre || !r.prerelease) {
            continue;
        }
        let k = key(v);
        if k <= installed || best.as_ref().is_some_and(|(old, _)| *old >= k) {
            continue;
        }
        let mut assets = r
            .assets
            .iter()
            .filter(|a| a.name == "EvoParseq-TV.apk" || a.name == "EvoParseq-TV-preview.apk");
        let Some(a) = assets.next() else {
            continue;
        };
        ensure!(
            assets.next().is_none(),
            "Release has ambiguous Android APKs"
        );
        ensure!(
            a.browser_download_url == format!("{REPO}/releases/download/{}/{}", r.tag_name, a.name),
            "Untrusted APK URL"
        );
        ensure!(a.size > 0 && a.size <= LIMIT, "Invalid APK size");
        let digest = a
            .digest
            .as_deref()
            .context("Release APK has no SHA-256 digest")?;
        hash(digest)?;
        best = Some((
            k,
            Candidate {
                version: r.tag_name,
                notes: r.body.unwrap_or_default().chars().take(12000).collect(),
                url: a.browser_download_url.clone(),
                size: a.size,
                digest: digest.into(),
            },
        ));
    }
    Ok(best.map(|(_, v)| v))
}
pub fn check(installed: &str, pre: bool) -> Result<Option<Candidate>> {
    select(&crate::release_http::metadata(API)?, installed, pre)
}
pub fn download(candidate: &Candidate, destination: &Path) -> Result<()> {
    let result = (|| -> Result<()> {
        let response = crate::release_http::client()?
            .get(&candidate.url)
            .send()?
            .error_for_status()?;
        if let Some(size) = response.content_length() {
            ensure!(size == candidate.size, "APK size changed");
        }
        let mut file = std::fs::File::create(destination)?;
        copy_verified(response, &mut file, candidate)?;
        file.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}
fn copy_verified(reader: impl Read, writer: &mut impl Write, candidate: &Candidate) -> Result<()> {
    crate::release_http::copy_verified(reader, writer, candidate.size, &candidate.digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str, pre: bool) -> serde_json::Value {
        serde_json::json!({"tag_name":tag,"draft":false,"prerelease":pre,"body":"Changes","assets":[{"name":"EvoParseq-TV.apk","size":12,"digest":format!("sha256:{}","ab".repeat(32)),"browser_download_url":format!("{REPO}/releases/download/{tag}/EvoParseq-TV.apk")} ]})
    }
    #[test]
    fn apk_download_requires_exact_length_and_digest() {
        let bytes = b"signed APK fixture";
        let candidate = Candidate {
            version: "v0.9.6".into(),
            notes: String::new(),
            url: String::new(),
            size: bytes.len() as u64,
            digest: format!("sha256:{:x}", Sha256::digest(bytes)),
        };
        let mut output = Vec::new();
        copy_verified(&bytes[..], &mut output, &candidate).unwrap();
        assert_eq!(output, bytes);
        assert!(copy_verified(&bytes[..bytes.len() - 1], &mut Vec::new(), &candidate).is_err());
        assert!(copy_verified(&[0u8; 64][..], &mut Vec::new(), &candidate).is_err());
        let mut wrong = candidate.clone();
        wrong.digest = format!("sha256:{}", "00".repeat(32));
        assert!(copy_verified(&bytes[..], &mut Vec::new(), &wrong).is_err());
    }
    #[test]
    fn apk_channels_and_versions_are_ordered() {
        let data = serde_json::to_vec(&vec![
            release("v0.9.7-dev.1", true),
            release("v0.9.6", false),
            release("v0.9.5", false),
        ])
        .unwrap();
        assert_eq!(
            select(&data, "0.9.6-dev.1", false)
                .unwrap()
                .unwrap()
                .version,
            "v0.9.6"
        );
        assert_eq!(
            select(&data, "0.9.6-dev.1", true).unwrap().unwrap().version,
            "v0.9.7-dev.1"
        );
        assert!(select(&data, "0.9.7", true).unwrap().is_none());
        for bad in ["0.9.6/evil", "0.09.6", "0.9.6-dev.0", "0.9.6+bad"] {
            assert!(version(bad).is_err());
        }
    }
    #[test]
    fn apk_rejects_untrusted_or_unverifiable_assets() {
        for field in ["digest", "browser_download_url", "size"] {
            let mut r = release("v0.9.6", false);
            r["assets"][0][field] = match field {
                "size" => serde_json::json!(LIMIT + 1),
                _ => serde_json::json!("bad"),
            };
            assert!(select(&serde_json::to_vec(&vec![r]).unwrap(), "0.9.5", false).is_err());
        }
        let mut r = release("v0.9.6", false);
        r["assets"] = serde_json::json!([]);
        assert!(
            select(&serde_json::to_vec(&vec![r]).unwrap(), "0.9.5", false)
                .unwrap()
                .is_none()
        );
        assert!(select(b"garbage", "0.9.5", false).is_err());
    }
}
