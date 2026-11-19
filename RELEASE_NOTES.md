EvoParseq v0.9.8 - Multi-display streaming and Linux Flatpak

This stable release builds on **EvoParseq v0.9.7**. It includes Windows x64
packages, a distribution-signed Android TV APK and an experimental Linux x86_64
Flatpak. All three clients report **0.9.8**.

## What's new and improved

- **Windows and Linux multi-display:** open up to three host-reported outputs
  over one WebRTC or BUD connection, with one decoder per display and one
  shared audio pipeline. Each additional display has independent resolution,
  fullscreen and input controls. Resolution changes refresh the host settings
  before updating the selected display, preserving other outputs' settings.
  Closing an additional window leaves the other streams running.
- **Cross-display input:** Windows sends the clicked display and position before
  forwarding the activating click and restores the cursor on the destination
  display. Linux uses native Wayland drag-and-drop for held-button handoff
  between stream windows while retaining focused clipboard access.
- **Linux Vulkan Video:** H.264, HEVC Main and capability-gated HEVC Main10
  playback, with saved Intel, AMD or NVIDIA GPU selection. Decode and
  presentation share one Vulkan device and use GPU timeline synchronization
  where supported. NV12 and P010 decoder images feed the GPU color conversion
  pass directly. Main10 presentation is SDR.
- **Linux desktop and audio:** shared launcher, Settings, session overlay and
  connection statistics, with secure remembered sign-in through the Secret
  portal. PipeWire provides local volume and mute, automatic device recovery,
  stereo Opus and BUD PCM or supported 5.1/7.1 output. Normal keyboard and mouse,
  opt-in BUD HID, focused text clipboard and controllers with rumble are wired.
- **Linux networking and session controls:** shared STUN, custom TURN and
  Cloudflare TURN settings, direct-only and Force TURN modes, and optional
  DSCP/socket-priority hints. The overlay exposes display, resolution, bitrate,
  frame-rate and frame-pacing requests. Vulkan adaptive presentation is available
  when supported.
- **Windows reliability:** fix a BUD teardown deadlock that could freeze the
  client and remove duplicate scrollbars from information pages.
- **Desktop sign-in:** accept spaced six-digit authenticator codes and show clearer
  messages for rejected fields, confirmation requirements and rate limits.
- **Linux updates:** check GitHub for versioned Flatpak releases, verify the
  bundle and open the system installer for confirmation.
- **Shared client code:** reuse desktop UI definitions and connection statistics
  views, and share host frame-rate hints with Android TV. Android versionCode is 35.
- **Virtual displays:** the guide recommends ParsecDisplay / parsec-vdd for
  configuring host-side virtual outputs using Parsec's installed driver. The
  client opens existing extended outputs reported by the host.

## Compatibility and validation

Windows requires Windows 11 x64 and Direct3D feature level 11.0+. Android TV
requires Android 10 (API 29) or later on ARMv7 or ARM64. The Linux preview
requires Wayland, PipeWire, Flatpak 1.15.6 or later and a graphics driver with
compatible Vulkan Video support. The maintainer tested Linux H.264 on Intel,
AMD and NVIDIA, including RTX 3060 on hybrid Nobara, and HEVC on tested Intel
hardware. Cross-display dragging was tested on Zorin OS 18/GNOME. GPU support
and Wayland presentation depend on the driver and compositor. Requesting
adaptive presentation does not verify physical VRR. Linux 4:4:4 and Windows AI
video processing are outside the Linux preview.

## Requirements and upgrading

**Windows:** close the client and extract **EvoParseq-v0.9.8-win64.zip**.
**ParsecWebTurn-v0.9.8-win64.zip** contains the identical executable under the
old filename. The packages exclude user settings and the optional NVIDIA runtime.

**Android TV:** install **EvoParseq-TV.apk** or use **Settings > Updates**.
**EvoParseq-TV-v0.9.8.zip** includes the same APK, setup guide and license notices.
The distribution signing certificate is unchanged.

**Linux:** install **EvoParseq-Linux-v0.9.8-x86_64.flatpak**. The existing
application ID and private settings directory are retained.

Verify downloads with `SHA256SUMS.txt`. [Changes since v0.9.7](https://github.com/vampywiz17/EvoParseq/compare/v0.9.7...v0.9.8)
