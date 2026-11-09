EvoParseq v0.9.5-dev.2 - AI upscaling, diagnostics and input fixes

This Windows prerelease builds on **EvoParseq v0.9.5-dev.1**, with optional GPU
upscaling, clearer diagnostics and input/recovery fixes for existing, unmodified
Parsec hosts.

**This is a development preview. ParsecWebTurn v0.9.4 remains the stable release.**

## What's new and improved

- **Simultaneous mouse-button holds:** fix capture being reacquired when the
  stream window already owns it. Holding left and right together no longer
  generates premature releases. Independent releases, focus-loss cleanup and
  relative-camera transitions retain their existing behavior.
- **Input under load:** combine adjacent absolute mouse positions and sum
  relative camera movement without crossing button, wheel, focus or mode
  boundaries. Reserve queue capacity for other input and fail safely on
  unrecoverable overflow instead of leaving held input behind.
- **Clipboard reliability:** retry a failed Windows clipboard write using only
  the newest remote text, with bounded retries. Cancel obsolete writes after
  focus/permission changes or a new local copy.
- **Organized Connection stats:** group diagnostics into Overview, Network,
  Video, Audio and Timings tabs with a fixed header and separate scrolling.
  Add recent two-second video timing averages, expire stale values and show
  decoded FPS separately from presentation-submission FPS.
- **Native video stall handling:** supervise decoder/device setup, processing,
  presentation and teardown. Keep driver-owned resources on their owning worker
  and prevent accumulating replacement workers. An unrecoverable native timeout
  requires restarting the app.
- **DXGI VRR:** add optional variable-refresh presentation to the
  normal DXGI path used by vendor and Media Foundation decoders. Check tearing
  support, preserve configuration through resize/precision changes and fall back
  without adding decoded-frame copies. Physical panel activation depends on the
  display, driver and Windows settings.
- **Experimental AI upscaling:** one GPU-dependent selector offers NVIDIA RTX
  VSR quality levels or Intel VPL AI. Off by default, upscaling only, with normal
  GPU rendering retained on unsupported configurations. NVIDIA RTX 3060 playback
  is live-confirmed. Intel support is implemented but **not hardware-validated**;
  installed runtimes may reject the AI filter.
- **Optional NVIDIA runtime:** keep the VSR DLL outside the EXE. With explicit
  consent, download the pinned official NVIDIA RTX Video SDK 1.1.0 payload and
  verify both archive and DLL SHA-256 hashes. Reuse verified caches and store the
  runtime under `%LOCALAPPDATA%/EvoParseq/vsr/1.1.0`, without administrator rights.
  Intel uses its installed driver runtime and downloads no DLL or model.
- **VSR startup/recovery:** preserve warmed effect state on the same GPU during
  decoder-reference recovery, reducing repeated setup and black-picture
  interruptions. Genuine native errors still recreate the decoder; repeated
  steady-state overload disables the effect rather than growing the queue.
- **AMD HEVC compatibility:** check coded Main/Main10 dimensions once through
  D3D11 on the selected GPU before AMF initialization. Unsupported configurations
  take the existing logged Media Foundation fallback; visible-size AMF setup and
  normal frame processing remain unchanged.
- **WebRTC cryptography and maintenance:** update the existing DTLS engine's
  P-256/P-384 dependencies while preserving the WebRTC/ICE/SCTP integration,
  certificate checks, record ciphers and transport policy. Update Tokio to
  1.54.1, webpki-roots to 1.0.11 and serde_json to 1.0.154. Remove unused legacy
  control callback payloads and unnecessary copies.

## Compatibility and validation

**AI upscaling, vendor HEVC, 10-bit HEVC, HEVC 4:4:4 and multichannel audio remain experimental.** For conservative
operation, use H.264, stereo Opus and Automatic timing with optional effects off.

AI upscaling is **GPU processing, not the application's Zero copy mode**. It
adds GPU resources/work and an output copy, and may increase latency. Intel AI
currently supports 8-bit SDR normal rendering only; actual runtime support is
required. VRR application flags do not prove physical variable-refresh scanout.
10-bit output remains SDR, not HDR. No universal performance improvement is claimed.

Existing host permissions, HID primary-display restrictions and codec/driver
requirements still apply. The DTLS upgrade does not replace the separate BUD
cryptography dependency chain. See README for the full compatibility limits.

## Updating

Close the client and extract **EvoParseq-v0.9.5-dev.2-win64.zip**. Keep your
`settings.json` under the same Windows account; native encrypted remembered
sessions and compatible settings are retained. Protected credentials are not
portable between machines or Windows accounts.

**ParsecWebTurn-v0.9.5-dev.2-win64.zip** contains the same executable under the
legacy name for existing shortcuts. Both ZIPs exclude settings.json; the NVIDIA
runtime is not bundled. Run only one client and keep a backup while testing.

This is a **Windows x64 prerelease**, installed manually. The main branch and
stable ParsecWebTurn v0.9.4 remain unchanged.
