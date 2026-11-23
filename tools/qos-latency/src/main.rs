//! Standalone synthetic network test; never linked into EvoParseq.
mod qos;
mod wire;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use wire::{Channel, Peer};
const PEER_FILE: &str = "qos-peer.json";
const DEFAULT_PORT: u16 = 49192;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Load {
    Idle,
    Upload,
    Download,
    Both,
}
impl Load {
    fn directions(self) -> &'static [u8] {
        match self {
            Self::Idle => &[],
            Self::Upload => &[1, 1],
            Self::Download => &[2, 2],
            Self::Both => &[1, 1, 2, 2],
        }
    }
}
#[derive(Debug, Serialize)]
struct Metrics {
    samples: usize,
    p50_ms: Option<f64>,
    p95_ms: Option<f64>,
    p99_ms: Option<f64>,
    max_ms: Option<f64>,
}
fn metrics(samples: &[f64]) -> Metrics {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |percent: usize| {
        sorted
            .get((sorted.len() * percent).div_ceil(100).saturating_sub(1))
            .copied()
    };
    Metrics {
        samples: sorted.len(),
        p50_ms: percentile(50),
        p95_ms: percentile(95),
        p99_ms: percentile(99),
        max_ms: sorted.last().copied(),
    }
}
#[derive(Serialize)]
struct BulkResult {
    direction: &'static str,
    bytes: u64,
    error: Option<String>,
}
#[derive(Serialize)]
struct Trial {
    transport: &'static str,
    load: Load,
    order: usize,
    qos: qos::Status,
    measured_seconds: f64,
    metrics: Metrics,
    timeouts: usize,
    error: Option<String>,
    load_results: Vec<BulkResult>,
    rtt_ms: Vec<f64>,
}
#[derive(Serialize)]
struct Report {
    schema: u32,
    source: &'static str,
    receiver: String,
    seconds_per_trial: u32,
    notes: Vec<&'static str>,
    trials: Vec<Trial>,
}
fn server(port: u16) -> Result<()> {
    ensure!(
        (1025..65535).contains(&port),
        "choose a port between 1025 and 65534"
    );
    let tcp = TcpListener::bind(("0.0.0.0", port)).context("TCP receiver port unavailable")?;
    let tls = TcpListener::bind(("0.0.0.0", port + 1)).context("TLS receiver port unavailable")?;
    let (peer, config) = wire::receiver_config(port)?;
    std::fs::write(PEER_FILE, serde_json::to_vec_pretty(&peer)?)?;
    println!(
        "Receiver ready. TCP {port}, TLS {}.\nCopy {PEER_FILE} beside the EXE on the CLIENT.\nUse this receiver's LAN/VPN IP on the client.\nStop with Ctrl+C or close this window. No firewall rules are changed.",
        port + 1
    );
    let active = Arc::new(AtomicUsize::new(0));
    let accept = |listener: TcpListener, tls: bool| {
        let peer = peer.clone();
        let config = config.clone();
        let active = active.clone();
        thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else {
                    break;
                };
                if active.fetch_add(1, Ordering::AcqRel) >= 8 {
                    active.fetch_sub(1, Ordering::AcqRel);
                    continue;
                }
                let active = active.clone();
                let peer = peer.clone();
                let config = config.clone();
                thread::spawn(move || {
                    struct Permit(Arc<AtomicUsize>);
                    impl Drop for Permit {
                        fn drop(&mut self) {
                            self.0.fetch_sub(1, Ordering::AcqRel);
                        }
                    }
                    let _permit = Permit(active);
                    if wire::configure(&socket).is_err() {
                        return;
                    }
                    let channel = if tls {
                        let Ok(connection) = rustls::ServerConnection::new(config) else {
                            return;
                        };
                        Channel::Server(Box::new(rustls::StreamOwned::new(connection, socket)))
                    } else {
                        Channel::Plain(socket)
                    };
                    // EOF, expired trials and wrong pairing close only this connection.
                    let _ = wire::serve(channel, &peer);
                });
            }
        })
    };
    let plain = accept(tcp, false);
    let encrypted = accept(tls, true);
    plain
        .join()
        .map_err(|_| anyhow::anyhow!("TCP accept worker failed"))?;
    encrypted
        .join()
        .map_err(|_| anyhow::anyhow!("TLS accept worker failed"))?;
    Ok(())
}
struct LoadGate {
    ready: std::sync::mpsc::SyncSender<bool>,
    start: std::sync::mpsc::Receiver<()>,
}
fn await_loads(receiver: &std::sync::mpsc::Receiver<bool>, count: usize) -> bool {
    let deadline = Instant::now() + Duration::from_secs(12);
    (0..count).all(|_| {
        receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) == Ok(true)
    })
}
fn bulk(
    addr: SocketAddr,
    peer: Peer,
    tls: bool,
    direction: u8,
    seconds: u32,
    stop: Arc<AtomicBool>,
    gate: LoadGate,
) -> BulkResult {
    let mut result = BulkResult {
        direction: if direction == 1 {
            "upload_submitted"
        } else {
            "download_received"
        },
        bytes: 0,
        error: None,
    };
    let work = (|| -> Result<()> {
        let connection = wire::connect(addr, &peer, tls, false, direction, seconds);
        let _ = gate.ready.send(connection.is_ok());
        let (mut channel, _flow) = connection?;
        gate.start
            .recv_timeout(Duration::from_secs(15))
            .context("load start deadline")?;
        let started = wire::start(&mut channel);
        let _ = gate.ready.send(started.is_ok());
        started?;
        let mut bytes = [0u8; wire::BLOCK];
        while !stop.load(Ordering::Acquire) {
            if direction == 1 {
                channel.write_all(&bytes)?;
                channel.flush()?;
                result.bytes += bytes.len() as u64;
            } else {
                let count = channel.read(&mut bytes)?;
                ensure!(count != 0, "load ended before sampling finished");
                result.bytes += count as u64;
            }
        }
        Ok(())
    })();
    if let Err(error) = work {
        result.error = Some(error.to_string());
    }
    result
}
fn trial(
    addr: SocketAddr,
    peer: &Peer,
    tls: bool,
    load: Load,
    mark: bool,
    seconds: u32,
    order: usize,
) -> Trial {
    let mut result = Trial {
        transport: if tls { "TLS" } else { "TCP" },
        load,
        order,
        qos: qos::Status {
            requested: mark,
            applied: false,
            configured_dscp: None,
            configured_l2: None,
            error: None,
        },
        measured_seconds: 0.0,
        metrics: metrics(&[]),
        timeouts: 0,
        error: None,
        load_results: vec![],
        rtt_ms: vec![],
    };
    let (mut channel, flow) = match wire::connect(addr, peer, tls, mark, 0, seconds) {
        Ok(connection) => connection,
        Err(error) => {
            result.error = Some(error.to_string());
            return result;
        }
    };
    result.qos = flow.status.clone();
    if mark && (!flow.status.applied || !flow.status.configured_dscp.is_some_and(|v| v > 0)) {
        result.error = Some(
            "QoS not verified on this route: marked trial skipped, not a latency comparison".into(),
        );
        return result;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let mut workers = vec![];
    let mut starters = vec![];
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(4);
    for &direction in load.directions() {
        let peer = peer.clone();
        let stop = stop.clone();
        let (start, receiver) = std::sync::mpsc::sync_channel(1);
        starters.push(start);
        let gate = LoadGate {
            ready: ready_tx.clone(),
            start: receiver,
        };
        workers.push(thread::spawn(move || {
            bulk(addr, peer, tls, direction, seconds, stop, gate)
        }));
    }
    // All load connections must be established before warmup and sampling.
    drop(ready_tx);
    if !await_loads(&ready_rx, load.directions().len()) {
        result.error = Some("load connection setup failed; trial is inconclusive".into());
    }
    for starter in starters {
        let _ = starter.send(());
    }
    if result.error.is_none() && !await_loads(&ready_rx, load.directions().len()) {
        result.error = Some("load did not start; trial is inconclusive".into());
    }
    if result.error.is_none() {
        thread::sleep(Duration::from_secs(1));
        if let Err(error) = wire::start(&mut channel) {
            result.error = Some(error.to_string());
        }
    }
    let start = Instant::now();
    let until = start + Duration::from_secs(u64::from(seconds));
    let mut sequence = 0u64;
    while result.error.is_none() && Instant::now() < until {
        let sent = Instant::now();
        sequence += 1;
        let exchange = (|| -> std::io::Result<()> {
            channel.write_all(&sequence.to_be_bytes())?;
            channel.flush()?;
            let mut reply = [0; 8];
            channel.read_exact(&mut reply)?;
            if u64::from_be_bytes(reply) != sequence {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "echo sequence mismatch",
                ));
            }
            Ok(())
        })();
        if let Err(error) = exchange {
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                result.timeouts += 1;
            }
            result.error = Some(error.to_string());
            break;
        }
        result.rtt_ms.push(sent.elapsed().as_secs_f64() * 1000.0);
        thread::sleep(Duration::from_millis(20).saturating_sub(sent.elapsed()));
    }
    result.measured_seconds = start.elapsed().as_secs_f64();
    stop.store(true, Ordering::Release);
    drop(ready_rx); // Also release senders if a setup/start phase failed early.
    // Socket deadlines bound workers if congestion prevents an immediate write.
    for worker in workers {
        match worker.join() {
            Ok(load) => {
                if load.bytes == 0 || load.error.is_some() {
                    result.error.get_or_insert_with(|| {
                        "load was not sustained; trial is inconclusive".into()
                    });
                }
                result.load_results.push(load);
            }
            Err(_) => result.error = Some("load worker failed".into()),
        }
    }
    result.metrics = metrics(&result.rtt_ms);
    result
}
fn client(host: &str, seconds: u32) -> Result<()> {
    ensure!(
        (5..=30).contains(&seconds),
        "trial duration must be 5..30 seconds"
    );
    ensure!(
        std::fs::metadata(PEER_FILE)?.len() <= 65536,
        "peer file exceeds limit"
    );
    let peer: Peer = serde_json::from_slice(&std::fs::read(PEER_FILE)?)?;
    ensure!(
        peer.version == 1 && (1025..65535).contains(&peer.port) && peer.certificate.len() < 16384,
        "invalid peer file"
    );
    // Pick one address and retain it across A/B trials; never silently change routes.
    let base = (host, peer.port)
        .to_socket_addrs()?
        .find(SocketAddr::is_ipv4)
        .context("receiver IPv4 address unavailable; use its LAN/VPN IPv4 address")?;
    for tls in [false, true] {
        let address = SocketAddr::new(base.ip(), peer.port + u16::from(tls));
        drop(
            wire::connect(address, &peer, tls, false, 0, 1).with_context(|| {
                format!("receiver preflight failed on TCP port {}", address.port())
            })?,
        );
    }
    let mut report = Report {
        schema: 1,
        source: option_env!("QOS_TEST_SOURCE").unwrap_or("local"),
        receiver: base.to_string(),
        seconds_per_trial: seconds,
        notes: vec![
            "Synthetic echo RTT, not capture-to-display or Parsec end-to-end latency.",
            "Bulk load uses separate sockets; this does not model multiplexed video/control head-of-line blocking inside one LOWLAT TLS stream.",
            "Only client probe sockets are marked. Load and receiver sockets remain unmarked.",
            "Priority query is configured OS policy, not proof of DSCP on the wire or router scheduling.",
            "Two unmarked bulk sockets per direction. Upload bytes count submitted writes; download bytes count received data.",
            "ABBA order: off/on/on/off. Repeat on the same path; p99 with fewer than 100 samples is unstable.",
            "Failed, QoS-unverified or load-failed trials cannot establish a QoS latency gain.",
        ],
        trials: vec![],
    };
    println!(
        "About 7 minutes at 10 seconds/trial. Stop streams and unrelated speed tests.\nThe test itself intentionally creates heavy upload/download load.\nPairing and TLS setup are excluded from RTT."
    );
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let file = format!("qos-results-{stamp}.json");
    for tls in [false, true] {
        let addr = SocketAddr::new(base.ip(), peer.port + u16::from(tls));
        for load in [Load::Idle, Load::Upload, Load::Download, Load::Both] {
            for (order, mark) in [false, true, true, false].into_iter().enumerate() {
                println!(
                    "{} {:?} QoS {} ({}/4)...",
                    if tls { "TLS" } else { "TCP" },
                    load,
                    if mark { "ON" } else { "OFF" },
                    order + 1
                );
                let result = trial(addr, &peer, tls, load, mark, seconds, order);
                println!(
                    "  {:?}; samples={} timeouts={} DSCP={:?} {}",
                    result.metrics,
                    result.metrics.samples,
                    result.timeouts,
                    result.qos.configured_dscp,
                    result.error.as_deref().unwrap_or("OK")
                );
                report.trials.push(result);
                std::fs::write(&file, serde_json::to_vec_pretty(&report)?)?;
                // Drain prior load before alternating the mark.
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
    println!(
        "Saved {file}. Share this result file, not qos-peer.json.\nA LAN/VPN result does not establish TURN TCP/TLS benefit."
    );
    Ok(())
}
fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut value = String::new();
    std::io::stdin().read_line(&mut value)?;
    Ok(value.trim().into())
}
fn main() -> Result<()> {
    if let Some(parent) = std::env::current_exe()?.parent() {
        std::env::set_current_dir(parent)?;
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    let interactive = args.is_empty();
    let role = if interactive {
        prompt("1 = receiver (other PC), 2 = client (this PC): ")?
    } else {
        args[0].clone()
    };
    let result = match role.as_str() {
        "1" | "server" => server(
            args.get(1)
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(DEFAULT_PORT),
        ),
        "2" | "client" => {
            let host = if let Some(host) = args.get(1) {
                host.clone()
            } else {
                prompt("Receiver LAN/VPN IP or hostname: ")?
            };
            client(
                &host,
                args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(10),
            )
        }
        _ => {
            println!(
                "Usage: evoparseq-qos-test server [port]\n       evoparseq-qos-test client HOST [seconds_per_trial]\nOr double-click and select 1/2."
            );
            Ok(())
        }
    };
    if interactive {
        if let Err(error) = &result {
            eprintln!("Test failed: {error:#}");
        }
        let _ = prompt("Press Enter to close.");
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coordinated_tls_load_stays_active_until_echo_sampling_finishes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (peer, config) = wire::receiver_config(addr.port().min(65534)).unwrap();
        let server_peer = peer.clone();
        let done = Arc::new(AtomicBool::new(false));
        let server_done = done.clone();
        let worker = thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(15);
            let mut clients = vec![];
            while !server_done.load(Ordering::Acquire) && Instant::now() < until {
                match listener.accept() {
                    Ok((socket, _)) => {
                        let peer = server_peer.clone();
                        let config = config.clone();
                        clients.push(thread::spawn(move || {
                            wire::configure(&socket).unwrap();
                            let connection = rustls::ServerConnection::new(config).unwrap();
                            let _ = wire::serve(
                                Channel::Server(Box::new(rustls::StreamOwned::new(
                                    connection, socket,
                                ))),
                                &peer,
                            );
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("listener failed: {error}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        let result = trial(addr, &peer, true, Load::Both, false, 1, 0);
        done.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(result.metrics.samples > 0);
        assert_eq!(result.load_results.len(), 4);
        assert!(
            result
                .load_results
                .iter()
                .all(|load| load.bytes > 0 && load.error.is_none())
        );
    }
    #[cfg(windows)]
    #[test]
    fn an_unverified_windows_flow_is_not_reported_as_a_marked_latency_sample() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (peer, _) = wire::receiver_config(addr.port().min(65534)).unwrap();
        let server_peer = peer.clone();
        let worker = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            wire::configure(&socket).unwrap();
            let _ = wire::serve(Channel::Plain(socket), &server_peer);
        });
        let result = trial(addr, &peer, false, Load::Idle, true, 1, 0);
        if result.qos.applied && result.qos.configured_dscp.is_some_and(|v| v > 0) {
            assert!(!result.rtt_ms.is_empty());
        } else {
            assert_eq!(result.metrics.samples, 0);
            assert!(result.error.as_ref().unwrap().contains("not verified"));
        }
        worker.join().unwrap();
    }
    #[test]
    fn percentiles_keep_tails_and_empty_samples_unknown() {
        assert!(metrics(&[]).p95_ms.is_none());
        let values: Vec<_> = (1..=100).rev().map(f64::from).collect();
        let report = metrics(&values);
        assert_eq!(report.p50_ms, Some(50.0));
        assert_eq!(report.p95_ms, Some(95.0));
        assert_eq!(report.p99_ms, Some(99.0));
        assert_eq!(report.max_ms, Some(100.0));
    }
}
