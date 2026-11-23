# Pinned native protocol core with isolated TURN stream additions

MIT-licensed nomi-san/lowlat core at
`5f8db5689e55de0d396b0520824caa9fc053dd86`. The original license is retained.
This is the pinned open-source protocol dependency.

Only `relay.rs`, `turn.rs` and new `turn/stream.rs` receive the TURN changes from
[lowlat PR #2](https://github.com/nomi-san/lowlat/pull/2), pinned to
`befd5a29c7a331111f11378c0b5623de788e9769`. All other retained protocol code
stays at the original pin, apart from the omissions below, avoiding unrelated newer upstream changes. Unused
cursor/PNG decoding, controller report conversion, microphone and inactive
PMTU-probe modules and their fixtures are omitted. Existing framed input remains
forwarded untouched by the transport-only client.

The unused host encoder bitrate controller and its experimental strategies/tests
are also omitted from `congestion.rs`. The send ring's original window floor,
three staleness levels and default resolution are retained without tuning changes.

The changes distinguish reliable client-to-TURN transactions from UDP retries,
bound their deadlines, preserve nonce/authentication recovery and frame TURN
STUN/ChannelData on streams. Allocation and peer traffic remain UDP. The existing
240-second permission/channel renewal policy is retained. Tests cover both modes.
No SDK ABI, decoder, host encoder, renderer or account API is imported.

## Native core branch (2026-10-24)

The receive-only cursor parser/cache is adapted from upstream revision
`e548d48c81ae50937940b2b47f700557cd9d50a4` (MIT). The cursor cache is
only enabled by the native-core feature; no upstream PNG decoder, SDK, renderer,
host-side cursor encoder or extra dependency is imported. Existing Win32 cursor
creation and PNG decoding remain the application's.
