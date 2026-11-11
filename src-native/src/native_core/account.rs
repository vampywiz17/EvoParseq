//! Rust adaptation of lowlat scripts/kessel-login.py at e548d48.
//! Only account authentication: no daemon, installer, host or extra runtime.
//! MIT attribution: src-native/crates/bud/vendor/lowlat-client/LICENSE.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

pub struct Login {
    pub email: String,
    pub password: zeroize::Zeroizing<String>,
    pub tfa: zeroize::Zeroizing<String>,
}

// Deliberately no Debug/Serialize: neither token nor password belongs in logs.
pub struct Session {
    token: zeroize::Zeroizing<String>,
    revoke_on_drop: std::sync::atomic::AtomicBool,
}
impl Session {
    pub fn from_token(token: &str) -> Result<Self> {
        ensure!(
            !token.is_empty() && token.len() <= 4096 && !token.chars().any(char::is_control),
            "Invalid account session"
        );
        Ok(Self {
            token: zeroize::Zeroizing::new(token.into()),
            revoke_on_drop: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn remembered(&self) {
        self.revoke_on_drop
            .store(false, std::sync::atomic::Ordering::Release);
    }
    pub fn revoke(&self) {
        self.remembered();
        // Same DELETE contract as lowlat's --logout. Never log a server body.
        let result = reqwest::blocking::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(3))
            .build()
            .and_then(|client| {
                client
                    .delete("https://kessel-api.parsec.app/auth/sessions")
                    .bearer_auth(self.token())
                    .send()
            });
        if !result.is_ok_and(|response| response.status().is_success()) {
            crate::event_log::record("WARN", "native_account_revoke_failed", json!({}));
        }
    }
}

fn body(login: &Login) -> Value {
    // Same service scopes and optional second-factor contract as lowlat.
    let mut body = json!({"email":login.email.trim(), "password":login.password.as_str(),
        "session_scopes":["ws.host", "ws.client", "api.writer"]});
    let tfa = login.tfa.trim();
    if !tfa.is_empty() {
        let digits = zeroize::Zeroizing::new(
            tfa.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
        );
        let code = if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()) {
            digits.as_str()
        } else {
            tfa
        };
        body["tfa"] = json!(code);
    }
    body
}

fn accepted(status: u16, value: Value) -> Result<Session> {
    if status == 403
        && (value["error"] == "ip_unverified"
            || value["codes"].as_array().is_some_and(|codes| {
                codes
                    .iter()
                    .any(|code| code == "ip_unverified" || code["type"] == "ip_unverified")
            }))
    {
        anyhow::bail!("Confirm the sign-in email from Parsec, then try again.");
    }
    match status {
        200 | 201 => {}
        400 => {
            let fields: Vec<_> = ["email", "password", "tfa", "session_scopes"]
                .into_iter()
                .filter(|field| {
                    value["codes"].as_array().is_some_and(|codes| {
                        codes.iter().any(|code| {
                            code["type"] == "request_body_invalid"
                                && code["values"].as_array().is_some_and(|values| {
                                    values.iter().any(|value| value.as_str() == Some(*field))
                                })
                        })
                    })
                })
                .collect();
            if fields == ["tfa"] {
                anyhow::bail!("Parsec rejected the two-factor code format (HTTP 400). Enter the six digits from your authenticator app.");
            }
            if !fields.is_empty() {
                anyhow::bail!(
                    "Parsec rejected these sign-in fields (HTTP 400): {}.",
                    fields.join(", ")
                );
            }
            anyhow::bail!("Sign-in refused (HTTP 400). Parsec did not identify an invalid field.");
        }
        401 => anyhow::bail!(
            "Sign-in refused (HTTP 401). Check your email, password and current two-factor code."
        ),
        403 => {
            anyhow::bail!("Sign-in blocked (HTTP 403). Check for a confirmation email from Parsec.")
        }
        429 => anyhow::bail!("Too many sign-in attempts (HTTP 429). Wait before retrying."),
        500..=599 => {
            anyhow::bail!("Parsec sign-in service returned HTTP {status}. Try again later.")
        }
        _ => anyhow::bail!("Sign-in refused (HTTP {status})."),
    }
    let token = value["session_id"]
        .as_str()
        .context("No session returned by Parsec")?;
    Session::from_token(token)
}

pub fn login(login: &Login) -> Result<Session> {
    ensure!(
        !login.email.trim().is_empty() && !login.password.is_empty(),
        "Email and password are required."
    );
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let response = client
        .post("https://kessel-api.parsec.app/v2/auth")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .body(body(login).to_string())
        .send()
        .map_err(|_| anyhow::anyhow!("Could not reach Parsec sign-in securely."))?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response
        .take(131073)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Could not read the sign-in response."))?;
    ensure!(bytes.len() <= 131072, "Sign-in response exceeds limit");
    let value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) if !matches!(status, 200 | 201) => Value::Null,
        Err(_) => anyhow::bail!("Invalid sign-in response."),
    };
    let session = accepted(status, value)?;
    session
        .revoke_on_drop
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_created_sessions_and_rejects_missing_tokens_without_echoing_secrets() {
        for status in [200, 201] {
            assert_eq!(
                accepted(status, json!({"session_id":"fixture"}))
                    .unwrap()
                    .token(),
                "fixture"
            );
        }
        for status in [302, 401, 403, 500] {
            let error = accepted(
                status,
                json!({"error":"secret-sentinel", "session_id":"secret-sentinel"}),
            )
            .err()
            .unwrap();
            assert!(!error.to_string().contains("secret-sentinel"));
            assert!(error.to_string().contains(&format!("HTTP {status}")));
        }
        assert!(accepted(201, json!({})).is_err());
    }
    #[test]
    fn lowlat_auth_shape_and_email_confirmation_are_preserved() {
        let mut login = Login {
            email: " user@example.invalid ".into(),
            password: zeroize::Zeroizing::new("fixture".into()),
            tfa: zeroize::Zeroizing::new(String::new()),
        };
        assert!(body(&login).get("tfa").is_none());
        assert_eq!(body(&login)["email"], "user@example.invalid");
        login.tfa = zeroize::Zeroizing::new("012 345".into());
        assert_eq!(body(&login)["tfa"], "012345");
        let e = accepted(403, json!({"codes":[{"type":"ip_unverified"}]}))
            .err()
            .unwrap();
        assert!(e.to_string().contains("Confirm"));
        let invalid_code = accepted(
            400,
            json!({"codes":[{"type":"request_body_invalid","values":["tfa", "secret-sentinel"]}]}),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(invalid_code.contains("two-factor code format"));
        assert!(!invalid_code.contains("secret-sentinel"));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub peer: String,
    pub name: String,
}
#[derive(Debug)]
pub struct ExpiredSession;
impl std::fmt::Display for ExpiredSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Your saved sign-in has expired. Sign in again.")
    }
}
impl std::error::Error for ExpiredSession {}

pub fn hosts(session: &Session) -> Result<Vec<Host>> {
    // Audited pinned core's desktop discovery endpoint; no credentials in URL.
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?;
    let response = client
        .get("https://kessel-api.parsec.app/v2/hosts?mode=desktop")
        .bearer_auth(session.token())
        .send()
        .map_err(|_| anyhow::anyhow!("Could not securely refresh the computer list."))?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(ExpiredSession.into());
    }
    ensure!(
        response.status().is_success(),
        "Computer list request refused (HTTP {}).",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Could not read the computer list."))?;
    ensure!(bytes.len() <= 1024 * 1024, "Computer list exceeds limit");
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid computer list response."))?;
    parse_hosts(&value)
}
fn parse_hosts(value: &Value) -> Result<Vec<Host>> {
    let data = value["data"]
        .as_array()
        .context("Computer list format is not supported; use a peer ID instead.")?;
    ensure!(
        data.len() <= 1024,
        "Too many computers in discovery response"
    );
    let mut hosts = Vec::new();
    for host in data {
        let Some(peer) = host["peer_id"].as_str() else {
            continue;
        };
        if peer.is_empty() || peer.len() > 256 || peer.chars().any(char::is_control) {
            continue;
        }
        let name = host["name"].as_str().unwrap_or(peer);
        let name: String = name.chars().filter(|c| !c.is_control()).take(128).collect();
        if !hosts.iter().any(|h: &Host| h.peer == peer) {
            hosts.push(Host {
                peer: peer.into(),
                name,
            });
        }
    }
    ensure!(
        data.is_empty() || !hosts.is_empty(),
        "Computer identifiers were not recognized; use a peer ID instead."
    );
    hosts.sort_by_cached_key(|h| h.name.to_lowercase());
    Ok(hosts)
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn bounded_discovery_uses_host_identity_not_account_identity() {
        let h = parse_hosts(&json!({"data":[{"peer_id":"peer","name":"My PC","user_id":123},{"peer_id":"peer"},{"peer_id":"other","name":"Laptop"}]})).unwrap();
        assert_eq!(h.len(), 2);
        assert!(h.iter().any(|h| h.peer == "peer"));
        assert!(parse_hosts(&json!({"data":[{"user_id":123}]})).is_err());
        assert!(parse_hosts(&json!({"session_id":"secret"})).is_err());
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self
            .revoke_on_drop
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.revoke();
        }
    }
}
