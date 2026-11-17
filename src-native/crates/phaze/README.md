# Independent experimental Phaze protocol adapter

This crate is original EvoParseq Rust source implementing observed wire behavior
from authorized interoperability tests with a Phaze 0.13.2 host.
It contains no copied Phaze native source, binary extracts, SDK bindings,
or calls to the installed Phaze runtime.
Constants and message layouts express protocol compatibility, not shared native code.

The adapter uses open-source Rust libraries: purecrypto (MIT) for DTLS 1.2/CID,
rustls-rustcrypto (MIT OR Apache-2.0) with rustls for service HTTPS/WebSocket TLS,
and the Rust ecosystem dependencies recorded in the application lockfile.
The open-source RustCrypto provider is vendored with its original licenses and
unchanged crypto source; its manifest replaces unmaintained paste with pastey.
See vendor/rustls-rustcrypto/EVOPARSEQ-PATCH.md for provenance and the exact change.
Both crypto backends are experimental; this is an experimental development branch,
not a production cryptographic dependency recommendation.

## Implemented

- Service authentication, optional 2FA, authorized online host discovery and logout.
- Bounded authenticated WebSocket offer/answer; host approval is preserved.
- Direct IPv4/IPv6 UDP candidate probing with authenticated STUN.
- Protected peer heartbeat (cd 03, no payload) every 200 ms. Heartbeats are
  handled separately from reliable media frames; a static image is not a dead peer.
- DTLS 1.2/CID with fresh ephemeral identity, mutual certificate handshake and
  mandatory signaling-fingerprint pinning before application requests.
- Output-list reception, active-output selection and bounded stream requests for
  H.264/HEVC, full chroma, bitrate and FPS.
- Bounded fragment reassembly, duplicate handling, acknowledgment and bounded
  retry of outbound hello, request and input commands.
- AAC-LC or raw 48 kHz stereo s16le audio. Pure-Rust Symphonia 0.6.1 decodes
  bounded raw AAC packets using AudioSpecificConfig 11 90; PCM is checked against
  its declared sample count. Shared reusable samples feed existing WASAPI playback.
  The saved Audio format selection applies at reconnect; AAC-LC is the default.
  Original synthetic decoder fixtures are covered by CI; live AAC validation
  remains outstanding. No native codec library or native Phaze code is included.
- Physical keyboard, normalized absolute mouse, relative deltas, five buttons,
  wheel input and focus-loss release. Input starts only when a video viewport is ready.
- Borrowed compressed video callbacks; decode and presentation stay in EvoParseq's
  existing Windows Pipeline and shared Annex B inspector. No decoded-pixel readback.

A user-confirmed Windows GUI test displayed H.264 8-bit 4:2:0 through the existing
Media Foundation pipeline and continued updating beyond one minute after the
protected heartbeat fix. A subsequent direct/VPN test confirmed sound, mouse
clicks and scrolling; keyboard typing still needs explicit user confirmation.
Loss behavior and the full native congestion/retransmission behavior remain unverified.

## Explicit limitations

Windows integration only. H.264 8-bit 4:2:0 has live validation. HEVC and
4:4:4 requests are integrated for testing through the existing GPU video path;
their native-host results still need live confirmation. Initial bitrate/FPS use
the saved shared preferences, defaulting to 10 Mbps / 60 FPS.
No Phaze clipboard, USB, virtual-display management, simultaneous multi-display
or 10-bit host request is implemented.
Explicit Force TURN routing now uses the production Rust carrier over UDP, TCP
or TLS, with no direct fallback. Cloudflare TURN/UDP connectivity and H.264 video
were confirmed against the native host on 2026-11-17 after the post-answer
`sync` fix. Native-host TCP/TLS tests remain outstanding.
Raw audio costs approximately 1.536 Mbit/s before transport overhead. The observed
host AAC-LC configuration is nominally 192 kbit/s. Actual codec is reported in stats.
Unintegrated Parsec session controls/settings are unavailable in Phaze mode.
Existing local GPU/presentation, playback buffer and immersive input preferences
are reused. The canonical session overlay provides local controls, bitrate/FPS
requests, active-output selection and measured stats. Missing host telemetry
remains unknown. Settings tabs show the selected service's integrated options.
Connection RTT uses local reliable-command/ACK timing on the selected carrier;
retransmitted packets are excluded and samples expire after ten seconds without
a fresh eligible ACK. No extra probes are sent. Host ACK and local scheduling
delay are included; live RTT validation remains outstanding.
No automatic fallback to Parsec or WebRTC occurs.

The observed host omits the RFC 7627 EMS extension. This adapter explicitly permits
legacy non-EMS DTLS 1.2 while continuing to offer EMS. This has weaker protection
than mandatory EMS; certificate/signature/Finished checks and signaling pinning
remain required. HTTPS and WSS use normal public roots, hostname checks and an
explicit RustCrypto provider; Parsec's existing TLS provider is not changed.

Credentials and frames are not written to diagnostic files. The application uses
its existing Windows-protected saved-session mechanism, with an encrypted provider
tag and provider selection check so a saved Phaze session is not used for Parsec.
Only one account is remembered at a time. Switching service does not itself revoke
a remembered account; signing out removes/revokes the selected account.

## Validation

Use synthetic fixtures only in automated tests. The protocol crate's tests cover
fragment reordering, duplicates, malformed lengths, outgoing ACK bounds, mutual
DTLS/CID authentication/encrypted data and the missing-EMS negotiation distinction.
Live validation requires interactive login in the client; never insert account
credentials into test sources, command arguments, fixtures or logs.

## Testing an explicit TURN route on Windows

Select Phaze, open Settings > Network, enter a custom TURN URL and its username
and password (or use the existing Cloudflare credential provider), choose
Force TURN relay, save and reconnect. Other routing modes retain direct UDP
behavior in this experimental Phaze implementation. No new secrets storage is used.

Supported URL forms are turn:server:3478?transport=udp,
turn:server:3478?transport=tcp and turns:server:5349?transport=tcp.
TLS validates the hostname and public/Windows-trusted roots. Do not use an
untrusted certificate or send credentials in chat. IPv4 relay allocations and
legacy MD5/SHA-1 TURN long-term authentication are supported; extended RFC 8489
nonce-cookie algorithm negotiation is explicitly refused rather than downgraded.

Only the allocated relay address is advertised in a forced-relay offer; every
Phaze STUN/DTLS/media datagram uses that allocation. Route telemetry is emitted
only after authenticated peer STUN and signaling-pinned DTLS complete. A failed
relay does not connect directly. The host still needs an active physical or
virtual display.

The carrier reuses lowlat-core's existing RFC TURN codec and RustCrypto primitives.
A bounded receiver owns complete TCP/TLS frames; a maintenance task refreshes
the allocation and permissions before expiry. Neither runs on a decoded-video path.
