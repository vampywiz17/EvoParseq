//! Shared bounded GitHub release transport and SHA-256 verification.
use anyhow::{bail, ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    time::Duration,
};
pub fn hash(s: &str) -> Result<&str> {
    let s = s
        .strip_prefix("sha256:")
        .context("Release asset has no SHA-256 digest")?;
    ensure!(
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid asset digest"
    );
    Ok(s)
}
pub fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("EvoParseq-Release-Updater")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() >= 5 {
                return a.error("Too many redirects");
            }
            let allowed = matches!(
                a.url().host_str(),
                Some(
                    "api.github.com"
                        | "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                )
            );
            if a.url().scheme() == "https" && allowed {
                a.follow()
            } else {
                a.error("Untrusted release redirect")
            }
        }))
        .build()?)
}

pub fn metadata(url: &str) -> Result<Vec<u8>> {
    let response = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?;
    let mut data = Vec::new();
    response.take(1024 * 1024 + 1).read_to_end(&mut data)?;
    ensure!(data.len() <= 1024 * 1024, "Release metadata too large");
    Ok(data)
}
pub fn copy_verified(
    reader: impl Read,
    writer: &mut impl Write,
    size: u64,
    expected: &str,
) -> Result<()> {
    let mut reader = reader.take(size + 1);
    let mut digest = Sha256::new();
    let mut count = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(count <= size, "Release asset exceeds release size");
        digest.update(&buffer[..n]);
        writer.write_all(&buffer[..n])?;
    }
    ensure!(count == size, "Truncated Release asset download");
    let actual = format!("{:x}", digest.finalize());
    if !actual.eq_ignore_ascii_case(hash(expected)?) {
        bail!("Release asset integrity verification failed");
    }
    Ok(())
}
