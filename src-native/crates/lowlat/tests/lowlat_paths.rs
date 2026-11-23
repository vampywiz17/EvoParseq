//! Real UDP connectivity gates. Two native clients are connectivity peers,
//! not a stock host/media fixture: this proves the adapter's route and lifecycle
//! only. The separately configured coturn test must not use real credentials.
use evoparseq_lowlat::lowlat::{RelayTransport, Route, Session};
use lowlat_client::{Event, Peer};
use lowlat_core::conn::Kind;
use std::time::{Duration, Instant};

fn peer(ours: lowlat_crypto::Credentials) -> Peer {
    Peer {
        ufrag: ours.ufrag,
        pwd: ours.pwd,
        fingerprint: ours.fingerprint,
        aes256: Some(ours.aes256),
    }
}

fn connect(route: Route, relay_expected: bool) {
    // Exercise the actual application-decoded media mode on the client side.
    // The other endpoint is a connectivity peer, not a stock media host.
    let mut left = Session::external_video(false).unwrap();
    let mut right = Session::external_video(false).unwrap();
    let left_ours = left.offer("left", route).unwrap();
    let right_ours = right
        .offer("right", Route::Direct { stun: vec![] })
        .unwrap();
    // A real host seals BOTH directions with its answer's media key. Two
    // clients otherwise each select a different peer's key: ICE establishes
    // but authenticated native control cannot be decoded. Emulate the host's
    // single selected key explicitly in this symmetric transport fixture.
    let host_media_key = right_ours.aes256.clone();
    left.answer("left", peer(right_ours)).unwrap();
    let mut fixture_peer = peer(left_ours);
    fixture_peer.aes256 = Some(host_media_key);
    right.answer("right", fixture_peer).unwrap();
    let began = Instant::now();
    let mut connected = [false; 2];
    let mut left_candidates = 0;
    while !connected.iter().all(|x| *x) && began.elapsed() < Duration::from_secs(10) {
        for (side, established) in connected.iter_mut().enumerate() {
            let (source, target, target_id) = if side == 0 {
                (&mut left, &mut right, "right")
            } else {
                (&mut right, &mut left, "left")
            };
            while let Some(event) = source.poll().unwrap() {
                match event {
                    Event::Candidate {
                        addr, from_stun, ..
                    } => {
                        if side == 0 {
                            left_candidates += 1;
                            if relay_expected {
                                assert!(from_stun, "relay leaked a direct candidate");
                            }
                        }
                        target
                            .candidate(
                                target_id,
                                addr,
                                if from_stun {
                                    Kind::Reflexive
                                } else {
                                    Kind::Direct
                                },
                            )
                            .unwrap();
                    }
                    Event::Ready => target.ready(target_id).unwrap(),
                    Event::Established { .. } => *established = true,
                    Event::Ended { outcome } => panic!("LOWLAT path ended: {outcome:?}"),
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(left_candidates > 0, "no local candidates");
    if relay_expected {
        assert_eq!(left_candidates, 1, "expected one explicit UDP allocation");
    }
    assert_eq!(
        connected,
        [true, true],
        "native UDP connectivity did not establish"
    );
    // Verify actual encrypted native control delivery through this route.
    // The existing app's framed input is not remapped a second time.
    let mut frame = [0; 13];
    frame[..4].copy_from_slice(&37u32.to_be_bytes());
    frame[12] = 28;
    left.send_control(&frame).unwrap();
    frame[..4].copy_from_slice(&41u32.to_be_bytes());
    right.send_control(&frame).unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    let mut delivered = [false; 2];
    while !delivered.iter().all(|v| *v) && Instant::now() < until {
        while let Some(event) = left.poll().unwrap() {
            if let Event::ExternalControl { bytes } = event
                && bytes.len() == 13
                && bytes[12] == 28
                && bytes[..4] == 41u32.to_be_bytes()
            {
                delivered[0] = true;
            }
        }
        while let Some(event) = right.poll().unwrap() {
            if let Event::ExternalControl { bytes } = event
                && bytes.len() == 13
                && bytes[12] == 28
                && bytes[..4] == 37u32.to_be_bytes()
            {
                delivered[1] = true;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(delivered, [true, true], "native control round trip failed");
    // Native telemetry must cover real UDP traffic in both directions,
    // including protocol traffic beyond the delivered application message.
    for session in [&left, &right] {
        let stats = session.snapshot();
        assert!(stats.udp_sent > 13);
        assert!(stats.udp_received > 13);
        assert!(stats.udp_received >= stats.channels.iter().map(|c| c.bytes).sum());
    }
    left.close();
    right.close();
}

#[test]
fn direct_udp_connectivity() {
    connect(Route::Direct { stun: vec![] }, false);
}

#[test]
#[ignore = "requires isolated coturn on loopback, EVOPARSEQ_TEST_TURN endpoint"]
fn turn_udp_connectivity() {
    let server = std::env::var("EVOPARSEQ_TEST_TURN")
        .expect("isolated TURN endpoint")
        .parse()
        .unwrap();
    connect(
        Route::turn_udp(server, "evoparseq-fixture", "isolated-fixture-only"),
        true,
    );
}

#[test]
#[ignore = "requires isolated coturn with TCP listener, EVOPARSEQ_TEST_TURN endpoint"]
fn turn_tcp_connectivity() {
    let server = std::env::var("EVOPARSEQ_TEST_TURN")
        .expect("isolated TURN endpoint")
        .parse()
        .unwrap();
    connect(
        Route::turn(
            server,
            "evoparseq-fixture",
            "isolated-fixture-only",
            RelayTransport::Tcp,
        ),
        true,
    );
}
