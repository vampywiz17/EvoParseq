# Experimental Phaze media/input and TURN investigation

This work is confined to codex/phaze-protocol and the Windows adapter.
The protocol source is original Rust, derived from wire-format observations
under authorized interoperability testing. No proprietary Phaze implementation
body, SDK or binary is included. Public physical-key/button constants are described
in [libmatoya's header](https://github.com/snowcone-ltd/libmatoya/blob/master/src/matoya.h).

## Audio and input

The hello requests raw audio (option bit 0); it does not request a virtual display
layout. Audio opcode 7 contains an encoded flag, big-endian stereo frame count
and samples. The implemented path accepts only unencoded, interleaved 48 kHz
signed 16-bit little-endian stereo with an exact bounded length. It passes PCM
to existing WASAPI playback; there is no new codec dependency or native Phaze code.
Raw samples use 1.536 Mbit/s before overhead. Compressed Phaze audio is not
silently interpreted as PCM. Existing Parsec Opus decoding is unchanged.

Input opcodes 8/9/10/11 carry keyboard/wheel/button/motion messages. Each has
five big-endian words: selected output plus four event fields. Absolute motion
uses normalized 0..65535 coordinates in the presented letterbox. Keyboard events
use physical PC set-1 scans and side-specific modifiers; host text layout applies.
The public vocabulary and wire serialization live in the platform-independent
Phaze crate. Windows drains its existing event queue and retains focus, viewport,
fullscreen and detach policy. It positions before an activating button press,
releases remote state on focus loss/detach/queue failure and sends reset opcode 13.
No Parsec input packet is forwarded to a Phaze host.

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
Live native-host interoperability must still be confirmed by the user.
