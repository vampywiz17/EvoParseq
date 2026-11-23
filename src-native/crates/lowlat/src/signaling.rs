//! Native LOWLAT account signaling, isolated from WebRTC SDP.
use crate::lowlat::Session;
use anyhow::{Result, anyhow, ensure};
use lowlat_client::{Event, Peer};
use lowlat_core::conn::Kind;
use lowlat_crypto::Credentials;
use serde_json::{Value, json};

pub struct Signaling {
    peer: String,
    attempt: String,
}

fn versions() -> Value {
    json!({"p2p":1,"bud":1,"init":1,"video":1,"audio":1,"control":1})
}

impl Signaling {
    pub fn new(peer: &str, attempt: &str) -> Result<Self> {
        ensure!(
            !peer.is_empty() && peer.len() <= 256,
            "invalid native peer id"
        );
        ensure!(
            !attempt.is_empty() && attempt.len() <= 256,
            "invalid native attempt id"
        );
        Ok(Self {
            peer: peer.to_owned(),
            attempt: attempt.to_owned(),
        })
    }

    fn message(&self, action: &str, data: Value) -> Value {
        json!({"version":1,"action":action,"payload":{
            "to":self.peer,"attempt_id":self.attempt,"data":data}})
    }

    /// Sensitive output: send to authenticated signaling only, never a log.
    pub fn offer(&self, ours: &Credentials) -> Value {
        let mut creds =
            json!({"ice_ufrag":ours.ufrag,"ice_pwd":ours.pwd,"fingerprint":ours.fingerprint});
        if !ours.aes256.is_empty() {
            creds["aes256"] = json!(ours.aes256);
        }
        let mut out = self.message(
            "offer",
            json!({"ver_data":1,"mode":1,"versions":versions(),"creds":creds}),
        );
        out["payload"]["secret"] = json!("");
        out
    }

    pub fn candidate(&self, event: &Event) -> Option<Value> {
        let (ip, port, lan, from_stun, sync) = match event {
            Event::Candidate {
                addr,
                lan,
                from_stun,
            } => (addr.ip().to_string(), addr.port(), *lan, *from_stun, false),
            Event::Ready => (String::new(), 0, false, false, true),
            _ => return None,
        };
        Some(self.message(
            "candex",
            json!({"ver_data":1,"versions":versions(),
            "ip":ip,"port":port,"lan":lan,"from_stun":from_stun,"sync":sync}),
        ))
    }

    /// Bounded, attempt-scoped signaling. Wrong-attempt events cannot mutate
    /// this session; malformed current-attempt credentials fail explicitly.
    pub fn receive(&self, text: &str, session: &mut Session) -> Result<()> {
        ensure!(
            text.len() <= 16 * 1024,
            "native signaling message too large"
        );
        let value: Value =
            serde_json::from_str(text).map_err(|_| anyhow!("invalid native signaling JSON"))?;
        let payload = &value["payload"];
        if payload["attempt_id"]
            .as_str()
            .is_some_and(|id| id != self.attempt)
        {
            return Ok(());
        }
        if value["action"] == "close" {
            anyhow::bail!("native signaling service closed the session");
        }
        if payload["attempt_id"].as_str() != Some(&self.attempt) {
            return Ok(());
        }
        let data = &payload["data"];
        match value["action"].as_str() {
            Some("answer_relay") => {
                ensure!(
                    payload["approved"] == true,
                    "host declined native LOWLAT offer"
                );
                ensure!(
                    data.get("mode").is_none_or(|mode| mode == 1),
                    "host selected a different transport"
                );
                let creds = &data["creds"];
                let field = |key: &str| -> Result<String> {
                    let text = creds[key]
                        .as_str()
                        .ok_or_else(|| anyhow!("native answer missing {key}"))?;
                    ensure!(text.len() <= 256, "native credential too long");
                    Ok(text.to_owned())
                };
                session.answer(
                    &self.attempt,
                    Peer {
                        ufrag: field("ice_ufrag")?,
                        pwd: field("ice_pwd")?,
                        fingerprint: field("fingerprint")?,
                        aes256: creds["aes256"].as_str().map(str::to_owned),
                    },
                )?;
            }
            Some("candex_relay") => {
                if data["sync"] == true {
                    session.ready(&self.attempt)?;
                } else {
                    let ip = data["ip"]
                        .as_str()
                        .ok_or_else(|| anyhow!("missing native candidate address"))?
                        .parse()?;
                    let port = data["port"]
                        .as_u64()
                        .and_then(|v| u16::try_from(v).ok())
                        .ok_or_else(|| anyhow!("invalid native candidate port"))?;
                    let kind = if data["lan"] == true {
                        Kind::Direct
                    } else if data["from_stun"] == true {
                        Kind::Reflexive
                    } else {
                        Kind::Wan
                    };
                    session.candidate(&self.attempt, std::net::SocketAddr::new(ip, port), kind)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lowlat::Route;
    #[test]
    fn native_offer_has_lowlat_mode_and_media_key() {
        let mut session = Session::external_video(false).unwrap();
        let ours = session
            .offer("test", Route::Direct { stun: vec![] })
            .unwrap();
        let signal = Signaling::new("peer", "test").unwrap();
        let offer = signal.offer(&ours);
        assert_eq!(offer["payload"]["data"]["mode"], 1);
        assert_eq!(offer["payload"]["data"]["creds"]["aes256"], ours.aes256);
        assert!(offer.to_string().find("webrtc").is_none());
        let ready = signal.candidate(&Event::Ready).unwrap();
        assert_eq!(ready["payload"]["data"]["sync"], true);
        assert_eq!(ready["payload"]["data"]["port"], 0);
    }
    #[test]
    fn wrong_attempt_and_invalid_endpoint_are_isolated() {
        let signal = Signaling::new("peer", "test").unwrap();
        let mut session = Session::external_video(false).unwrap();
        session
            .offer("test", Route::Direct { stun: vec![] })
            .unwrap();
        assert!(
            signal
                .receive(
                    r#"{"action":"answer_relay","payload":{"attempt_id":"stale"}}"#,
                    &mut session
                )
                .is_ok()
        );
        assert!(
            signal
                .receive(
                    r#"{"action":"close","payload":{"attempt_id":"stale"}}"#,
                    &mut session
                )
                .is_ok()
        );
        assert!(signal.receive(r#"{"action":"candex_relay","payload":{"attempt_id":"test","data":{"ip":"127.0.0.1","port":65536}}}"#,&mut session).is_err());
        assert!(signal.receive(&" ".repeat(16385), &mut session).is_err());
    }
}
