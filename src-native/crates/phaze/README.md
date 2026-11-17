# Independent experimental Phaze protocol adapter

This crate is original EvoParseq Rust source implementing observed wire behavior
from authorized interoperability tests with a Phaze 0.13.2 host.
It contains no copied Phaze native source, binary extracts, SDK bindings,
or calls to the installed Phaze runtime.
Constants and message layouts express protocol compatibility, not shared native code.

The adapter uses open-source Rust libraries: purecrypto (MIT) for DTLS 1.2/CID,
rustls-rustcrypto (MIT OR Apache-2.0) with rustls for service HTTPS/WebSocket TLS,
and the Rust ecosystem dependencies recorded in the application lockfile.
Both crypto backends are experimental; this is an experimental development branch,
not a production cryptographic dependency recommendation.

## Implemented

- Service authentication, optional 2FA, authorized online host discovery and logout.
- Bounded authenticated WebSocket offer/answer; host approval is preserved.
- Direct IPv4/IPv6 UDP candidate probing with authenticated STUN.
- DTLS 1.2/CID with fresh ephemeral identity, mutual certificate handshake and
  mandatory signaling-fingerprint pinning before application requests.
- Output-list reception, selection of the primary active output and H.264 request.
- Bounded fragment reassembly, duplicate handling, acknowledgment and bounded
  retry of the small outbound hello/request commands.
- Borrowed compressed video callbacks; decode and presentation stay in EvoParseq's
  existing Windows Pipeline and shared Annex B inspector. No decoded-pixel readback.

The Rust proof received the real host output list and decoded one H.264 picture.
That proof is not validation of this GUI integration, sustained playback, remote
routing under loss, or the full native congestion/retransmission behavior.

## Explicit limitations

Windows integration only. Experimental video-only primary-display session:
H.264, 8-bit, 4:2:0; initial request uses the verified 10 Mbps / 30 fps fields.
No Phaze audio, input, clipboard, USB, virtual-display management, multi-display,
HEVC, 10-bit, 4:4:4, or relay integration is claimed.
Unintegrated Parsec session controls/settings are unavailable in Phaze mode.
Existing local GPU/presentation preferences are reused.
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
