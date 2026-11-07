//! Shared BUD TURN URL ordering and endpoint resolution, independent of UI/OS.
use anyhow::{ensure, Result};
use evoparseq_bud::bud::Route;
use std::{collections::VecDeque, time::Duration};

pub(crate) fn relay_carrier(
    url: &webrtc::ice::url::Url,
) -> Option<evoparseq_bud::bud::RelayTransport> {
    use evoparseq_bud::bud::RelayTransport;
    use webrtc::ice::url::{ProtoType, SchemeType};
    match (url.scheme, url.proto) {
        (SchemeType::Turn, ProtoType::Udp) => Some(RelayTransport::Udp),
        (SchemeType::Turn, ProtoType::Tcp) => Some(RelayTransport::Tcp),
        (SchemeType::Turns, ProtoType::Tcp) => Some(RelayTransport::Tls {
            server_name: url.host.clone(),
        }),
        _ => None,
    }
}

pub(crate) async fn turn_relays(
    servers: Vec<webrtc::ice_transport::ice_server::RTCIceServer>,
) -> Result<VecDeque<Route>> {
    let mut urls = Vec::new();
    for server in &servers {
        for raw in &server.urls {
            let url = webrtc::ice::url::Url::parse_url(raw)?;
            if let Some(carrier) = relay_carrier(&url) {
                let priority = match carrier {
                    evoparseq_bud::bud::RelayTransport::Udp => 0,
                    evoparseq_bud::bud::RelayTransport::Tls { .. } => 1,
                    _ if url.port == 80 => 2,
                    _ => 3,
                };
                urls.push((priority, url, carrier, server));
            }
        }
    }
    urls.sort_by_key(|entry| entry.0);
    // Resolve concurrently so an unavailable DNS entry does not serially hold
    // up other routes. join_all preserves the explicitly sorted URL priority.
    let routes: VecDeque<_> = futures_util::future::join_all(urls.into_iter().take(32).map(
        |(_, url, carrier, server)| async move {
            let addresses = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::lookup_host((url.host.as_str(), url.port)),
            )
            .await
            .ok()?
            .ok()?;
            let addr = addresses.into_iter().find(|a| a.is_ipv4())?;
            Some(Route::turn(
                addr,
                &server.username,
                &server.credential,
                carrier,
            ))
        },
    ))
    .await
    .into_iter()
    .flatten()
    .collect();
    ensure!(
        !routes.is_empty(),
        "BUD requires a reachable UDP, TLS or TCP TURN URL"
    );
    Ok(routes)
}

// Force TURN starts on the selected relay carrier; otherwise try direct UDP first.
#[cfg(any(
    all(windows, feature = "bud"),
    all(target_os = "linux", feature = "desktop-telemetry")
))]
pub(crate) async fn route(
    manager: &crate::connection_settings::Manager,
) -> Result<(
    Route,
    Option<VecDeque<Route>>,
    crate::connection_settings::Settings,
)> {
    let (settings, servers) = manager.resolve_bud().await?;
    if settings.force_relay {
        let mut routes = turn_relays(servers).await?;
        let first = routes
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("BUD requires a supported TURN URL"))?;
        return Ok((first, Some(routes), settings));
    }

    let mut stun = Vec::new();
    for server in servers {
        for raw in server.urls {
            let url = webrtc::ice::url::Url::parse_url(&raw)?;
            if url.scheme == webrtc::ice::url::SchemeType::Stun
                && url.proto == webrtc::ice::url::ProtoType::Udp
            {
                stun.extend(
                    tokio::net::lookup_host((url.host.as_str(), url.port))
                        .await?
                        .filter(|a| a.is_ipv4()),
                );
            }
        }
    }
    Ok((Route::Direct { stun }, None, settings))
}

#[cfg(any(
    all(windows, feature = "bud"),
    all(target_os = "linux", feature = "desktop-telemetry")
))]
pub(crate) async fn relay_route(
    manager: &crate::connection_settings::Manager,
    settings: &crate::connection_settings::Settings,
) -> Result<VecDeque<Route>> {
    if settings.stun_only || (settings.provider == "custom" && settings.turn_urls.is_empty()) {
        return Ok(VecDeque::new());
    }
    turn_relays(manager.resolve_bud_relay(settings).await?).await
}

#[cfg(any(
    all(windows, feature = "bud"),
    all(target_os = "linux", feature = "desktop-telemetry")
))]
pub(crate) fn native_attempt_id() -> Result<String> {
    let mut random = [0u8; 24];
    getrandom::fill(&mut random).map_err(|_| anyhow::anyhow!("BUD attempt entropy unavailable"))?;
    Ok(random
        .as_chunks::<4>()
        .0
        .iter()
        .map(|part| {
            format!(
                "{:02x}{:02x}{:02x}{:02x}",
                part[0], part[1], part[2], part[3]
            )
        })
        .collect::<Vec<_>>()
        .join("-"))
}
