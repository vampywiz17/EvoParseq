# Transport-only lowlat client seam

MIT-licensed source derived from nomi-san/lowlat `feat/win-seams`, revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, `crates/client/src`.
Copyright (c) 2026 Nguyen Duy; original license in `LICENSE`.
Common/crypto remain pinned to that revision; local net/core copies carry the
isolated TURN stream additions described below.
This is an EvoParseq-specific subset, not the upstream SDK's complete API.

The application may supply a validated audio channel count/mask in the existing
initialization contract. Stereo remains the default. This adds no SDK decoder,
audio device, network scheduler or session-layer dependency.

Application-verified Main 10 capability is retained for external GPU decoding.
The stream-0 encoder-config bridge preserves the negotiated HEVC/range/depth
flags on later encoder requests, including negotiated 4:4:4/10-bit combinations. No packet scheduling, retransmission or congestion policy is changed.

The subset retains native credentials, AES-GCM, candidate/readiness exchange,
BUD rings, acknowledgements/NACKs, congestion control, liveness, UDP TURN,
allocation/permission renewal and clean departure. Packet/window sizes and the
native retransmission/congestion policies are unchanged. The application owns
account signaling and chooses direct or relay attempts separately from WebRTC.

Only bounded compressed-video/audio pools cross into the application. A sole,
non-cloneable media consumer borrows each pool lease synchronously outside the
input-command mutex. Network arrivals unpark it without a GUI/frame timer.
Stop the media consumer before teardown/reconnect; never retain a lease's pointer.
Native input packets enter a fixed-size bounded ring.
A full ring refuses input explicitly; session initialization/reports remain
transport-owned. Native AES keys remain inside the transport adapter.

Removed because EvoParseq supplies these functions already: SDK decoder
selection and threads, vendor/software video backends, decoded frame exporters,
CPU plane/readback paths, SDK audio decode and PCM staging, input remapping,
controller report conversion, SDK cursor-image decoding, semantic UI events and
microphone capture/encoding. Their tests/fixtures and unused dependencies were
removed with them. The unused port-mapping and separate NAT-classification APIs
were removed; normal STUN gathering and host candidates remain. There is no
second renderer, decoder, UI or account WebSocket in this subset. The native
client enables the small cursor-cache feature for host cache/forget semantics;
PNG decoding and cursor ownership remain in EvoParseq.

Original native controls are forwarded once to the retained UI. Only session
status, encoder generation/encode time and the client's roster number are also
read by the transport. Initialization requests uncached cursor updates to match
the existing UI contract. Decoder/keyframe recovery stays in the existing
application's MF pipeline; its keyframe request wakes the native protocol loop.
Periodic protocol reports retain the previous zero decode timings until actual
application timings are supplied; these zeros are not displayed as measured data.

Remaining upstream config/pool tests and the application's encrypted direct and
isolated UDP TURN tests validate this subset. Stock-host playback was confirmed
by the user over direct and relay paths, including automatic direct-to-relay
retry where direct access was unavailable.
WebRTC sessions do not use this subset. Removed offline Intel investigations
are not part of the live BUD decoder path.

UDP totals are published from the native IO shell, independently of delivered
media/control counters. The narrow socket-accounting copy and its exact scope
are documented in ../lowlat-net/EVOPARSEQ.md. Browser/SCTP transport code is
omitted from this optional native-only dependency; our WebRTC path is unchanged.

TURN stream integration: native IO/configuration and established-carrier telemetry
from lowlat PR #2, revision befd5a29c7a331111f11378c0b5623de788e9769.
No SDK ABI or decoder API is imported. Common/crypto remain at the original pin;
core is the sibling pinned copy with TURN stream additions and unused host-only
modules omitted, as recorded in its EVOPARSEQ.md. See
../../../../../../docs/native-core.md for scope and verification.

EvoParseq's optional USB HID input is local adapter code, not an imported
lowlat SDK input implementation. One device lifecycle and bounded interrupt
queue serve mouse-only, keyboard-only or combined collections. The application
retains local UI routing; selected HID inputs suppress their normal Parsec
counterparts. Protocol/control, descriptor, relative/absolute mouse and physical
keyboard state are isolated from media and crypto. No new dependency, host
patch, probe launcher or per-input-event logging is introduced. See
../../../../../../docs/native-core.md for behavior, scope and validation.

On 2026-10-22, the shell also drains successfully queued control/HID output
produced after its normal IO pass, before entering another wait. The driver
tracks only successful enqueues; the same carrier handles UDP batching and
TCP/TLS backpressure. No protocol deadline, HID report cadence or transport
ordering is changed. Temporary input-trace instrumentation is not retained.

## Native core branch (2026-10-24)

The receive-only cursor parser/cache is adapted from upstream revision
`e548d48c81ae50937940b2b47f700557cd9d50a4` (MIT). The cursor cache is
only enabled by the native-core feature; no upstream PNG decoder, SDK, renderer,
host-side cursor encoder or extra dependency is imported. Existing Win32 cursor
creation and PNG decoding remain the application's.

This notice documents the application-owned changes used by EvoParseq.

The 2026-10-25 native-core integration adds an explicit combined HEVC 4:4:4
10-bit capability. Initialization and later encoder requests retain both flags
only when the application verifies the combined GPU path and its fallback
rows. No decoder, exporter or vendor SDK is added to this transport subset.

On 2026-10-26, video encode telemetry adopts the kind filter from upstream
`0512edbc662fb75d1f816b0b9799eb2f1df88424`: opcode 21 kind 1 updates video timing; audio kind 2 and unknown
kinds do not. The existing last-nonzero-video value and per-session reset
behavior are retained. This is telemetry-only; no packet timing, decoding,
frame copies or GPU work changes.

## Windows additional display streams

The Windows external-video adapter can receive stream 1/2 on independent BUD
channels 3/4. Its extra receive rings use the original bounded ring size and
sequence/ACK policy. Compressed handoff pools are created only after an explicit
stream-enable request and retained within the connection's fixed two extra slots.
No SDK decoder, audio pipeline, timer, socket or rendering pass is added. The
existing keyframe-aligned catch-up helper accepts the selected channel; the
primary stream passes its original channel 1. This code is `cfg(windows)`;
Android/Linux retain their original single-stream receipt behavior.
