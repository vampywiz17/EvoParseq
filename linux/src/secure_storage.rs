//! Linux secret protection: per-application XDG Secret portal key, HKDF and
//! authenticated AES-256-GCM. No plaintext key or credential fallback file.
use anyhow::{Context, Result, bail, ensure};
use ring::{aead, hkdf};
use std::{
    io::Read,
    os::unix::net::UnixStream,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use zeroize::Zeroizing;
const MAGIC: &[u8] = b"EVOPARSEQ-LINUX\x01";
const DOMAIN: &[u8] = b"io.github.vampywiz17.EvoParseq/settings/v1";
static KEY: OnceLock<Mutex<Option<Zeroizing<[u8; 32]>>>> = OnceLock::new();
fn portal_key() -> Result<Zeroizing<[u8; 32]>> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_read_timeout(Some(Duration::from_secs(5)))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), async {
            let portal = ashpd::desktop::secret::Secret::new().await?;
            portal
                .retrieve(&writer, Default::default())
                .await?
                .response()?;
            Ok::<_, ashpd::Error>(())
        })
        .await
        .context("Secure storage request timed out")?
        .context("The desktop Secret portal is unavailable or locked")
    })?;
    drop(writer);
    let mut secret = Zeroizing::new(Vec::new());
    reader.take(4097).read_to_end(&mut secret)?;
    ensure!(
        !secret.is_empty() && secret.len() <= 4096,
        "Invalid Secret portal key"
    );
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, DOMAIN);
    let prk = salt.extract(&secret);
    let mut key = Zeroizing::new([0u8; 32]);
    prk.expand(&[DOMAIN], hkdf::HKDF_SHA256)
        .map_err(|_| anyhow::anyhow!("Cannot derive storage key"))?
        .fill(key.as_mut())
        .map_err(|_| anyhow::anyhow!("Cannot derive storage key"))?;
    Ok(key)
}
fn cipher(key: &[u8; 32]) -> Result<aead::LessSafeKey> {
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| anyhow::anyhow!("Cannot create storage cipher"))?,
    ))
}
fn seal(input: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    ensure!(input.len() <= 32768, "Secret exceeds storage limit");
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("Cannot create secret nonce"))?;
    let mut payload = Zeroizing::new(input.to_vec());
    cipher(key)?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(DOMAIN),
            &mut *payload,
        )
        .map_err(|_| anyhow::anyhow!("Cannot protect secret"))?;
    let mut blob = Vec::with_capacity(MAGIC.len() + 12 + payload.len());
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&payload);
    Ok(blob)
}
fn open(input: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    ensure!(
        input.starts_with(MAGIC),
        "Saved secret belongs to another platform or protection format"
    );
    ensure!(
        input.len() >= MAGIC.len() + 12 + 16 && input.len() <= 65536,
        "Invalid protected secret"
    );
    let nonce: [u8; 12] = input[MAGIC.len()..MAGIC.len() + 12].try_into()?;
    let mut payload = Zeroizing::new(input[MAGIC.len() + 12..].to_vec());
    let plain = cipher(key)?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(DOMAIN),
            &mut *payload,
        )
        .map_err(|_| anyhow::anyhow!("Saved secret cannot be decrypted for this Linux user"))?;
    Ok(plain.to_vec())
}
pub fn protect(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    #[cfg(test)]
    if TEST_STORAGE.with(|active| active.get()) {
        return if encrypt {
            seal(input, &[7; 32])
        } else {
            open(input, &[7; 32])
        };
    }
    // Called only on the existing account/settings worker, never the UI thread.
    if !encrypt && !input.starts_with(MAGIC) {
        bail!("Saved secret belongs to another platform or protection format");
    }
    let mut cache = KEY
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if cache.is_none() {
        *cache = Some(portal_key()?);
    }
    let key = cache.as_ref().unwrap();
    if encrypt {
        seal(input, key)
    } else {
        open(input, key)
    }
}
// Isolated fixture key, never installed in the production portal cache.
#[cfg(test)]
thread_local! { static TEST_STORAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(test)]
pub(crate) fn fixture<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_STORAGE.with(|active| active.set(self.0));
        }
    }
    let _reset = Reset(TEST_STORAGE.with(|active| active.replace(true)));
    run()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_encrypted_session_rejects_tampering_and_another_user_key() {
        let key = [7u8; 32];
        let plain = b"synthetic-session-never-a-password";
        let blob = seal(plain, &key).unwrap();
        assert!(!blob.windows(plain.len()).any(|w| w == plain));
        assert_eq!(open(&blob, &key).unwrap(), plain);
        assert!(open(&blob, &[8; 32]).is_err());
        let mut changed = blob.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert!(open(&changed, &key).is_err());
        assert_ne!(blob, seal(plain, &key).unwrap());
        assert!(open(b"Windows-DPAPI-fixture", &key).is_err());
    }
}
