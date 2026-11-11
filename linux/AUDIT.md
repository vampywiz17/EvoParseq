# Linux branch cleanup review — 2026-11-05

This audit was prepared on the former `codex/linux-flatpak` branch. That work
is now integrated into `dev`; the original audit scope is retained below.

Scope: application sources and manifests on `codex/linux-flatpak`, including the
shared Rust changes and Android/Windows adapters. This is a source/build review,
not a claim of hardware validation on every platform.

## Removed or consolidated

- Removed saved arbitrary launcher geometry. Fixed canonical constraints now
  survive asynchronous fullscreen/unmaximize acknowledgments after Disconnect.
- Restricted the codec/status fixture adapters to `cfg(test)` and removed a
  decoder test convenience method with no remaining callers.
- Removed retired Settings labels/help duplicated by the active form descriptors;
  Windows-consumed layout constants remain shared.
- Removed unused Linux imports, a retired format-query variable and unused
  profile constant reexports; retained the Windows-only Resolution alias.
  Declared shared Windows cfg names without enabling their features on Linux.
  Made UI stroke literals explicitly f32 for future compiler compatibility.
- Extracted duplicated GitHub transport, redirect bounds, metadata size bounds
  and SHA-256 streaming verification into shared Rust `release_http`. Android and
  Linux retain separate package/channel/installer policies.
- Shared desktop About content and layout with Windows instead of maintaining
  platform copies of the prose or rectangles. Windows output is preserved.
- Consolidated Vulkan surface eligibility/format queries across startup, probe
  and session allocation. Retained explicit unsupported DPB-mode failures.
- CI produces one versioned Flatpak, not two copies of the same binary.
- Corrected stale README statements about audio recovery, input, clipboard,
  orientation, Help actions and resizable Connection stats.

## Retained deliberately

- Offline `cfg(test)` fixtures cover framing, authentication validation, bounded
  queues/recovery, codec references/parameters, input releases, clipboard echo
  suppression, protected persistence and controller/rumble lifecycle. They do
  not connect to real accounts, contact TURN services or enter release binaries.
- `linux/tests/data` fixtures exercise real H.264/HEVC parsing and unsupported
  profiles; their license and provenance remain included. They are not installed
  into `/app` or read by production playback.
- `--probe-video` is an explicit capability diagnostic, not automatic per-frame
  instrumentation. It creates no decode session and performs no pixel readback.
- The two pinned egui patches remain necessary for Wayland cursor invalidation,
  deferred clipboard repaint and public surface latency requests. Their upstream
  source/licenses stay intact; unused optional upstream APIs are not fork-pruned.
- All 29 Linux direct build/test dependencies have source consumers, including
  shared-path modules. Android direct dependencies also have source consumers.
  Windows `windows-core` is required by the `IMMNotificationClient` implement
  macro expansion, even though no handwritten `windows_core::` reference exists.
  No dependency was removed merely because a simple text scan missed its use.

## Media/platform boundaries

- BUD media parsing, ingress, Opus/PCM buffering, lifecycle and policy remain in
  shared Rust. Vulkan Video/PipeWire, D3D11/WASAPI and MediaCodec/AAudio remain
  explicit platform backends. Normal/HID and controller wire policy is shared;
  native event/device/motor access stays platform-specific.
- Live Linux presentation samples original NV12 decoder images in the existing
  output pass. No decoded-image copy/blit/readback, intermediate RGBA video image,
  additional render pass or cross-GPU transfer was introduced. Shader conversion
  still costs GPU work and is not the application's strict Zero copy mode.
- No runtime Test WebRTC/BUD/audio preview path remains in the product GUI.
  Test-only convenience entry points are excluded from production builds.
- No new background telemetry sampler, input polling worker or media timer was
  added. Update networking/storage/portal work runs only on explicit user action,
  outside the UI/audio/video hot paths.

`main` and `dev` are not changed. CI validates this branch's Windows and Android
builds where shared code was touched; user hardware validation remains necessary
for native window/portal behavior and AMD/NVIDIA interoperability.
