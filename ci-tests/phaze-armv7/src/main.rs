use aes_gcm::{aead::AeadInPlace, Aes256Gcm, KeyInit};
use purecrypto::{
    cipher::{Aes256, Aes256Gcm as CurrentGcm},
    ec::{BoxedEcdsaPrivateKey, CurveId},
    tls::{Config, Connection, ProtocolVersion, SigningKey},
    x509::{CertSigner, Certificate, DistinguishedName, Time, Validity},
};
use std::{
    hint::black_box,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const BODY: usize = 1187;
const RECORDS: usize = 512;
const RUNS: usize = 5;
const KEY: [u8; 32] = [0x42; 32];
const AAD: [u8; 32] = [0x58; 32];
const PAYLOAD: [u8; BODY] = [0x5e; BODY];

fn summarize(label: &str, mut runs: [f64; RUNS]) {
    runs.sort_by(f64::total_cmp);
    println!(
        "{label}: {:.2} us/packet (median; {:.2}–{:.2} us)",
        runs[RUNS / 2],
        runs[0],
        runs[RUNS - 1]
    );
}

fn aead() {
    let current = CurrentGcm::new(Aes256::new(&KEY));
    let candidate = Aes256Gcm::new_from_slice(&KEY).unwrap();
    let fixtures: Vec<_> = (0..RECORDS)
        .map(|i| {
            let mut nonce = [0; 12];
            nonce[4..].copy_from_slice(&(i as u64).to_be_bytes());
            let mut ciphertext = PAYLOAD;
            let tag = current.encrypt(&nonce, &AAD, &mut ciphertext);
            let mut alternate = PAYLOAD;
            let alternate_tag = candidate
                .encrypt_in_place_detached((&nonce).into(), &AAD, &mut alternate)
                .unwrap();
            assert_eq!(ciphertext, alternate);
            assert_eq!(tag, <[u8; 16]>::from(alternate_tag));
            (nonce, ciphertext, tag)
        })
        .collect();
    let (nonce, ciphertext, mut tag) = fixtures[0];
    tag[0] ^= 1;
    assert!(current
        .decrypt(&nonce, &AAD, &mut ciphertext.clone(), &tag)
        .is_err());
    assert!(candidate
        .decrypt_in_place_detached(
            (&nonce).into(),
            &AAD,
            &mut ciphertext.clone(),
            (&tag).into()
        )
        .is_err());

    let mut current_runs = [0.0; RUNS];
    let mut candidate_runs = [0.0; RUNS];
    for (current_time, candidate_time) in current_runs.iter_mut().zip(&mut candidate_runs) {
        let start = Instant::now();
        for (nonce, ciphertext, tag) in black_box(&fixtures) {
            let mut plain = *ciphertext;
            current.decrypt(nonce, &AAD, &mut plain, tag).unwrap();
            black_box(plain);
        }
        *current_time = start.elapsed().as_secs_f64() * 1e6 / RECORDS as f64;

        let start = Instant::now();
        for (nonce, ciphertext, tag) in black_box(&fixtures) {
            let mut plain = *ciphertext;
            candidate
                .decrypt_in_place_detached(nonce.into(), &AAD, &mut plain, tag.into())
                .unwrap();
            black_box(plain);
        }
        *candidate_time = start.elapsed().as_secs_f64() * 1e6 / RECORDS as f64;
    }
    summarize("purecrypto AES-256-GCM decrypt", current_runs);
    summarize("RustCrypto AES-256-GCM decrypt", candidate_runs);
}

fn dtls_pair(suite: u16) -> (Connection, Connection) {
    let key = BoxedEcdsaPrivateKey::generate(CurveId::P256, &mut purecrypto::rng::OsRng);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let certificate = Certificate::self_signed_general(
        &CertSigner::Ecdsa(&key),
        &DistinguishedName::common_name("Offline benchmark fixture"),
        &Validity::new(Time::from_unix(now - 300), Time::from_unix(now + 3600)),
        1,
        false,
        &[],
    )
    .unwrap()
    .to_der()
    .to_vec();
    let cid = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let suites = [suite];
    let config = || {
        Config::builder()
            .rng(Arc::new(purecrypto::rng::OsRng))
            .dtls()
            .versions(ProtocolVersion::DTLSv1_2, ProtocolVersion::DTLSv1_2)
            .cipher_suites(&suites)
            .connection_id(cid.clone())
            .max_record_size(1232)
            .verify_certificates(false)
    };
    let mut client = Connection::client(&config().build()).unwrap();
    let mut server = Connection::server(
        &config()
            .cookie_secret([0x81; 32])
            .peer_address(b"armv7-benchmark-fixture".to_vec())
            .identity(vec![certificate.clone()], SigningKey::Ecdsa(key))
            .build(),
    )
    .unwrap();
    client.handshake().unwrap();
    for _ in 0..40 {
        exchange(&mut client, &mut server);
        exchange(&mut server, &mut client);
        if client.is_handshake_complete() && server.is_handshake_complete() {
            assert_eq!(client.peer_certificates()[0], certificate);
            assert_eq!(client.local_connection_id(), Some(cid.as_slice()));
            assert_eq!(client.negotiated_cipher_suite(), Some(suite));
            return (client, server);
        }
    }
    panic!("DTLS CID fixture did not connect");
}

fn exchange(from: &mut Connection, to: &mut Connection) {
    loop {
        let bytes = from.pop().unwrap();
        if bytes.is_empty() {
            break;
        }
        to.feed(&bytes).unwrap();
        to.handshake().unwrap();
    }
}

fn dtls(suite: u16, label: &str) {
    let (mut client, mut server) = dtls_pair(suite);
    server.send(&PAYLOAD[..BODY - 1]).unwrap();
    let valid = server.pop().unwrap();
    let mut tampered = valid.clone();
    *tampered.last_mut().unwrap() ^= 1;
    client.feed(&tampered).unwrap();
    assert!(client.recv().unwrap().is_empty());
    client.feed(&valid).unwrap();
    assert_eq!(client.recv().unwrap(), PAYLOAD[..BODY - 1]);
    client.feed(&valid).unwrap();
    assert!(client.recv().unwrap().is_empty());
    let mut runs = [0.0; RUNS];
    for measured in &mut runs {
        let packets: Vec<_> = (0..RECORDS)
            .map(|_| {
                server.send(&PAYLOAD[..BODY - 1]).unwrap();
                let wire = server.pop().unwrap();
                assert_eq!(wire.len(), 1232);
                assert_eq!(wire[0], 25);
                wire
            })
            .collect();
        let start = Instant::now();
        for packet in black_box(&packets) {
            client.feed(packet).unwrap();
            let plain = client.recv().unwrap();
            assert_eq!(plain.len(), BODY - 1);
            black_box(plain);
        }
        *measured = start.elapsed().as_secs_f64() * 1e6 / RECORDS as f64;
    }
    summarize(label, runs);
}

fn main() {
    println!(
        "Target: {} / {}, {}-bit; {RECORDS} records per run",
        std::env::consts::ARCH,
        std::env::consts::OS,
        usize::BITS
    );
    aead();
    dtls(0xc02b, "DTLS 1.2 AES-128-GCM CID feed + recv");
    dtls(0xc02c, "DTLS 1.2 AES-256-GCM CID feed + recv");
}
