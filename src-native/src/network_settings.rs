//! Shared desktop network form buffers and normalization. Secrets are replacement
//! buffers only: saved secrets are never decrypted into widgets.
#[cfg(target_os = "linux")]
use crate::connection_settings::Manager;
use crate::connection_settings::Settings;
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use zeroize::Zeroizing;

pub const NOTICE: &str = "ICE chooses the route automatically unless Force TURN relay is enabled.\nChanges apply to your next connection.";
#[cfg(target_os = "linux")]
pub const SAVING: &str = "Saving settings...";
pub const SAVED: &str = "Settings saved. They apply on your next connection.";

#[cfg(target_os = "linux")]
pub struct Draft {
    pub values: Settings,
    pub stun_urls: String,
    pub turn_urls: String,
    pub ttl: String,
    pub audio_min: String,
    pub audio_max: String,
    pub password: Zeroizing<String>,
    pub token: Zeroizing<String>,
    pub forget_password: bool,
    pub forget_token: bool,
}
#[cfg(target_os = "linux")]
impl Draft {
    pub fn new(values: Settings) -> Self {
        Self {
            stun_urls: values.stun_urls.join("\n"),
            turn_urls: values.turn_urls.join("\n"),
            ttl: values.ttl.to_string(),
            audio_min: values.audio_min_buffer_ms.to_string(),
            audio_max: values.audio_max_buffer_ms.to_string(),
            values,
            password: Zeroizing::new(String::new()),
            token: Zeroizing::new(String::new()),
            forget_password: false,
            forget_token: false,
        }
    }
    // Storage protection may prompt the desktop keyring; run outside the UI and
    // outside a Tokio runtime. No network request is made while saving.
    pub fn save(&mut self, manager: &Manager) -> Result<()> {
        let mut candidate = self.values.clone();
        apply_fields(&mut candidate, &self.stun_urls, &self.turn_urls, &self.ttl)?;
        apply_audio_fields(&mut candidate, &self.audio_min, &self.audio_max)?;
        manager.save(
            candidate,
            &self.password,
            &self.token,
            self.forget_password,
            self.forget_token,
        )?;
        *self = Self::new(manager.view().0);
        Ok(())
    }
}
pub fn apply_fields(settings: &mut Settings, stun: &str, turn: &str, ttl: &str) -> Result<()> {
    settings.stun_urls = urls(stun);
    settings.turn_urls = urls(turn);
    settings.custom_username = settings.custom_username.trim().into();
    settings.turn_key_id = settings.turn_key_id.trim().into();
    settings.ttl = ttl.parse().context("Enter a valid credential lifetime")?;
    Ok(())
}
pub fn apply_audio_fields(settings: &mut Settings, minimum: &str, maximum: &str) -> Result<()> {
    settings.audio_min_buffer_ms = minimum
        .parse()
        .context("Enter an audio minimum in milliseconds")?;
    settings.audio_max_buffer_ms = maximum
        .parse()
        .context("Enter an audio maximum in milliseconds")?;
    Ok(())
}
pub fn urls(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(|u| {
            if let Some((scheme, rest)) = u.split_once(':') {
                format!("{}:{rest}", scheme.to_ascii_lowercase())
            } else {
                u.into()
            }
        })
        .collect()
}
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::connection_settings::RouteMode;
    #[test]
    fn linux_network_form_preserves_dormant_preferences_and_secret_replacements() {
        let mut draft = Draft::new(Settings::default());
        draft.values.encrypted_custom_password = "protected-password".into();
        draft.values.encrypted_api_token = "protected-token".into();
        draft.values.prefer_hevc = true;
        draft.stun_urls = " STUN:example.invalid:3478 \r\n\n".into();
        draft.turn_urls = " TURNS:turn.example.invalid:443?transport=tcp ".into();
        apply_fields(
            &mut draft.values,
            &draft.stun_urls,
            &draft.turn_urls,
            &draft.ttl,
        )
        .unwrap();
        assert_eq!(draft.values.stun_urls, ["stun:example.invalid:3478"]);
        assert_eq!(
            draft.values.turn_urls,
            ["turns:turn.example.invalid:443?transport=tcp"]
        );
        for route in RouteMode::ALL {
            route.apply(&mut draft.values);
            assert_eq!(RouteMode::current(&draft.values), route);
            assert!(!(draft.values.stun_only && draft.values.force_relay));
        }
        assert_eq!(draft.values.encrypted_custom_password, "protected-password");
        assert_eq!(draft.values.encrypted_api_token, "protected-token");
        assert!(draft.values.prefer_hevc && draft.password.is_empty() && draft.token.is_empty());
        assert!(apply_fields(&mut draft.values, "", "", "invalid").is_err());
    }
}
