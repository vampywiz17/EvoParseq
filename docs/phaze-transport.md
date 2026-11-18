# Experimental Phaze media/input and TURN investigation

This work is confined to codex/phaze-protocol and the Windows adapter.
The protocol source is original Rust, derived from wire-format observations
under authorized interoperability testing. No proprietary Phaze implementation
body, SDK or binary is included. Public physical-key/button constants are described
in [libmatoya's header](https://github.com/snowcone-ltd/libmatoya/blob/master/src/matoya.h).

## Audio and input

The hello requests raw PCM with option bit 0 set, or AAC-LC with it clear;
it does not request a virtual display layout. The saved Audio format preference
selects AAC-LC (default) or Uncompressed PCM on the next connection. Audio opcode
7 contains an encoded flag, big-endian stereo frame count and payload. Flag 0
accepts interleaved 48 kHz signed 16-bit little-endian stereo with an exact bounded
length. Flag 1 requires a zero frame-count header and a bounded raw AAC block.
The observed host uses AAC-LC 48 kHz stereo with AudioSpecificConfig `11 90`;
decoded output must contain 1024 frames. Symphonia 0.6.1 decodes AAC entirely in
Rust. Parsing, decoder state and reusable interleaved s16 storage live in the
shared Phaze crate. WASAPI only receives decoded samples and the observed codec.
No native Phaze or Media Foundation AAC codec code is used. A malformed packet
fails explicitly; it is never interpreted as another codec. No automatic mode
fallback or new worker/queue is added. Raw samples use 1.536 Mbit/s before
overhead; observed host AAC configuration is nominally 192 kbit/s. Existing
Parsec Opus decoding is unchanged. Original synthetic silent AAC blocks validate
actual decode, continuity, mode changes, bounds and error recovery in CI; live
AAC-host interoperability and latency still need user validation.

### USB/HID investigation (not implemented)

The official client's USB send/receive settings govern USB forwarding; they do
not establish compatibility with Parsec HID input messages. Static symbol
metadata includes `ClientUsbForwardDevice`, `ClientUsbGetDevices` and a separate
`ClientUsbFakeAdd` / `ClientUsbFakeSend` / `ClientUsbFakeRemove` family, plus host
USB enablement and a separately loaded USB library. These are evidence of a
potential virtual-device route, not a verified wire contract or a usable public
SDK. We do not call these proprietary entry points. Device descriptors, report
transport, host creation/acceptance and lifecycle/release behavior must be
established before an independent implementation. Existing common HID report
models may be reusable; the Phaze transport must be original and separately
validated. The HID/USB controls remain unavailable in EvoParseq Phaze mode.

Input opcodes 8/9/10/11 carry keyboard/wheel/button/motion messages. Each has
five big-endian words: selected output plus four event fields. Absolute motion
uses normalized 0..65535 coordinates in the presented letterbox. Keyboard events
use physical PC set-1 scans and side-specific modifiers; host text layout applies.
The public vocabulary and wire serialization live in the platform-independent
Phaze crate. Windows drains its existing event queue and retains focus, viewport,
fullscreen and detach policy. It positions before an activating button press,
releases remote state on focus loss/detach/queue failure and sends reset opcode 13.
No Parsec input packet is forwarded to a Phaze host.

Both Windows clients use the same adjacent-movement merging and preserve
click order. If the Phaze reliable window has at least 48 outstanding
packets, the last absolute position in an input batch can wait for space.
A newer position replaces it. Releases and output changes clear it.
Phaze host cursor images (opcode 12) and cached image IDs (opcode 19) feed
the shared Win32 cursor owner. Color images use the existing PNG/RGBA path.
Monochrome images preserve both cursor masks and their hotspot.

Video decode/presentation remains on the existing Windows backend. Audio and input
need user validation with a real host; synthetic tests cannot establish actual
sound, keyboard layout behavior or host permissions.

## TURN can carry the datagrams; host interoperability is a separate proof

A standard TURN server relays opaque UDP application datagrams. It does not need
to understand Phaze's STUN flavor, DTLS Connection ID or reliable media envelopes.
This is the same transport principle used by EvoParseq's Parsec BUD relay path.
The Phaze signaling candidate contains an address, port and short_ttl flag;
there is no SDP/WebRTC-only candidate type needed to describe an allocation.
A TURN allocation address can therefore be advertised in that candidate shape.
That is an architectural inference, not evidence that the installed host accepts it.

The first carrier-boundary implementation accepted a datagram carrier without
changing cryptography or media framing; its Windows runtime used direct UDP.
That initial isolated TURN fixture allocated an authenticated relay and checked both
payload preservation and the peer-observed relay source address for 20, 35 and
1232 byte datagrams. It uses only loopback, fixed fixture credentials and the
existing TURN implementation. The production carrier below supersedes that fixture.
The existing DTLS/CID tests independently cover mutual authentication and encrypted
heartbeat delivery. Neither test substitutes for an end-to-end native-host relay test.

To prove the real route, a client must allocate/renew TURN, install peer
permissions, advertise ONLY the allocated address in a forced-relay test, then
complete authenticated Phaze STUN, pinned DTLS, output discovery and sustained
video/audio/input through that allocation. Record selected-path evidence, not
just successful allocation. UDP and TCP/TLS client-to-TURN carriers need separate
validation; no direct fallback may hide a relay failure. Any runtime adapter
must retain the pure-Rust dependency requirement.

[TURN RFC 8656](https://www.rfc-editor.org/rfc/rfc8656.html) defines these allocation
and permission semantics. Phaze also distributes a separate company relay
([official download page](https://phaze.app/downloads)); implementing that proprietary
service is unnecessary for this TURN approach and is not part of this work.

## Additional public relay evidence (2026-11-17)

The user-supplied [public relay archive](https://public.phaze.app/relay/latest/phaze-relay-linux-amd64.zip)
was inspected locally without executing it. Its README describes UDP forwarding
(default inbound port 41000), outbound signaling to relay.phaze.app and a relay
key provisioned by the Phaze admin panel. This downloadable binary is not an
open-source code grant and is not incorporated into EvoParseq.

Bounded static inspection of its routing path identifies DTLS record type 25
(CID) and reads the eight-byte Connection ID at record offset 11. Handshake CID
extraction and a forward-table lookup precede UDP forwarding. This is consistent
with the already observed direct-host CID framing; no media-codec implementation
is required in the relay. It strengthens the opaque-datagram TURN design, but
does not prove that a native host selects an advertised TURN allocation.
No private endpoint, account credential, binary or native disassembly is included
in the repository.

## Production carrier integration (next feature-branch build)

The existing Datagram boundary is now consumed by a Rust TURN allocation client
using the already-vendored lowlat-core TURN codec. UDP, TCP and TLS are explicit
carriers for the same Phaze packets. Integrity, 401/438 challenges, transaction
IDs/timeouts, allocation expiry, periodic refresh and IP permissions are handled
without a native crypto provider. Send/Data indications provide the required
transparent datagram transport; unregistered ChannelData is not accepted.

The Windows Phaze Network page reuses the existing URL validation, credential
provisioning and protected storage. Force TURN advertises only the allocation.
The route is confirmed only after authenticated STUN and pinned DTLS complete.
Direct fallback is prohibited in this mode. No Parsec transport policy changes.
The TLS embedding adapter supplies public and OS-trusted root certificates;
hostname checks remain mandatory. No peer-supplied CA is trusted.

Automated fixtures exercise the actual production carrier over UDP/TCP/TLS,
fragmented stream reads, untrusted-certificate rejection and allocation/permission
renewal. Test server crypto dependencies are not the production Phaze provider.
Live native-host Cloudflare/UDP validation is recorded below; TCP/TLS still
require their own native-host tests.

The first live Cloudflare/UDP trial completed allocation and signaling but no
peer datagram returned: 30 sends succeeded locally and 90 sends to the other
three candidates were unreachable. The host advertised three private addresses
and one public IPv4 address; its log showed no peer probe. These counters do not
prove delivery to the host or a particular NAT behavior.

Unlike a local-interface candidate, the public TURN candidate now advertises
`short_ttl: true`, requesting the native host's outbound NAT-opening probe.
The original `false` value did not request that probe. This changes candidate
signaling only; the client's relayed STUN/DTLS packets retain normal routing.
Direct local-interface candidates retain `false`. A live retry is required to
establish whether this resolves the observed failure; symmetric NAT and relay
reachability are not established by the synthetic carrier tests.

## Post-answer synchronization

The short-TTL trial was observed in the native host log, but it did not produce
a returned peer datagram. Further static interoperability analysis identified
the separate signaling action that starts normal-TTL host checks: `sync`.
The native client sends it after processing `answer`; the host's `SyncData`
handler invokes its P2P-start operation. This advances its peer-check state,
whose scheduler sends authenticated STUN requests with the default IP TTL.
It is distinct from the short-TTL candidate probe and from a response to an
already received UDP request. It requires neither the company relay service
nor an application/media packet on an established DTLS connection.

The independent Rust client now sends `{"action":"sync","to":host_peer_id,
"attempt_id":current_attempt_id,"data":null}` after validating the answer, before peer
STUN/DTLS. This uses the same authenticated signaling connection and attempt;
it does not add credentials or change certificate verification. A successful
write logs `host_sync_sent`, not a claim that the host has processed it.
Existing authenticated peer-reflexive endpoint learning can then accept the
host's actual NAT source port. Native-host TURN success still needs a live
retry; neither a static state transition nor synthetic carrier tests prove it.
No native implementation body is incorporated into the Rust source.

## Live Cloudflare TURN/UDP validation (2026-11-17)

The user tested Windows build `33fc27c` with Cloudflare and Force TURN/UDP and
confirmed that the connection displayed video. The credential-free client log
records `host_sync_sent`, authenticated STUN, authenticated DTLS and
`phaze_route_selected` with `route=TURN`, `carrier=TURN / UDP` and
`authenticated=true`. The existing carrier advertises only the allocation in
this mode and has no direct fallback. The log then records display discovery,
H.264 Media Foundation decoder selection and PCM s16le audio startup.

This verifies native-host TURN/UDP connection establishment and user-visible
H.264 video after the missing sync was added. The roughly 67-second connected
log interval contains no session-failure event, but it does not independently
prove continuous frame updates, audible sound or input delivery over that
whole interval. Audio playback and input were previously user-confirmed on the
direct/VPN path; their TURN behavior needs user confirmation. Native-host
TURN/TCP and TURN/TLS remain unverified by this trial. No private addresses,
credentials, packet captures or proprietary implementation bodies are included.


## Windows session controls and service-specific settings

The shared settings model selects the service's implemented rows. Parsec retains
its transport, HID and audio negotiation controls. Phaze exposes the shared GPU,
GPU processing, presentation, playback buffer and immersive input preferences;
it exposes its own AAC/PCM selection, but not Parsec HID or a 10-bit host request.
The login service dropdown uses the native dark form styling and separate rows.

Phaze stream request opcode 3 carries six big-endian u32 words: output ID,
capabilities (bit 0 HEVC, bit 1 full chroma), maximum Mbps, FPS, maximum width
and maximum height. The independent encoder bounds Mbps to 1–100 and FPS to
1–240 and retains the existing 4096x4096 request bound. HEVC and 4:4:4 are
requests, not proof of the resulting stream format. The existing Annex B
inspector and GPU video pipeline determine and report the actual codec/format.
These controls need live host validation, including GPU-specific HEVC/4:4:4.
No proprietary implementation bodies or binary components are incorporated.

The existing session overlay handles bitrate/FPS requests, advertised active
output selection, local fullscreen/immersive input, volume/mute, settings and
stats. Output switching stops the old stream (opcode 4 + output ID) before
requesting the new one. A sent request is not labeled host-confirmed.
The Video tab distinguishes the last sent bitrate/FPS request from measured
decoded FPS and received compressed video payload bitrate. The latter counts
reassembled encoded bytes, excluding message headers, padding and transport
overhead. The event log records the requested values after carrier submission.
The client also reports observed packet-train bandwidth in extended Phaze
feedback. It measures complete long fragment groups and host padding probes,
using positive spacing between consecutive packets of the same size. The
feedback contains two smoothed bounds in bytes per second. Short, incomplete
or coalesced samples without enough usable intervals produce no estimate.
This transport measurement is separate from the requested encoder limit and
from the received encoded payload rate.
Host resolution/rotation/constant-FPS, display refresh requests, secure attention,
additional simultaneous displays and USB forwarding remain unavailable until
implemented. The official product's support does not establish our client support.

Stats reuse the shared sample, rates and native playback/decoder/presentation
reports. Carrier byte counters exclude TURN encapsulation and initial handshake;
Audio payload bitrate counts delivered AAC or PCM payload bytes before decoding,
not the expanded PCM playback buffer. Audio message headers, DTLS/TURN/network
overhead and duplicate retransmissions are excluded. It is measured over the
sampling interval, not a nominal encoder bitrate read from metadata. Audio layout
and the observed codec are reported. Connection RTT is
measured locally from reliable commands and their authenticated ACKs (kinds 3/4),
using the existing cumulative frontier and selective bitmap. Timing starts at
carrier batch submission, committed only after a successful flush. Each ACK
contributes at most one sample, from its newest newly acknowledged command.
Retransmitted commands are excluded following the ambiguity rule described in
[RFC 6298 section 3](https://www.rfc-editor.org/rfc/rfc6298.html#section-3).
Reliable commands retain the existing 250 ms retry interval and 20-retry budget
per packet. Sending fresh input does not consume another packet's retry budget.
Duplicate ACKs do not refresh the sample. It expires after ten seconds without a
new eligible ACK, so an idle connection can show N/A. The measurement includes
local/carrier scheduling and host ACK delay; it is not host-reported telemetry,
ICMP ping, frame latency or a separate TURN-server ping. It follows the selected
direct/TURN carrier without sending extra probes. Deterministic fixtures cover
ACK reordering, cumulative/selective feedback, retries, invalid/duplicate ACKs,
unsent packets, sample expiry and the shared Windows stats adapter. Live RTT
validation remains outstanding. Host encode time and other unparsed host
telemetry remain unknown (N/A). The new controls
and UI do not establish live HEVC, full chroma or display-switching success.
