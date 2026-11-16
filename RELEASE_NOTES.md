EvoParseq v0.9.8-dev.3 - Linux multi-display, HEVC Main10 and shared-core cleanup

This development preview builds on **EvoParseq v0.9.8-dev.2**. It includes
Windows x64, the distribution-signed Android TV APK and the experimental Linux
x86_64 Flatpak. All three application versions are **0.9.8-dev.3**.

**This is a development preview. EvoParseq v0.9.7 remains the stable release.**

## What's new and improved

- **Linux GPU selection:** enumerate available Intel, AMD and NVIDIA Vulkan
  adapters, save the selected adapter identity and restore it on startup.
  The command-line adapter override remains available.
- **Linux multi-display:** open additional host-listed outputs over the existing
  BUD or WebRTC session, with independent Vulkan Video decoders and per-display
  resolution, fullscreen, input and close controls. Keep a single connection and
  audio pipeline; closing an additional window closes only that stream.
- **Native Wayland window dragging:** transfer held-button motion between stream
  windows using the core Wayland drag-and-drop protocol and the existing shared
  clipboard data device. Preserve first-click input, final target/position,
  button release, cancellation and clipboard access. Show native pointer feedback
  between stream windows without permanently locking the mouse or using XWayland.
- **Linux HEVC Main10:** decode supported 10-bit HEVC into original P010 images
  and sample them in the existing Vulkan presentation pass. Fix compressed-video
  size validation so valid encoded pictures use the bounded video budget rather
  than the smaller control-message limit.
- **Linux session controls:** wire the available existing video/audio controls,
  including fixed/dynamic host FPS, orientation, multichannel/PCM audio requests
  and Vulkan VRR presentation requests. Keep capability-dependent controls gated.
  Settings menus now open the settings page from any display, and the volume
  track uses the same full width as Windows.
- **Shared core and cleanup:** share desktop session action labels/bounds and
  statistics tab definitions, and move Android's host frame-rate hint policy into
  shared Rust. Remove unused statistics scaffolding, the unconsumed Windows
  character-event/UTF-16 path, redundant resize-event payloads, obsolete cursor
  state, temporary debugging traces and the forced-CPU diagnostic switch. Vulkan
  format tests now exercise the
  production NV12/P010 compatibility predicate.
- **Virtual-display guidance:** document parsec-vdd as the recommended host-side
  display manager. It uses Parsec's own VDD; an existing Parsec-installed driver
  does not need reinstalling. Other compatible host virtual-display drivers also
  work with already configured extended outputs.

## Compatibility and validation

Linux remains a Wayland/Vulkan Video preview. The maintainer has confirmed Linux
playback on Intel, AMD and NVIDIA, including an NVIDIA RTX 3060 on a hybrid
Nobara system. Linux HEVC playback is confirmed on the tested Intel notebooks;
Main10 is confirmed on the Intel ARL test system. Cross-display dragging has been
live-confirmed on Zorin OS 18/GNOME. This does not validate every driver, GPU or
compositor. The earlier Pop!_OS NVIDIA surface-presentation failure is not a
universal NVIDIA failure; that particular environment remains unverified.

Linux output is **SDR**, including Main10. Its YUV-to-RGB presentation pass is
GPU processing, not the application's strict Zero copy mode. Linux 4:4:4,
Windows-only AI upscaling and unsupported rendering paths remain disabled.
Requesting Vulkan VRR does not prove physical variable-refresh scanout or recover
host capture cadence. No host protocol timing reconstruction is implemented.

Multi-display uses existing host-reported extended outputs (up to three streams
in total). It does not remotely create VDD devices or change host account/driver
permissions. Normal and HID input retain their existing eligibility restrictions.

## Updating

Windows: close the client and extract **EvoParseq-v0.9.8-dev.3-win64.zip**.
The legacy **ParsecWebTurn-v0.9.8-dev.3-win64.zip** contains the same executable
under its previous name. Neither ZIP contains user settings.

Android TV: install the signed **EvoParseq-TV.apk**, or choose the prerelease
channel in Settings > Updates. Its distribution signing identity is unchanged.

Linux: install **EvoParseq-Linux-v0.9.8-dev.3-x86_64.flatpak**. The application ID
and settings directory are unchanged; the saved GPU selection is retained.
