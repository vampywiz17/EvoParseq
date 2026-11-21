EvoParseq v0.9.9-dev.1 - Phaze preview and expanded Parsec bitrate options

This development preview builds on **EvoParseq v0.9.8**. It includes Windows
x64, the distribution-signed Android TV APK and the experimental Linux
x86_64 Flatpak. All three clients report **0.9.9-dev.1**.

**This is a development preview. EvoParseq v0.9.8 remains the stable release.**

## What's new and improved

- **Phaze sign-in and streaming:** choose Parsec or Phaze on the sign-in
  screen, discover hosts for the selected service and see the active service
  after signing in. Connect to a stock Phaze host using the shared Rust
  protocol implementation. Phaze adds direct UDP and configured TURN, including
  UDP, TCP and TLS for the connection from the client to the relay.
- **Phaze video and sound:** decode H.264 and HEVC through the existing native
  video pipelines on Windows, Linux and Android TV. Windows can request HEVC
  4:4:4 on compatible systems. Linux and Android TV use 8-bit 4:2:0.
  Phaze uses stereo AAC-LC by default and offers uncompressed stereo PCM
  after reconnecting with that preference.
- **Phaze desktop controls:** use service-specific settings, switch between
  host-listed outputs, and request a bitrate limit or frame rate from the
  session overlay. Windows also exposes host-authorized guest controls and
  Ctrl+Alt+Del. The Android TV overlay shows the selected service and sent
  stream limits using the existing TV controls.
- **Phaze input:** forward normal mouse and keyboard input and use text
  clipboard sync on desktop. Apply the host's cursor shape, visibility and
  relative mouse mode. Windows positions the pointer before forwarding the
  activating click. Input batching preserves click order and limits queued
  mouse movement under load.
- **Phaze virtual USB HID on Windows and Linux:** optionally use a virtual
  mouse, keyboard or both with a host that permits USB forwarding. The HID
  mouse targets the primary display. Preserve normal input when HID is
  unavailable and release held input on mode transitions and disconnect.
  The implementation includes corrected mouse button and vertical wheel mappings.
- **Phaze connection recovery and stats:** keep authenticated sessions alive
  and reopen the stream after supported decoder resets. Report measured
  encoded video and audio traffic separately from requested limits. Show a
  local RTT estimate when recent authenticated command acknowledgments are
  available. Use packet-train feedback to inform the host's bandwidth estimate.
  Android ARMv7 uses RustCrypto AES-GCM for Phaze DTLS.
- **Parsec bitrate options:** add 100 and 150 Mbps request presets on Windows,
  Linux and Android TV. Keep Parsec and Phaze bandwidth preferences separate.
  Phaze offers 5, 10, 20, 30, 40 and 50 Mbps presets and preserves prior saved
  values. A requested limit does not guarantee the measured stream bitrate.
- **Build and packaging:** include Phaze audio and crypto notices with the
  applicable packages. Add Flatpak and Android TV validation to PRs targeting
  dev, with a debug APK that needs no release signing key for PR checks.
  Expand tests for Phaze transport, input, audio and stream controls.

## Compatibility and validation

Windows streaming was tested with a stock Phaze host, including video, audio,
mouse input and Cloudflare TURN over UDP. Phaze playback was also exercised on
Linux Flatpak and Android TV. Codec, hardware, USB and relay combinations vary.
The virtual USB HID mouse and keyboard were exercised on Windows. The Linux
port and the final button and wheel mappings need wider live validation.

Phaze streams one active host output at a time. Use a host with an active
physical or virtual display. Parsec retains multi-display on Windows and Linux.
The release does not claim automatic Phaze virtual display creation, physical
USB passthrough or Phaze Main10 support.

## Updating

Windows: close the client and extract **EvoParseq-v0.9.9-dev.1-win64.zip**.
The legacy **ParsecWebTurn-v0.9.9-dev.1-win64.zip** contains the same
executable under its previous name. The ZIPs exclude user settings.

Android TV: install **EvoParseq-TV.apk** or select the prerelease channel in
Settings > Updates. The APK uses the existing distribution signing identity.

Linux: update or install **EvoParseq-Linux-v0.9.9-dev.1-x86_64.flatpak**. The
application ID and settings directory remain the same.
