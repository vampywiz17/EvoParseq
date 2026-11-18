//! Independent experimental Phaze client. No Phaze native code or libraries.
//! Wire behavior was observed from an authorized local host. Decode/presentation
//! belong to the embedding application. Unknown media/control formats are ignored.
mod audio;
mod bandwidth;
mod control;
mod cursor;
mod datagram;
mod input;
mod transport;
pub mod turn;
mod wire;
use anyhow::{bail, ensure, Context, Result};
pub use audio::AudioCodec;
pub use control::{ClientControl, PeerState, MAX_CLIPBOARD_BYTES};
pub use cursor::{CursorFormat, CursorImage};
pub use datagram::Datagram;
use futures_util::{SinkExt, StreamExt};
pub use input::{Input, Keyboard};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};
pub use wire::{Output, VideoConfig};
use zeroize::{Zeroize, Zeroizing};
const API: &str = "https://api.phaze.app";

/// Deliberately no Debug/Serialize; tokens must not enter diagnostic output.
pub struct Session {
    token: Zeroizing<String>,
}
impl Session {
    pub fn from_token(token: &str) -> Result<Self> {
        ensure!(
            !token.is_empty() && token.len() <= 4096 && !token.chars().any(char::is_control),
            "Invalid Phaze session"
        );
        Ok(Self {
            token: Zeroizing::new(token.into()),
        })
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    pub async fn revoke(&self) -> Result<()> {
        let r = http()?
            .delete(format!("{API}/v2/auth/session"))
            .bearer_auth(self.token())
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Could not revoke Phaze session securely"))?;
        ensure!(
            r.status().is_success() || r.status().as_u16() == 401,
            "Phaze session revocation refused"
        );
        Ok(())
    }
}
fn tls() -> Result<Arc<rustls::ClientConfig>> {
    tls_with_roots(&[])
}
fn tls_with_roots(extra: &[Vec<u8>]) -> Result<Arc<rustls::ClientConfig>> {
    let mut roots =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots.add_parsable_certificates(
        extra
            .iter()
            .cloned()
            .map(rustls::pki_types::CertificateDer::from),
    );
    let config =
        rustls::ClientConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
            .with_safe_default_protocol_versions()
            .context("Phaze TLS initialization failed")?
            .with_root_certificates(roots)
            .with_no_client_auth();
    Ok(Arc::new(config))
}
fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .use_preconfigured_tls((*tls()?).clone())
        .build()
        .context("Phaze HTTPS initialization failed")
}
async fn json_response(mut response: reqwest::Response) -> Result<Value> {
    // Streaming bound also covers chunked bodies without Content-Length.
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Could not read Phaze response"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= 1024 * 1024,
            "Phaze response exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid Phaze response"))
}
pub async fn login(email: &str, password: &str, tfa: &str) -> Result<Session> {
    ensure!(
        !email.trim().is_empty() && !password.is_empty(),
        "Email and password are required."
    );
    let mut body = json!({"email":email.trim(),"password":password});
    if !tfa.trim().is_empty() {
        body["tfa_code"] = json!(tfa.trim());
    }
    let result = http()?
        .post(format!("{API}/v2/auth/session"))
        .json(&body)
        .send()
        .await;
    for name in ["email", "password", "tfa_code"] {
        if let Some(Value::String(s)) = body.get_mut(name) {
            s.zeroize();
        }
    }
    let r = result.map_err(|_| anyhow::anyhow!("Could not reach Phaze sign-in securely"))?;
    ensure!(
        r.status().is_success(),
        "Phaze sign-in refused (HTTP {}).",
        r.status().as_u16()
    );
    let mut auth = json_response(r).await?;
    let Value::String(token) = auth["data"]["token"].take() else {
        bail!("No Phaze session returned")
    };
    let token = Zeroizing::new(token);
    Session::from_token(&token)
}
#[derive(Debug)]
pub struct ExpiredSession;
impl std::fmt::Display for ExpiredSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Your Phaze sign-in has expired. Sign in again.")
    }
}
impl std::error::Error for ExpiredSession {}
#[derive(Deserialize)]
pub struct Host {
    pub peer_id: String,
    pub name: String,
    pub is_online: bool,
}
pub async fn hosts(session: &Session) -> Result<Vec<Host>> {
    let r = http()?
        .get(format!("{API}/v2/hosts?show_offline=true&limit=100"))
        .bearer_auth(session.token())
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Could not refresh Phaze computers securely"))?;
    if matches!(r.status().as_u16(), 401 | 403) {
        return Err(ExpiredSession.into());
    }
    ensure!(
        r.status().is_success(),
        "Phaze computer list refused (HTTP {}).",
        r.status().as_u16()
    );
    let v = json_response(r).await?;
    let hosts: Vec<Host> = serde_json::from_value(v["data"].clone())
        .map_err(|_| anyhow::anyhow!("Invalid Phaze computer list"))?;
    ensure!(hosts.len() <= 1024, "Phaze computer list exceeds limit");
    Ok(hosts
        .into_iter()
        .filter(|h| h.is_online && valid_peer(&h.peer_id))
        .collect())
}
fn valid_peer(peer: &str) -> bool {
    !peer.is_empty() && peer.len() <= 256 && !peer.chars().any(char::is_control)
}
#[derive(Deserialize)]
struct Candidate {
    addr: String,
    port: u16,
}
#[derive(Deserialize)]
struct Answer {
    ufrag: String,
    pwd: Zeroizing<String>,
    cert_hash: String,
    cid: u32,
    candidates: Vec<Candidate>,
}
fn random_hex(n: usize) -> Result<String> {
    let mut b = vec![0; n];
    getrandom::fill(&mut b).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    Ok(b.iter().map(|v| format!("{v:02x}")).collect())
}
/// Called directly by the network consumer. No decoded frames or extra queue.
pub trait Sink {
    /// Request stereo PCM instead of AAC-LC. Applies at connection startup.
    fn raw_audio(&self) -> bool {
        false
    }
    fn video_config(&self) -> VideoConfig {
        VideoConfig::default()
    }
    /// A validated stream update, optionally targeting another advertised output.
    fn stream_request(&mut self) -> Option<(u32, VideoConfig)> {
        None
    }
    /// The carrier accepted a stream request for sending.
    fn stream_request_sent(&mut self, _output: u32, _config: VideoConfig) {}
    fn cursor_visibility(&mut self, _visible: bool) {}
    fn cursor_image(&mut self, _image: CursorImage<'_>) {}
    fn bandwidth_feedback_started(&mut self) {}
    fn peer_state(&mut self, _local_id: u32, _state: PeerState) {}
    fn host_flags(&mut self, _flags: u32) {}
    fn controller(&mut self, _id: u32) {}
    fn clipboard_text(&mut self, _text: &str) {}
    fn control_sent(&mut self, _control: &ClientControl, _sent: bool) {}
    /// DTLS carrier bytes, excluding TURN encapsulation and handshake traffic.
    fn traffic(&mut self, _received: u64, _sent: u64) {}
    /// Local reliable-command ACK round trip, including peer ACK scheduling.
    /// None means no unambiguous sample in the last ten seconds.
    fn rtt(&mut self, _elapsed: Option<Duration>) {}
    fn stage(&mut self, stage: &'static str);
    fn outputs(&mut self, outputs: &[Output]);
    fn video(&mut self, encoded: &[u8], reset: bool) -> Result<()>;
    /// Audio payload bytes are measured before decoding; headers are excluded.
    fn audio(&mut self, pcm: &[i16], codec: AudioCodec, payload_bytes: usize) -> Result<()>;
    /// Drain the application's existing event queue. False closes the session.
    fn poll(
        &mut self,
        output: Option<u32>,
        input: &mut Vec<Input>,
        controls: &mut Vec<ClientControl>,
    ) -> Result<bool>;
}
/// Experimental direct UDP session. Video, PCM audio and input reuse the
/// embedding client's backends. No automatic Parsec fallback occurs.
pub async fn connect(session: &Session, peer: &str, sink: &mut impl Sink) -> Result<()> {
    ensure!(valid_peer(peer), "Invalid Phaze host peer ID");
    let socket = transport::local_socket()?;
    connect_via(session, peer, sink, socket).await
}
/// Explicit caller-owned UDP/TURN carrier. No direct fallback occurs here.
pub async fn connect_via(
    session: &Session,
    peer: &str,
    sink: &mut impl Sink,
    socket: impl Datagram,
) -> Result<()> {
    ensure!(valid_peer(peer), "Invalid Phaze host peer ID");
    let addresses = socket.candidates()?;
    ensure!(
        !addresses.is_empty()
            && addresses.len() <= 16
            && addresses
                .iter()
                .all(|a| a.port() != 0 && !a.ip().is_unspecified()),
        "Invalid Phaze advertised candidates"
    );
    let candidates: Vec<_> = addresses
        .iter()
        .map(|a| json!({"addr":a.ip().to_string(),"port":a.port(),"short_ttl":socket.candidate_short_ttl()}))
        .collect();
    let identity = transport::Identity::generate()?;
    let mut random = [0; 4];
    getrandom::fill(&mut random).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    let cid = (u32::from_le_bytes(random) & 0xfffff000) | 1;
    let ufrag = random_hex(7)?;
    let pwd = Zeroizing::new(random_hex(24)?);
    let attempt = random_hex(32)?;
    let mut url = reqwest::Url::parse("wss://ws.phaze.app/")?;
    url.query_pairs_mut()
        .append_pair("token", session.token())
        .append_pair(
            "subscriptions",
            "host_refresh,machine_access_invite_refresh",
        );
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|_| anyhow::anyhow!("Invalid Phaze signaling request"))?;
    let mut header = HeaderValue::from_str(&format!("Bearer {}", session.token()))
        .map_err(|_| anyhow::anyhow!("Invalid Phaze session header"))?;
    header.set_sensitive(true);
    req.headers_mut().insert("Authorization", header);
    sink.stage("connecting_signaling");
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let (mut ws, _) = tokio::time::timeout(
        Duration::from_secs(25),
        tokio_tungstenite::connect_async_tls_with_config(
            req,
            Some(config),
            false,
            Some(tokio_tungstenite::Connector::Rustls(tls()?)),
        ),
    )
    .await
    .context("Phaze signaling timeout")?
    .map_err(|_| anyhow::anyhow!("Phaze signaling TLS/WebSocket failed"))?;
    let ready = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let m = tokio::time::timeout_at(ready, ws.next())
            .await
            .context("Phaze ready timeout")?
            .context("Phaze signaling closed")?
            .map_err(|_| anyhow::anyhow!("Phaze signaling read failed"))?;
        if let Message::Text(t) = m {
            let v: Value =
                serde_json::from_str(&t).map_err(|_| anyhow::anyhow!("Invalid Phaze signaling"))?;
            if v["action"] == "ready" {
                break;
            }
        }
        ws.flush()
            .await
            .map_err(|_| anyhow::anyhow!("Phaze signaling flush failed"))?;
    }
    let mut offer = json!({"action":"offer","to":peer,"attempt_id":attempt,
        "data":{"ufrag":ufrag,"pwd":&*pwd,"cert_hash":identity.fingerprint,
            "cid":cid,"candidates":candidates,"control_type":"ui"}});
    let sent = ws.send(Message::Text(offer.to_string().into())).await;
    if let Some(Value::String(p)) = offer["data"].get_mut("pwd") {
        p.zeroize();
    }
    sent.map_err(|_| anyhow::anyhow!("Phaze offer failed"))?;
    sink.stage("waiting_host_approval");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let answer = loop {
        let m = tokio::time::timeout_at(deadline, ws.next())
            .await
            .context("Phaze host answer timeout")?
            .context("Phaze signaling closed")?
            .map_err(|_| anyhow::anyhow!("Phaze signaling read failed"))?;
        if let Message::Text(t) = m {
            let v: Value =
                serde_json::from_str(&t).map_err(|_| anyhow::anyhow!("Invalid Phaze signaling"))?;
            if v["attempt_id"].as_str() == Some(&attempt) {
                match v["action"].as_str() {
                    Some("answer") => {
                        break serde_json::from_value::<Answer>(v["data"].clone())
                            .map_err(|_| anyhow::anyhow!("Invalid Phaze host answer"))?
                    }
                    Some("decline") => bail!("Phaze host declined connection"),
                    _ => {}
                }
            }
        }
        ws.flush()
            .await
            .map_err(|_| anyhow::anyhow!("Phaze signaling flush failed"))?;
    };
    ensure!(
        answer.candidates.len() <= 32
            && answer.ufrag.len() <= 256
            && answer.pwd.len() <= 256
            && answer.cert_hash.len() == 64
            && answer.cert_hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid Phaze host credentials"
    );
    // A validated answer is not the host's start signal. This separate wire
    // action starts its normal-TTL peer checks, allowing NAT endpoint learning.
    ws.send(Message::Text(
        json!({"action":"sync","to":peer,"attempt_id":attempt,"data":null})
            .to_string()
            .into(),
    ))
    .await
    .map_err(|_| anyhow::anyhow!("Phaze host synchronization failed"))?;
    sink.stage("host_sync_sent");
    // Both futures are owned here: cancellation drops signaling and UDP together.
    let signaling = async {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                _=tick.tick()=>ws.send(Message::Ping(Vec::new().into())).await
                    .map_err(|_|anyhow::anyhow!("Phaze signaling keepalive failed"))?,
                m=ws.next()=>{
                    match m {
                        None|Some(Ok(Message::Close(_)))=>bail!("Phaze signaling closed"),
                        Some(Err(_))=>bail!("Phaze signaling read failed"),
                        _=>{}
                    }
                    ws.flush().await.map_err(|_|anyhow::anyhow!("Phaze signaling flush failed"))?;
                }
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        r=transport::run(socket,identity,cid,&ufrag,&pwd,answer,sink)=>r,
        r=signaling=>r,
    }
}
