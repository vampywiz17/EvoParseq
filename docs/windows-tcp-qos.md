# Windows TCP/TLS QoS investigation — 2026-10-26

## Result

A separate Windows probe successfully attached an already connected, dual-stack
TCP socket to a qWAVE non-adaptive AudioVideo flow. `QOSQueryPacketPriority`
returned DSCP **40** and L2 **0**. A certificate-validated TLS 1.2 HTTPS HEAD
request to `example.com` returned HTTP 200 both with and without the flow.
Create/add/query took **1.9847 ms in that single marked trial**, at connection
setup, not per packet. This is an API/compatibility observation, not a latency
benchmark or a packet-capture proof of DSCP on the wire.

IPv4-mapped and IPv6 loopback flows were refused with error 1168; TCP payload
still arrived. Loopback therefore cannot establish scheduling benefit. An
initial Microsoft HTTPS target timed out in both control and marked trials;
the successful example.com control/marked pair above is the usable result.

A subsequent two-machine ABBA test (32 trials, 12,923 RTT samples) accepted all
16 requested marks. Average trial p95 under bidirectional load changed from
61.70 to 39.22 ms for TCP and 57.03 to 37.91 ms for TLS. TLS upload p95 changed
48.11 to 33.64 ms; TCP download showed no benefit. Large outliers remained.
These are preliminary observations on one direct network path, not proof of
an improvement over a real TURN connection. Load direction/bandwidth varied.

## Optional integration

Settings > Network > TCP/TLS QoS offers **Off (default)** and experimental
Audio/video QoS. It applies to outgoing Windows TURN TCP/TLS sockets in both
lowlat and WebRTC, once after connect and before TLS setup. A tiny shared Rust
crate owns the flow; platform-specific FFI is isolated behind the same API.
Other targets are no-ops, preserving their existing socket policies.
No extra runtime dependency, thread, queue, timer or per-packet/frame work is
added. Existing rustls verification, IOCP/Tokio I/O, video decoding and
presentation stay unchanged. UDP, auth HTTPS and signaling WSS are unaffected.

Unsupported/refused marking leaves the stream usable and unmarked. With event
logging enabled, `turn_tcp_qos` records one requested/applied/error result per
connected socket, plus configured DSCP/L2 when available. Configured priority
is not a packet-capture proof or a router guarantee. Test with real TURN under
congestion before interpreting the synthetic results as a streaming benefit.

## Fit with the current client

- lowlat already has a dynamically loaded qWAVE helper for its UDP paths. Its UDP
  connect/mark/disconnect sequence must **not** be reused on established TCP.
- lowlat TCP/TLS shares one overlapped TCP socket. The stream adapter marks once after
  successful connect, without
  modifying the read/write buffers, completion port, TLS records, or media path.
- WebRTC TURN TCP/TLS has separate Tokio/socket ownership. The lowlat helper cannot
  automatically cover it; its socket lifetime needs its own explicit adapter.
- A client marks its **outgoing** traffic only: input, control and TCP ACKs may
  benefit under a local upstream bottleneck. The host/relay's incoming video is
  not automatically marked by this request. One multiplexed TCP connection also
  cannot assign independent per-message priority to video/audio/input records.
- Use `QOSTrafficTypeAudioVideo` with `QOS_NON_ADAPTIVE_FLOW`, no rate shaping,
  path probes or arbitrary DSCP override. Failure must leave the connection
  usable, with no repeated per-packet setup or mandatory service requirement.
- `QOSSetOutgoingDSCPValue` has additional privilege requirements and is not
  needed for this approach. No system policy, registry, certificate verification,
  congestion algorithm or transport selection was changed for the probe.
- Configured priority is not an end-to-end guarantee. Network policy/routers
  may ignore or rewrite it; L2 marking was zero in this trial.

## Measurement tool

A separate portable Rust receiver/client is now provided in
[tools/qos-latency](../tools/qos-latency/README.md). It runs alternating
off/on/on/off trials over direct IPv4 TCP and certificate-pinned TLS, with idle,
upload, download and bidirectional load. It records raw RTT samples, percentiles,
errors and qWAVE configuration. Failed/unverified-mark or failed-load trials are
inconclusive. Its dependencies and executable are isolated from the application;
the measurement protocol/load generator is not shipped in the client. The first
two-machine results are summarized above; real TURN A/B validation remains.

Compare disabled/enabled trials on the same real TURN TCP/TLS route, alternating
order, both idle and with controlled local upload contention. Measure input RTT,
p50/p95/p99 stalls and existing stream timing/drop counters. Capture only the
isolated test flow to verify outgoing DSCP; measure downstream congestion
separately. Keep it disabled until this establishes a worthwhile benefit and
verify refusal/service-unavailable cleanup without losing connectivity.

## Primary API references

- [QOSAddSocketToFlow](https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qosaddsockettoflow)
- [QOS traffic types](https://learn.microsoft.com/en-us/windows/win32/api/qos2/ne-qos2-qos_traffic_type)
- [QOSQueryFlow](https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qosqueryflow)
- [QOS_PACKET_PRIORITY](https://learn.microsoft.com/en-us/windows/win32/api/qos2/ns-qos2-qos_packet_priority)
- [QOSSetFlow and DSCP privileges](https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qossetflow)
