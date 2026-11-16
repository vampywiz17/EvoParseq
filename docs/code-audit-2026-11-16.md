# Dev source and platform-boundary audit — 2026-11-16

Scope: owned Windows, Linux and Android Rust/Kotlin sources, build manifests,
CI/packaging entry points and their shared source imports on dev. This is a
source/usage/build review, not proof that every platform/device path is exercised
on hardware. Third-party vendor source is checked for selected patches and
consumers; unused upstream APIs/tests are not fork-pruned.

## Completed changes

- Removed the unreferenced Linux StatRow/STATS/additional_stats scaffolding.
  Live diagnostics still use the shared stats_view presentation, not placeholders.
- Removed Windows Text events, their UTF-16 decoder/mutex and tests of that unused
  route. The session never consumed these events: physical USB usages and the
  host keyboard layout remain the implemented typing path. WM_CHAR is consumed
  without creating remote input. Clipboard paste and local text controls remain.
- Removed clipboard counters/mutex bookkeeping that only tests consumed. The
  test adapter records calls inside cfg(test) and verifies rejection before OS IO.
- Replaced unused resize dimensions in the event FIFO with a notification. Client
  resolution matching already reads the current viewport from the view.
- Windows and Linux now consume one session action table and stats tab model.
  Android's implemented constant-host-FPS hint rule/tests now live in the shared
  session preference module; its Surface API call stays in Android.
- Removed the test-only original_nv12 helper. Both live Vulkan format validation
  and the offline NV12/P010 checks use the same compatibility predicate.
- Removed the temporary forced-CPU Vulkan synchronization environment switch.
  The capability-selected synchronous fallback remains for devices without
  timeline semaphores; normal playback selection is unchanged.
- The preceding Wayland cleanup removed obsolete peer-restore stream tags,
  diagnostic cursor/confinement traces and unused fixture fields. Native DnD,
  the shared clipboard device and local gap pointer feedback remain necessary.

## Boundary review

Shared Rust owns authentication/protocol validation, ICE/TURN/certificate policy,
encoded-video parsing/recovery and bounds, audio buffering/layout/recovery,
keyboard/pointer/controller wire policy, clipboard permissions, host video/display
configuration/confirmation, settings validation and desktop presentation models.
Windows D3D11/vendor decode/WASAPI/DPAPI/Win32, Linux Vulkan/PipeWire/protected
storage/Wayland and Android MediaCodec/AAudio/Keystore/JNI/TV widgets remain native
adapters. Adapter-specific lifetime, resource ownership and asynchronous UI
coordination are retained rather than introducing speculative cross-platform
queues, workers or generic backend layers.

Direct dependencies were checked against owned/shared source consumers and build
features. No additional unused direct dependency was established. Implicit macro
consumers such as windows-core are retained. Unsupported persisted preferences
and disabled desktop controls preserve compatibility/UI parity; they are not
represented as implemented playback features.

## Validation and retained tests

The existing Windows/Android/Flatpak builds, Clippy/dependency checks, measured
Sonar gate and native GNOME 46 lifecycle fixture gate the release. Retained tests
cover actual framing/validation, bounded queues, credential protection, transport
policy, media formats, device/focus/input release and clipboard lifecycle. They
remain isolated from real accounts. Only tests of removed non-consumed helpers
were removed; active guards and coverage thresholds were not relaxed.
