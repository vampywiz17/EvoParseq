# lowlat-based transport

The optional **lowlat** setting selects this lowlat-based implementation for
interoperability with compatible Parsec hosts. It handles session signaling and
compressed media parsing. WebRTC remains the default transport.
[Research and test-data provenance](../../../docs/INTEROPERABILITY.md)

This crate has no decoder,
renderer, GPU exporter, audio device or account UI; the application supplies them.

Direct UDP, automatic relay fallback and Force TURN are retained. Client-to-TURN
carriers support UDP, TCP and verified TLS; the host leg remains UDP. The app
reuses its configured TURN URLs, preferring UDP, TLS, then TCP (port 80 first).
Failed startup attempts may advance to the next URL. Credentials and lifecycle stay isolated;
there is no silent fallback from lowlat to WebRTC. Compressed media enters the same
native decoder/audio pipeline as WebRTC, without exporting decoded pixels.

The transport-only lowlat-client and lowlat-net subsets are pinned and documented
in vendor/*/EVOPARSEQ.md. Their MIT licenses are retained. Regression tests
exercise encrypted direct and UDP TURN paths using synthetic local credentials.
One-off GPU investigations and their optional dependencies have been removed.


TCP/TLS QoS: optional default-off connected-socket marking through the shared
`evoparseq-socket-qos` platform adapter. Windows uses non-adaptive qWAVE
AudioVideo; other targets are no-ops. lowlat IOCP and WebRTC Tokio/TLS framing,
verification and media processing are preserved. Failure never rejects a stream.
