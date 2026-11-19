//! Offline fixtures only: private temporary settings and a scoped synthetic
//! storage key. No keyring, account or external TURN service is contacted.
use crate::{
    connection_settings::{Manager, RouteMode, Settings},
    network_settings::Draft,
};
use evoparseq_bud::bud::{RelayTransport, Route};

#[test]
fn linux_network_protected_turn_and_both_transports_share_routing_policy() {
    crate::secure_storage::fixture(|| {
        let file = Fixture::new();
        let manager = Manager::open(file.0.join("settings.json"));
        let mut draft = Draft::new(manager.view().0);
        draft.stun_urls = "stun:127.0.0.1:3478".into();
        draft.turn_urls = "turn:127.0.0.1:3478?transport=tcp\nturns:127.0.0.1:443?transport=tcp\nturn:127.0.0.1:3478?transport=udp".into();
        draft.values.custom_username = " synthetic-user ".into();
        draft.password.push_str("synthetic-password");
        draft.values.bud = true;
        draft.values.prefer_hevc = true;
        RouteMode::ForceRelay.apply(&mut draft.values);
        draft.save(&manager).unwrap();
        assert!(draft.password.is_empty());
        let raw = std::fs::read_to_string(file.0.join("settings.json")).unwrap();
        assert!(!raw.contains("synthetic-password"));
        let manager = Manager::open(file.0.join("settings.json"));
        assert_eq!(manager.view().0.secret().unwrap(), "synthetic-password");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;
            let config = manager.resolve_configuration().await.unwrap();
            assert_eq!(config.ice_transport_policy, RTCIceTransportPolicy::Relay);
            assert_eq!(config.ice_servers[1].credential, "synthetic-password");
            let (route, rest, snapshot) = crate::bud_routes::route(&manager).await.unwrap();
            assert!(matches!(route, Route::Turn(ref relay) if matches!(relay.transport, RelayTransport::Udp)));
            let mut rest = rest.unwrap();
            assert!(matches!(rest.pop_front(), Some(Route::Turn(ref relay)) if matches!(relay.transport, RelayTransport::Tls { .. })));
            assert!(matches!(rest.pop_front(), Some(Route::Turn(ref relay)) if matches!(relay.transport, RelayTransport::Tcp)));
            assert!(rest.is_empty() && snapshot.force_relay);
        });
        // Blank edits preserve protected secrets and all dormant platform fields.
        draft = Draft::new(manager.view().0);
        draft.save(&manager).unwrap();
        assert_eq!(manager.view().0.secret().unwrap(), "synthetic-password");
        assert!(manager.view().0.bud && manager.view().0.prefer_hevc);
        let before = std::fs::read(file.0.join("settings.json")).unwrap();
        draft.turn_urls = "https://invalid.invalid".into();
        assert!(draft.save(&manager).is_err());
        assert_eq!(std::fs::read(file.0.join("settings.json")).unwrap(), before);
    });
}
#[test]
fn linux_network_direct_only_skips_cloudflare_and_dormant_secrets() {
    let file = Fixture::new();
    let manager = Manager::open(file.0.join("settings.json"));
    let mut values = Settings::default();
    values.provider = "cloudflare".into();
    values.encrypted_api_token = "unusable-dormant-blob".into();
    values.stun_urls = vec!["stun:127.0.0.1:3478".into()];
    RouteMode::DirectOnly.apply(&mut values);
    manager.save(values, "", "", false, false).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let config = manager.resolve_configuration().await.unwrap();
            assert_eq!(config.ice_servers.len(), 1);
            let (route, relays, snapshot) = crate::bud_routes::route(&manager).await.unwrap();
            assert!(matches!(route, Route::Direct { .. }));
            assert!(relays.is_none());
            assert!(
                crate::bud_routes::relay_route(&manager, &snapshot)
                    .await
                    .unwrap()
                    .is_empty()
            );
        });
    let first = crate::bud_routes::native_attempt_id().unwrap();
    assert_ne!(first, crate::bud_routes::native_attempt_id().unwrap());
    assert_eq!(
        first.split('-').map(str::len).collect::<Vec<_>>(),
        vec![8; 6]
    );
}
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let name = crate::bud_routes::native_attempt_id().unwrap();
        let path = std::env::temp_dir().join(format!("evoparseq-network-{name}"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
