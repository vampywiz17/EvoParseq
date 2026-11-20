use serde::Deserialize;

const MAX_STATE_BYTES: usize = 128 * 1024;
pub const MAX_CLIPBOARD_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub enum ClientControl {
    Clipboard(String),
    SecureAttention,
    SetControl(u32),
    RemoveGuest(u32),
    RespondToOffer { attempt_id: String, accept: bool },
}

impl ClientControl {
    pub(crate) fn encode(&self) -> Option<Vec<u8>> {
        let (kind, body) = match self {
            Self::Clipboard(text) => {
                if text.len() > MAX_CLIPBOARD_BYTES || text.contains('\0') {
                    return None;
                }
                let mut body = text.as_bytes().to_vec();
                body.push(0);
                (0x201u32, body)
            }
            Self::SecureAttention => (0x600, b"{}".to_vec()),
            Self::SetControl(id) => (
                0x300,
                serde_json::to_vec(&serde_json::json!({"guest_id":id})).ok()?,
            ),
            Self::RemoveGuest(id) => (
                0x500,
                serde_json::to_vec(&serde_json::json!({"guest_id":id})).ok()?,
            ),
            Self::RespondToOffer { attempt_id, accept } => (
                0x400,
                serde_json::to_vec(&serde_json::json!({"attempt_id":attempt_id,"accept":accept}))
                    .ok()?,
            ),
        };
        let mut frame = Vec::with_capacity(5 + body.len());
        frame.push(14);
        frame.extend(kind.to_be_bytes());
        frame.extend(body);
        Some(frame)
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct PeerState {
    pub guests: Option<Vec<Guest>>,
    pub offers: Option<Vec<Offer>>,
}

#[derive(Clone, Deserialize)]
pub struct Guest {
    pub id: u32,
    pub user_name: String,
    pub has_control: bool,
    pub admin: bool,
}

#[derive(Clone, Deserialize)]
pub struct Offer {
    pub attempt_id: String,
    pub user_name: String,
}

#[derive(Deserialize)]
struct Removal {
    code: i32,
}

pub(crate) enum HostControl<'a> {
    Clipboard(&'a str),
    Peers(PeerState),
    Removed(i32),
}

pub(crate) fn decode(body: &[u8]) -> Option<HostControl<'_>> {
    if body.len() > MAX_STATE_BYTES {
        return None;
    }
    let (kind, data) = body.split_at_checked(4)?;
    match u32::from_be_bytes(kind.try_into().ok()?) {
        0x100 => {
            let state: PeerState = serde_json::from_slice(data).ok()?;
            if state
                .guests
                .as_ref()
                .is_some_and(|guests| guests.len() > 64)
                || state.offers.as_ref().is_some_and(|offers| {
                    offers.len() > 64
                        || offers.iter().any(|offer| {
                            offer.attempt_id.is_empty() || offer.attempt_id.len() > 256
                        })
                })
            {
                return None;
            }
            Some(HostControl::Peers(state))
        }
        0x201 => {
            let text = std::str::from_utf8(data.strip_suffix(&[0])?).ok()?;
            (!text.contains('\0')).then_some(HostControl::Clipboard(text))
        }
        0x900 => {
            let removal: Removal = serde_json::from_slice(data).ok()?;
            Some(HostControl::Removed(removal.code))
        }
        _ => None,
    }
}

#[cfg(test)]
#[test]
fn control_messages_keep_direction_flags_and_payload_formats() {
    assert_eq!(
        ClientControl::SecureAttention.encode().unwrap(),
        b"\x0e\0\0\x06\0{}"
    );
    let text = ClientControl::Clipboard("árvíz".into()).encode().unwrap();
    assert_eq!(&text[..5], &[14, 0, 0, 2, 1]);
    assert!(matches!(
        decode(&text[1..]),
        Some(HostControl::Clipboard("árvíz"))
    ));
    assert!(decode(&text[1..text.len() - 1]).is_none());
    assert!(matches!(
        decode(b"\0\0\x09\0{\"code\":4}"),
        Some(HostControl::Removed(4))
    ));
    assert!(matches!(
        decode(b"\0\0\x01\0{\"guests\":null,\"offers\":null}"),
        Some(HostControl::Peers(_))
    ));
    for (control, kind, expected) in [
        (ClientControl::SetControl(4), 3, r#"{"guest_id":4}"#),
        (ClientControl::RemoveGuest(4), 5, r#"{"guest_id":4}"#),
        (
            ClientControl::RespondToOffer {
                attempt_id: "fixture".into(),
                accept: false,
            },
            4,
            r#"{"accept":false,"attempt_id":"fixture"}"#,
        ),
    ] {
        let frame = control.encode().unwrap();
        assert_eq!(&frame[..5], &[14, 0, 0, kind, 0]);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&frame[5..]).unwrap(),
            serde_json::from_str::<serde_json::Value>(expected).unwrap()
        );
    }
}
