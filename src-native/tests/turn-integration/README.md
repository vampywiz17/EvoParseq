# Isolated TURN regression fixture

This separate crate tests the same patched ICE/WebRTC crates as the Windows app
without linking the Parsec core, UI, codecs or production settings. It is never
shipped. From the repository root, `bash src-native/tests/turn-integration/run.sh`
runs on an isolated Ubuntu CI runner
with coturn, OpenSSL and Rust installed. The script installs a one-day fixture CA
in that runner's trust store; do not run it on a personal machine unnecessarily.

Coverage: RFC 8656 stream framing/padding under fragmented and coalesced reads,
close during a partial frame, malformed framing, untrusted certificate rejection,
trusted certificate hostname mismatch, and actual bidirectional 64 KiB messages
on three negotiated SCTP channels through coturn using UDP, TCP and TLS. Relay-only
is used in the fixture to prove traffic traverses TURN; production defaults to ICE
`all`, with an explicit Force TURN relay setting for `relay` policy.

Trickle-ICE tests additionally verify the opt-in UDP > TLS > plain TCP preference
with all transports available, UDP unavailable, and UDP plus TLS unavailable.
Each case verifies the selected relay protocol and actual SCTP traffic rather
than treating a successful TURN allocation as proof of peer connectivity.

These fixtures prove transport interoperability, not that every Parsec host,
provider or corporate network will accept a session. Test a real host separately.

See the [interoperability and test-data policy](../../../docs/INTEROPERABILITY.md).
