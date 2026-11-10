# EvoParseq changelog

## v0.9.7 - Release security reports (2026-11-10)

Stable maintenance release since v0.9.6. Publish VirusTotal results for final
release binaries in the release notes and SECURITY-SCAN.md/JSON attachments.
Keep releases in draft until completed scans pass; findings and incomplete/error
results block publication. Report unsupported engines and analysis failures explicitly. Add
offline scanner tests, a real Flatpak extraction fixture and measured Python
coverage to the strict SonarQube gate. Flatpak support in the scanner prepares
future releases; this stable release includes Windows and Android TV only.

Application behavior, permissions and signing keys are unchanged. Only version
metadata advances to 0.9.7 (Android versionCode 31) so in-place updates work.
Scan reports are not a safety certification or Google Play Protect approval.


## v0.9.6 - Controllers, Android TV and local audio controls (2026-11-07)

This stable release adds shared controller support, Windows playback volume
controls and the initial Android TV client. The changes below cover implemented
features and fixes since the previous stable release, **EvoParseq v0.9.5**.
Windows x64 packages and a distribution-signed Android TV APK are included.
Linux alpha builds and the proposed macOS port are not part of this release.

## What's new and improved

- **Shared controller support:** one Rust state and protocol pipeline forwards
  up to four controllers over WebRTC or BUD. Windows uses native XInput;
  Android reads standard gamepad/joystick events. Forward buttons, D-pad,
  sticks and analog triggers; preserve trigger precision and avoid duplicate
  button mapping. Device-supported vibration uses the platform adapter.
  Joining, focus changes, held-state release, unplug and disconnect are handled
  explicitly; local menus release held controls without repeatedly removing
  the host controller. Home/Guide remains owned by the operating system.
- **Windows local audio controls:** the session overlay now has Mute audio and
  a 0-100% volume slider for either transport. Muting preserves the chosen level;
  moving the slider while muted keeps mute enabled. These controls affect only
  client playback through WASAPI, not the host's volume, and reset when the
  client restarts.
- **Android TV interface:** add a Google TV / Android TV-style launcher and
  computer picker, remote-friendly sign-in and two-factor-code entry, grouped
  Network/Video/Audio/Diagnostics/Device/Updates settings and device capabilities.
  Correct text contrast, focus indicators, IME actions and launcher artwork.
  Keep focused cards within their bounds, prevent settings overlap and retain
  readable category buttons in a bounded scrolling sidebar with Back accessible.
- **Protected TV sign-in and settings:** remember only the session token using
  AES-256-GCM and a non-exportable Android Keystore key in private no-backup
  storage. Passwords and two-factor codes are never saved. Sign out clears local
  credentials and requests server-side revocation. Relay credentials are
  separately protected and excluded from backup and diagnostic output.
- **Android video and audio:** H.264 and supported HEVC 8/10-bit 4:2:0 decode
  through hardware MediaCodec to a Surface. Share codec initialization, geometry
  validation, bounded queues and keyframe recovery. Decoded pixels do not cross
  JNI as Bitmaps or undergo CPU readback. Shared Rust audio decoding feeds a
  bounded nonblocking PCM handoff and 48 kHz AAudio pull output.
- **WebRTC and BUD on TV:** reuse Rust authentication, discovery, signaling,
  connection preferences, host settings and media handling. Support direct/STUN,
  Cloudflare or custom TURN, and Force TURN. Preserve WebRTC ICE selection and
  BUD direct UDP/configured UDP-TLS-TCP relay carriers. Transport selection stays
  explicit, with no silent switch. Keep useful transport errors and stop reasons
  visible after returning to the picker; distinguish empty WebRTC messages from
  channel EOF and retain bounded codec/output recovery.
- **Phone/computer relay setup:** the TV can open an opt-in HTTPS configuration
  page on its private LAN. Verify the displayed certificate fingerprint, enter
  the pairing code, submit STUN/Cloudflare/custom TURN settings, then approve
  them on the TV. Preserve origin/Host checks, bounded submissions and expiry.
  Setup stops on exit, stream start or the five-minute deadline. No external
  provisioning service, public setup listener or port forwarding is required.
- **TV remote mouse and physical input:** optionally move the pointer with
  D-pad; short OK clicks, two short presses double-click, held OK right-clicks,
  and OK plus direction drags on compatible remotes. Physical keyboards/mice use
  normal Parsec input and optional mouse capture. Ctrl+Shift+M releases input
  and opens controls. Correct cursor size, hotspot, edge mapping and input-device
  ownership while keeping gamepad controls separate. Cursor and overlay changes
  no longer rebuild the video Surface or terminate the session.
- **Dismissible TV controls and cadence hints:** a compact overlay reports
  available FPS, selected-pair RTT, incoming bitrate, actual codec/depth and
  direct/relay routing. Auto-hide after five idle seconds; retain manual Hide,
  Disconnect and Back/Menu access. Add fixed/dynamic FPS preferences. Hint the
  Surface rate from the host-confirmed fixed cadence, clearing the hint when
  dynamic or unknown. Display Android-reported Hz and decoded-output catch-up
  skips without forcing HDMI modes or introducing another frame scheduler.
- **Android audio recovery and buffering:** reopen a failed or disconnected
  AAudio output using bounded retry policy while preserving video and transport.
  Discard stale queued PCM before resuming; fold multichannel audio down when
  recovery selects stereo. Settings offers shared validated minimum/maximum
  buffering from 0-500 ms, with a positive maximum at least equal to the minimum.
  Defaults remain a zero minimum and 100 ms queue ceiling; larger minima add
  latency.
- **BUD PCM and surround on TV:** optionally request raw PCM and verified
  5.1/7.1 output layouts. Match explicit Android speaker positions to the actual
  AAudio route and opened stream; unknown or unsupported layouts use stereo.
  Resolve newer channel-mask APIs optionally so Android 10-12 still loads.
  API 29-31 uses the stereo fallback. WebRTC remains stereo Opus.
- **Signed Android updates:** Settings > Updates checks stable or optional dev
  GitHub releases and downloads the newest compatible APK. Shared Rust verifies
  bounded HTTPS downloads against published size and SHA-256; Android verifies
  package ID, version, OS compatibility and the installed signing certificate.
  Clear obsolete downloads when channels change and use the lifecycle-aware
  system installer. Release APKs use the persistent distribution signing key;
  Android still requires the user's installation confirmation.
- **TV diagnostic export:** optional bounded JSONL operational logging and
  Save/Share diagnostics export a ZIP through Android's document picker or
  temporary read-only sharing. Include up to two rotated 5 MiB logs and basic
  device information; exclude credentials, settings, clipboard, media and logcat.
  Export work stays outside the audio/video callbacks.
- **Shared core and dependency maintenance:** reuse Rust session, validation,
  media ingress, desktop input gestures, controller and audio policy across
  platforms, retaining explicit MediaCodec/AAudio/Keystore and D3D11/WASAPI
  adapters. Use common bounded, video-first BUD batches and promptly drain bursts.
  Remove unused nix AIO features and the resulting unmaintained pin-utils
  dependency without changing upstream Rust implementation or unrelated versions.
  Retain third-party licenses and local-change notices in Windows/Android packages.
- **Quality checks and project documentation:** enforce Sonar Quality Gate
  results with measured Rust and release-packaging coverage, alongside Clippy,
  advisory/license/source checks and workflow lint. Pin the affected CI actions
  to immutable commits and restrict tool downloads/redirects to HTTPS while
  preserving SHA-256 checks. Add Android ARMv7/ARM64, lint and optional-AAudio-symbol
  checks. Stabilize the Windows overlay drag regression fixture by waiting for
  initial window placement before measuring movement, preserving its assertions.
  Retire temporary TV test overlays while retaining useful errors and regression
  tests. README now includes AI-development disclosure, main-branch quality badges
  with dependency-scope context, and a live Star History chart.

## Compatibility and validation

**Windows 11 x64 with Direct3D feature level 11.0+ and compatible drivers** remains
required. Existing Windows video capabilities and their experimental status
remain as documented in v0.9.5; this release does not add another Windows decoder,
HDR, GPU AI mode or verified driver/compositor zero-copy guarantee.

The Android APK requires **Android 10 (API 29)+** and includes **ARMv7 and ARM64**.
The initial TV client has live validation on Xiaomi TV Box S (3rd Gen) EU,
Android 14 with 32-bit ARM userspace: H.264, HEVC 8-bit, NVIDIA-host Main10,
remote mouse and GameSir controller forwarding. Windows wired controller
forwarding is also live-tested. Other devices, audio routes, surround outputs
and rumble combinations require their own testing. DirectInput-only Windows
pads and Sony raw touch/gyro/adaptive-trigger passthrough are not implemented.
The host's normal controller driver and permissions are still required.

In the tested TV setup BUD at 50 Mbit/s was stable; sustained WebRTC 50 Mbit/s
still caused delay/video-channel closure, while around 30 Mbit/s was stable.
This is a transport-specific observation, not a universal hardware bitrate cap.
The AMD-host TV test returned 8-bit. 10-bit remains SDR, not HDR. Android adds
no 4:4:4, Windows BUD USB HID mode, VRR, AISR or Windows-equivalent Zero copy mode.
Surface output avoids application CPU pixel readback; driver/compositor copies
and physical scanout remain unverified. No quantified latency gain is claimed.
Some remotes cannot report simultaneous OK and direction presses for dragging.
Diagnostic export requires a compatible Android document-provider/sharing app.

## Requirements and upgrading

**Windows:** close the client and extract **EvoParseq-v0.9.6-win64.zip**, retaining
settings.json under the same Windows account. Help > Check for updates offers
this stable release. Native settings and protected remembered sessions are
retained; protected credentials are not portable between accounts or machines.
The legacy **ParsecWebTurn-v0.9.6-win64.zip** contains the identical executable
under the old filename. Both ZIPs exclude user settings and the optional NVIDIA
runtime. Older pre-rename updaters may need a one-time manual upgrade.

**Android TV:** install **EvoParseq-TV.apk**, or use **Settings > Updates** in an
existing distribution-signed installation. Stable v0.9.6 has versionCode 30,
newer than the signed dev.5 build; Include prereleases is not required to receive
this stable version. Android confirms installation and may first request install
permission. Normal signed updates retain settings and remembered sign-in.
**EvoParseq-TV-v0.9.6.zip** includes the same APK, setup guide and notices.

Old debug-signed previews, including v0.9.6-dev.1, have a different certificate:
uninstall once and install the distribution-signed APK if the signature is
rejected. Uninstalling erases private settings and remembered sign-in. Already
distribution-signed dev.2-dev.5 builds can update in place and should not be
uninstalled for this upgrade.

Verify downloaded assets with SHA256SUMS.txt. The Windows ZIP and TV APK are
separate applications for their respective platforms. See README, MIGRATION.md
and the Android TV README for setup and platform limits. EvoParseq remains
independent of Parsec, Unity and lowlat; their applicable terms and licenses
remain in force.

## v0.9.6-dev.5 — Audio controls, Android TV updates and recovery (2026-11-06)

This development preview builds on **EvoParseq v0.9.6-dev.1**. The changes below
cover all implemented additions and fixes since that previous dev release.
Internal Android test builds used dev.2-dev.4; this release aligns Windows and
the published Android APK at dev.5 so those installations can update forward.

**This is a development preview. EvoParseq v0.9.5 remains the stable Windows release.**

## What's new and improved

- **Windows local audio controls:** add Mute audio and a 0-100% volume slider
  to the session overlay. Muting preserves the chosen level; changing volume
  while muted keeps mute enabled. Controls affect client playback rather than
  the host's system volume, and apply through the existing WASAPI output.
- **Android TV in-app updates:** Settings > Updates can check stable or dev
  GitHub releases, download a compatible APK and open Android's system installer.
  Shared Rust selects the newest compatible release and verifies the bounded
  HTTPS download against its published size and SHA-256 digest. Android checks
  the package ID, version, compatibility and installed signing certificate.
  Changing release channels clears obsolete prepared downloads; installation
  follows the Activity lifecycle and still requires Android confirmation.
- **Persistent Android release signing:** release APKs now use a protected,
  persistent distribution key instead of temporary CI debug signing. Normal
  updates between distribution-signed builds retain settings and remembered
  sign-in. Older debug-signed installations require a one-time reinstall; see
  Updating below before removing an existing installation.
- **Android audio recovery:** reopen a disconnected or failed AAudio output
  using the shared bounded retry policy while keeping the video/session alive.
  Retire stale queued audio before resuming. A recovery onto a stereo output
  explicitly folds multichannel audio down, with saturation and LFE omitted.
  Native callbacks retain the shared nonblocking PCM handoff.
- **Configurable TV audio buffering:** add an Audio settings group with optional
  minimum/maximum buffer presets using the common Rust validation rules. The
  maximum must be positive and at least the minimum, with a 500 ms ceiling.
  Default behavior remains a zero minimum and 100 ms queue ceiling; increasing
  the minimum adds latency and can help absorb jitter.
- **BUD PCM and surround on Android:** optionally request uncompressed PCM and
  the routed output's verified 5.1/7.1 speaker layout. Match explicit Android
  speaker positions to the actual AAudio device and opened stream. Unknown or
  unsupported layouts fall back to stereo. Newer channel-mask APIs are resolved
  optionally so the APK still loads on Android 10-12. WebRTC remains stereo Opus.
- **Shared BUD media processing:** replace Android's separate media parsing
  with the shared Rust BUD ingress. Preserve bounded,
  video-first media batches, prompt draining of queued bursts, codec/layout
  validation, encoder-change handling and keyframe recovery. PCM and multistream
  Opus use the common decoder/output queue instead of a separate Kotlin path.
- **TV diagnostic export:** add opt-in, bounded operational logging and Save /
  Share diagnostics actions. Export a ZIP containing up to two 5 MiB rotated
  logs and basic device information through Android's document picker or
  temporary read-only sharing. Exclude credentials, settings, clipboard, media
  contents and logcat. ZIP work stays outside audio/video callbacks.
- **TV settings navigation:** add Audio and Diagnostics categories, retain Back
  above a bounded scrolling category list, and preserve readable button heights.
  The Updates entry no longer collapses or loses its caption when the menu is
  taller than the available screen area; focused categories can scroll into view.
- **Dependency and quality maintenance:** remove the unused nix AIO feature
  that brought in the unmaintained pin-utils dependency. Keep the existing
  WebRTC helper version and upstream Rust sources, requesting only the Unix
  network features actually used. Preserve advisory enforcement and unrelated
  dependency versions. Add a blocking SonarQube Quality Gate on every active
  branch except legacy, with measured Rust test coverage and real release-package
  coverage. Instrumented binaries are never shipped. Extend Android CI to catch accidental strong links to newer AAudio
  APIs that would prevent older Android versions from loading the client.

## Compatibility and validation

Windows x64 and Android TV remain the platforms included in this dev release.
Linux development and its separate alpha release stay on the linux-flatpak
branch and are not included in these packages.

The Android APK includes **ARMv7 and ARM64** libraries and requires **Android 10
(API 29) or newer**. The established Xiaomi TV Box S (3rd Gen) / Android 14
streaming and controller observations still apply. Surround output and audio
route/recovery behavior require live validation with the actual TV, receiver
and connected audio devices; CI does not establish hardware compatibility.
API 29-31 use the stereo fallback for optional channel-mask-based surround.
Some TV systems need a document-provider or sharing app to export diagnostics.

Android video remains H.264 / HEVC 8- or 10-bit **4:2:0 SDR** through the existing
MediaCodec-to-Surface path. This release adds no Android 4:4:4, HDR, VRR, AISR or
Windows-equivalent Zero copy mode. The related API research is documentation,
not an implemented or validated capability. Driver/compositor work and physical
scanout remain unverified; no universal performance or latency gain is claimed.
The earlier transport-specific high-bitrate WebRTC limitation remains under
investigation. Existing Windows experimental GPU, HID and audio limits remain
as documented in the README.

Existing host permissions and controller requirements still apply. Windows
volume/mute controls do not change the host's system volume or other clients.
EvoParseq remains independent of Parsec, Unity and lowlat.

## Updating

**Windows:** close the client and extract **EvoParseq-v0.9.6-dev.5-win64.zip**.
Keep your existing settings.json under the same Windows account. The legacy
**ParsecWebTurn-v0.9.6-dev.5-win64.zip** contains the identical executable under
the old filename for existing shortcuts. ZIPs exclude user settings and the
optional NVIDIA runtime. Protected credentials are not portable across machines
or Windows accounts.

**Android TV:** sideload **EvoParseq-TV.apk**, or, from an existing
distribution-signed build, open **Settings > Updates**, choose **Include
prereleases**, then Check / Download / Install update. Android asks you to
confirm installation and may request permission to install apps first. Normal
in-place updates retain settings and remembered sign-in. The accompanying
**EvoParseq-TV-v0.9.6-dev.5.zip** contains the same APK, setup guide and notices.

**Upgrading the old v0.9.6-dev.1 debug APK:** its signing certificate differs
from this release's persistent distribution certificate. If Android rejects the
update signature, remove that older preview and install the new APK. Removing
an app erases its private settings and remembered sign-in; configure it again
afterward. Already distribution-signed dev.2-dev.4 test builds can update
normally and should not be uninstalled for this update.

Use SHA256SUMS.txt to verify downloaded files. This is a **prerelease**, not the
stable updater's latest release. Windows dev previews are installed manually;
Android's updater must have Include prereleases selected to offer this build.
The stable main branch is unchanged. See MIGRATION.md and the Android TV README
for platform-specific setup and update details.


## v0.9.6-dev.1 — Android TV preview and shared controller support (2026-11-02)

This development preview builds on the latest stable release, **EvoParseq
v0.9.5**, adding an Android TV client and shared controller/input infrastructure.
The changes below cover implemented additions since that stable release;
Windows video features already shipped in v0.9.5 are not repeated.

**This is a development preview. EvoParseq v0.9.5 remains the stable Windows release.**

## What's new and improved

- **Android TV interface:** add a Google TV / Android TV-style computer picker,
  remote-friendly sign-in and two-factor-code entry, grouped Network/Video/Device
  settings and clear focus indicators. Keep device capabilities inside Settings,
  prevent focused cards/options from overlapping adjacent content and retain
  readable text and a circular launcher icon.
- **Shared Rust core:** reuse authentication, discovery, signaling, transport,
  session preferences, validation and input/controller state across Windows and
  Android. Kotlin/Compose handles TV presentation and Android APIs; MediaCodec,
  AAudio, input/lifecycle and Keystore remain explicit platform adapters. Windows
  builds exclude Android adapter dependencies and retain the existing GPU path.
- **Android video and audio:** decode H.264 and HEVC 8/10-bit 4:2:0 through
  hardware MediaCodec directly to a Surface, with shared codec initialization,
  bounded queues and keyframe recovery. Decoded pixels are not copied through
  JNI or read back to CPU memory. Stereo 48 kHz audio uses the shared Rust Opus
  decoder and AAudio pull output, with bounded audio storage.
- **BUD and WebRTC on TV:** connect to the unmodified official host directly or
  through configured STUN/TURN. Reuse explicit transport selection, Cloudflare
  and custom relay credentials, Force TURN and the existing BUD UDP/TLS/TCP
  carrier order. Preserve ordinary BUD input validation while adding shared
  application-control routing. No silent transport switch or host patch is added.
- **Secure remembered TV sign-in:** retain only the session token, encrypted
  using AES-256-GCM with a non-exportable Android Keystore key, in private
  no-backup storage. Passwords and 2FA codes are not saved. Sign out clears
  local credentials and requests server-side revocation; expired sessions
  require signing in again.
- **Local relay setup from a phone or computer:** open an opt-in, expiring HTTPS
  page on the TV's private LAN to enter Cloudflare/custom TURN or STUN settings.
  Authenticate the displayed certificate fingerprint, enter the pairing code
  and explicitly approve the pending settings on the TV. Force TURN is included.
  Preserve origin/Host checks, bounded submissions and Keystore-protected relay
  secrets. No external provisioning website or public setup endpoint is used.
- **Shared controller forwarding:** one Rust state/protocol pipeline supports
  up to four standard controllers on BUD and WebRTC. Windows uses native XInput;
  Android translates standard gamepad/joystick events. Forward buttons, D-pad,
  sticks and analog triggers, preserve host controller presence across local
  menus while releasing held state, and handle removal/disconnect explicitly.
  Device-supported vibration uses the platform adapter. Wired Windows and
  GameSir Android controller forwarding have been live-tested.
- **TV remote mouse and physical input:** optionally use D-pad to move, short
  OK for left click, two short presses for double-click, hold OK for right click,
  and OK plus direction for dragging on compatible remotes. Keep separate
  gamepad controls intact. USB/Bluetooth keyboards and mice use normal Parsec
  input with optional mouse capture; Ctrl+Shift+M releases input and opens local
  controls. Share gesture/encoding/handoff rules in Rust. Correct cursor size,
  hotspot/edge mapping and focus ownership without rebuilding the video Surface
  when the cursor or controls change.
- **Compact, dismissible TV controls:** show actual codec/depth, Surface
  submission FPS, selected-pair RTT, incoming bitrate and direct/relay routing
  when available. Auto-hide after five idle seconds, retain manual Hide and
  Disconnect, and restore controls through Back/Menu. Settings support fixed or
  dynamic FPS. Hint Surface cadence from the host-confirmed fixed rate, clearing
  the hint for dynamic/unknown cadence; do not force HDMI modes, add a frame
  scheduler or introduce a decoded-frame copy. Show display Hz and catch-up skips.
- **Recovery and maintenance:** retain meaningful stop/error reasons after
  returning to the computer picker, handle Surface/lifecycle and input-device
  transitions, and keep empty WebRTC messages distinct from channel EOF.
  Remove retired diagnostic overlays/probes while retaining bounded recovery and
  useful diagnostics. Add common Rust, ARMv7/ARM64, Clippy and Android lint/build
  checks alongside the existing Windows regressions.

## Compatibility and validation

**Android TV is an experimental preview**, not a Play Store or production-signed
release. The APK includes ARMv7 and ARM64 libraries and requires Android 10
(API 29) or newer. The first live-tested device is **Xiaomi TV Box S (3rd Gen)
EU on Android 14**, using a 32-bit ARM userspace. Other devices/codecs/controllers
need their own live validation.

H.264, HEVC 8-bit and NVIDIA-host HEVC Main10 are live-confirmed on this box.
The tested AMD host returned 8-bit. BUD at 50 Mbit/s was stable in the tested
setup; sustained WebRTC 50 Mbit/s still caused delay/video-channel closure on
Ethernet and Wi-Fi, while about 30 Mbit/s was stable. This is not a proven
universal hardware bitrate ceiling. Surface timing felt smoother in live use;
no quantified latency improvement or physical scanout result is claimed.

Controller forwarding is live-confirmed in the tested Windows wired and
Android GameSir configurations. DirectInput-only Windows pads, advanced Sony
features and all-device rumble compatibility are not provided. Home/Guide stays
OS-owned. The official host needs its normal controller driver and permissions.
Physical Android input/capture and device/focus edge cases need broader testing;
Android does not implement the Windows BUD USB HID mode. Some remotes cannot
report simultaneous OK and direction presses required for dragging.

MediaCodec-to-Surface output is not a claim of verified driver/compositor
zero copy. Android does not advertise HDR, 4:4:4 or AI upscaling; 10-bit is SDR.
Existing Windows experimental video/HID/QoS limits remain as documented for
v0.9.5. This release does not change the stable main branch or host configuration.
EvoParseq remains independent of Parsec/Unity and lowlat.

## Updating

**Windows:** close the client and extract **EvoParseq-v0.9.6-dev.1-win64.zip**.
Keep `settings.json` under the same Windows account. The legacy-named
**ParsecWebTurn-v0.9.6-dev.1-win64.zip** contains the identical executable for
existing shortcuts. ZIPs exclude settings.json and the optional NVIDIA runtime.
Protected credentials cannot be transferred between machines/accounts.

**Android TV:** sideload **EvoParseq-TV-v0.9.6-dev.1.apk**. No root or extra helper
is required. This is a debug-signed CI preview; its signing certificate can
change between builds. If Android rejects an update signature, uninstall the
older preview first. Uninstalling removes local settings and remembered sign-in.

For phone/computer relay setup, verify the local certificate's SHA-256
fingerprint against the TV before entering secrets, then approve settings on
TV. See the Android TV README for setup and controller instructions.

This is a **prerelease**, installed manually; it does not replace v0.9.5 as the
stable updater's latest release. The repository rename can require a one-time
manual upgrade from older ParsecWebTurn/EvoParseq previews. See MIGRATION.md.

## v0.9.5 — Native Rust core, GPU video and input improvements (2026-11-02)

This stable Windows release replaces the ParsecWebTurn client runtime with
**EvoParseq**, an independent native Rust client for existing, unmodified Parsec
hosts. The changes below cover the implemented changes since the latest stable
release, **ParsecWebTurn v0.9.4**, following live NVIDIA, AMD and Intel testing.
The repository is now **vampywiz17/EvoParseq**.

## What's new and improved

- **Native Rust core:** account sign-in, optional two-factor authentication,
  computer discovery, authenticated signaling and owner-session control run
  directly in Rust. No proprietary Parsec WebAssembly core, browser or WebView
  is bundled or executed. Remembered sessions use Windows CurrentUser encryption;
  passwords and two-factor codes are not saved. Computer discovery refreshes
  quietly in the background. Friend/invitation authorization is not implemented.
- **Native WebRTC and BUD:** retain direct connections, custom TURN over
  UDP/TLS/TCP, Cloudflare credentials, Force TURN and STUN-only routing while
  replacing WASM-dependent session orchestration. Transport selection is explicit;
  a BUD failure never silently switches to WebRTC. Shared account/session policy
  is separated from platform adapters without adding a media-processing layer.
- **EvoParseq interface and session controls:** native About/update pages,
  consistent dark windows, a draggable icon and a compact performance summary.
  Use dropdowns for resolution, bandwidth, FPS, constant/adaptive FPS, orientation,
  fullscreen and immersive input. Apply host requests with asynchronous
  confirmation; keep startup black and correct overlay/stats window ordering.
  Hidden controls can be restored by shortcut.
- **Host display selection:** switch the current stream between host-listed
  physical or virtual displays and release stale resolution overrides when
  changing output. HID mouse mode restricts the session to the primary display,
  disables display switching and pauses forwarding until that selection is
  reported. Some host virtual displays can still delay a switch.
- **Unified HEVC decoder policy:** normal HEVC uses direct D3D11 on NVIDIA,
  AMF on AMD and VPL on Intel. Supported NVIDIA/Intel paths cover 8/10-bit
  4:2:0 and 4:4:4; AMD offers 8/10-bit 4:2:0. H.264 remains Media Foundation
  8-bit 4:2:0. Decoder errors retain bounded, logged MF recovery. If a 4:4:4
  compatibility failure requires it, request 4:2:0 and use MF after the host
  changes format, preserving bit depth and saved preferences.
- **Intel HEVC validation:** 8/10-bit 4:4:4 normal rendering is live-confirmed
  on the tested UHD 770. A narrowly scoped, logged exception handles this
  adapter's false-negative Y410-to-RGB10 conversion query while preserving
  input/output format checks and runtime failure handling. No extra live-frame
  copy, CPU readback or per-frame test is introduced.
- **AMD HEVC compatibility:** initialize cropped AMF streams using visible
  dimensions on older runtimes, select the Main10 component where required and
  check coded dimensions once against the selected GPU before initialization.
  AMF Main10 is now the AMD default and live-confirmed on Radeon 780M, retaining
  original P010 output and the existing GPU conversion pass.
- **10-bit output and original-surface presentation:** normal rendering can
  produce RGB10 SDR through the existing conversion pass. Zero copy now supports
  original 4:2:0 P010 Main10 surfaces as well as 8-bit NV12 when compatible
  displayable surfaces are available. Main10 Zero copy is live-confirmed on
  tested AMD and NVIDIA systems. Unsupported original-surface setup/runtime
  falls back to normal rendering; 4:4:4 uses normal GPU rendering.
- **Format changes and recovery:** rebuild HEVC GPU sessions after actual
  format changes, preserve parameter-set dependency order and retire stale codec
  state across host encoder rebuilds. Supervise decoder/device setup, processing,
  presentation and teardown without freeing driver-owned resources prematurely
  or accumulating replacement workers. Unrecoverable native timeouts require
  restarting the app. Host Main10 encoder refusal asks users to disable 10-bit
  rather than silently reopening an 8-bit session.
- **Presentation and display cadence:** extend reference-safe burst catch-up
  to WebRTC, using bounded recent queue history to skip obsolete presentation
  work while decoding dependencies in order. Add optional DXGI VRR with bounded
  fallback. Session advice distinguishes configured client Hz, requested and
  host-reported FPS, and measured decoded FPS; refresh suggestions never change
  the client's display mode automatically.
- **Input reliability and immersive shortcuts:** preserve simultaneous mouse
  holds, independent releases and camera transitions. Coalesce adjacent absolute
  moves and accumulate relative motion without crossing button/wheel/focus
  boundaries; reserve capacity for other input and fail safely on overflow.
  Forward supported immersive keyboard chords through one ordered, focus-scoped
  path in normal and HID modes, keeping exact client shortcuts local. Limit
  startup state queries to required keys and preserve focus-loss releases.
- **Clipboard, cursor and audio recovery:** retry failed clipboard writes with
  bounded retries of only the newest text, respecting permissions and local
  changes. Match remote cursor canvases to the client's Windows cursor size.
  Reopen audio after transient endpoint errors or default-output changes, retire
  stale queued audio and preserve decoder state and buffer preferences.
- **Clearer settings and diagnostics:** group related choices into selectors
  and state explicitly that video settings are host requests, not guarantees.
  Organize Connection stats into Overview, Network, Video, Audio and Timings
  tabs. Add recent two-second timing averages, expire stale values and separate
  decoded FPS from presentation-submission FPS. BUD audio timing no longer
  overwrites video encode timing. Stats report the actual stream/output format.
- **Optional GPU AI upscaling:** one experimental GPU-dependent selector offers
  NVIDIA RTX VSR quality levels or Intel VPL AI. Upscaling is off by default,
  stays GPU-resident and retains normal rendering on unsupported configurations.
  RTX 3060 playback is live-confirmed; Intel AI remains hardware-unverified.
  With explicit consent, the pinned official NVIDIA runtime is downloaded,
  archive/DLL SHA-256 checked and cached under
  `%LOCALAPPDATA%/EvoParseq/vsr/1.1.0`, outside the EXE and without admin rights.
  Intel uses its installed driver runtime and downloads no model or DLL.
- **Optional TURN socket QoS:** experimental Windows AudioVideo marking for
  outgoing BUD/WebRTC TCP/TLS TURN sockets is off by default. Rejection is
  nonfatal. It does not prioritize incoming traffic or alter relay-server policy.
- **Maintenance and security:** update the existing WebRTC DTLS NIST-curve
  dependencies without replacing the WebRTC engine. Refresh transport/TLS
  dependencies, remove unused browser-certificate and binary state-export chains,
  and retain protocol/certificate checks. Add PR Clippy, dependency-policy and
  workflow checks. Remove retired probes and legacy adapters; retain licenses
  and third-party notices. README includes animated relay/rendering guides.

## Compatibility and validation

**Windows 11 x64 and Direct3D feature level 11.0 or newer are required**, with
compatible video drivers. H.264, stereo Opus and Automatic presentation remain
the conservative defaults. Host settings are requests: check **Connection stats**
for the codec, depth, chroma and output actually received.

**Vendor HEVC, Zero copy, 10-bit HEVC, HEVC 4:4:4, display timing/VRR, AI
upscaling, USB HID, QoS and multichannel audio remain experimental options.**
Successful tests do not guarantee every GPU/driver combination. Intel AI and
live multichannel playback remain unverified; no universal latency gain is claimed.

10-bit output is **SDR, not HDR**. Normal GPU conversion is not Zero copy.
Original-surface presentation does not prove driver/compositor copies are absent
or direct scanout is active. AI upscaling adds GPU work/resources and an output
copy, may increase latency and cannot be combined with Zero copy. VRR requires
compatible display/driver settings; application support does not prove panel
activation. Existing host permissions, HID driver/primary-display requirements
and raw-audio permissions remain applicable.

**Android TV previews and controller forwarding on that development line are
not included in this stable Windows release.** Development continues on dev.
EvoParseq is independent of Parsec/Unity and lowlat; applicable service terms
and third-party licenses still apply.

## Requirements and upgrading

Close the client and extract **EvoParseq-v0.9.5-win64.zip**, keeping your
`settings.json` under the same Windows account. Native remembered sessions and
compatible settings are retained; protected credentials are not portable between
machines/accounts. ParsecWebTurn web-core sign-in is not imported: sign in again,
including 2FA if enabled.

**ParsecWebTurn-v0.9.5-win64.zip** contains the identical executable under the
legacy name for existing shortcuts. Both ZIPs exclude settings.json and the
optional NVIDIA runtime. Run only one client at a time.

After the repository rename, older updaters may reject the new asset URL because
they validate against the old repository name. Use this release page for the
one-time manual upgrade. EvoParseq v0.9.5 checks the new repository. No WASM-core
version check or download remains. See README and MIGRATION.md.

## v0.9.5-dev.2 — AI upscaling, diagnostics and input fixes (2026-10-28)

Implemented changes since EvoParseq v0.9.5-dev.1.

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
- **Experimental DXGI VRR:** add optional variable-refresh presentation to the
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

## v0.9.5-dev.1 — Native Rust core and session controls (2026-10-27)

Implemented changes since stable ParsecWebTurn v0.9.4. This prerelease is the
first EvoParseq release from `dev`; the previous stable remains available.

- **Native core:** implement account sign-in, optional two-factor authentication,
  computer discovery, authenticated signaling and owner-session control directly
  in Rust. Remembered sessions use Windows CurrentUser encryption; passwords and
  two-factor codes are not saved. The computer picker refreshes in the background.
- **Native WebRTC and BUD sessions:** retain the existing media and transport
  backends while replacing their WASM-dependent orchestration. Use explicit
  transport selection, direct connections, custom TURN UDP/TLS/TCP, Force TURN,
  STUN-only and Cloudflare credentials without a silent transport switch.
- **New EvoParseq interface:** add application branding, native About/update
  windows, a draggable overlay icon, performance summary and session controls for
  resolution, bandwidth, frame rate, constant FPS, orientation, immersive input
  and fullscreen. Selection fields use dropdowns, host-reported choices where
  available and compatible presets otherwise. Restore hidden controls by shortcut
  and keep local menus available while forwarding remote input.
  Keep the stream canvas black during startup, fix overlay/stats window
  ordering and confirm asynchronous host-settings updates before accepting
  another apply, avoiding stale values and repeated clicks.
- **Host display selection:** switch the current stream between listed physical
  or virtual outputs, requesting the selected display's host resolution. HID
  mouse mode restricts selection to the primary output and pauses forwarding
  until that output is reported, preserving the absolute-positioning contract.
- **Experimental HEVC 4:4:4:** add compatible NVIDIA/Intel 8-bit AYUV and 10-bit
  Y410 decode/output paths with a single GPU RGB conversion and no decoded-frame
  CPU readback. NVIDIA RTX 3060 8/10-bit playback is live-validated; Intel remains
  capability-gated and unverified. AMD selection is disabled. H.264 stays 4:2:0,
  and original-NV12 zero-copy is not combined with 4:4:4.
- **AMD HEVC improvements:** initialize cropped AMF streams with their visible
  dimensions for older runtimes. Add optional Main10 component selection with
  original P010 GPU output and the existing RGB10 conversion; Radeon 780M
  playback is confirmed. Media Foundation remains the default Main10 route.
- **Video format changes:** rebuild HEVC GPU sessions when the host changes
  the encoded format, preserve parameter-set dependency order, and retire old
  codec state during encoder rebuilds without adding frame copies.
- **Clear 10-bit failure handling:** preserve the requested mode. An initial host
  encoder failure prompts the user to disable 10-bit and reconnect instead of
  silently reopening an 8-bit session. Stats retain actual stream/output precision.
- **Presentation catch-up:** extend the reference-safe BUD approach to WebRTC,
  decoding dependencies in order while skipping obsolete display work after a
  burst. Bounded recent queue history adapts WebRTC catch-up to sustained late
  input and resets after recovery. No extra frame copy, rendering thread or
  pacing buffer is introduced.
- **Automatic audio-output recovery:** reopen shared WASAPI after transient
  device errors and Windows default-output changes, without reconnecting video.
  Retire stale queued and in-flight audio, preserve Opus state and existing
  buffer preferences, and expose recovery transitions in logs/stats.
- **Optional TCP/TLS QoS:** add an off-by-default experimental Windows AudioVideo
  marking option for outgoing BUD/WebRTC TURN sockets. Refusal is nonfatal;
  existing TLS verification and transport IO remain intact. It does not set
  Cloudflare/server priorities or prioritize incoming relay traffic.
- **Cursor, settings and diagnostics:** size remote cursor images to the client's
  Windows cursor canvas while preserving shape/hotspot proportions. Group routing,
  video-format, HID and audio choices into dropdowns; retain saved preferences.
  Keep BUD video encode timing separate from audio timing reports and reuse
  existing telemetry for the session overlay and diagnostics.
- **Maintenance and packaging:** separate shared sign-in, settings, session,
  diagnostics and input rules from Win32 adapters while retaining the GPU/media
  path. Remove the old runtime, adapters, diagnostic launchers and obsolete
  prototype dependencies/documentation. Refresh Tokio to 1.53.3,
  rustls-webpki to 0.103.16 and webpki-roots to 1.0.10. Ship portable EvoParseq
  and legacy-named transition packages with retained third-party notices.


See [release notes](RELEASE_NOTES.md) for compatibility and migration details.

## ParsecWebTurn release history

User-facing changes are listed below. Native versions marked **dev** are development
builds, not published stable releases.

## Unreleased

- Extend experimental original-surface Zero copy to HEVC Main10 4:2:0 P010. Check optional P010 displayable support at setup; decode directly into displayable P010 DPB textures and retain existing presentation leases. No application pixel copy, RGB target, processor/shader pass or new per-frame format probe is added. Unsupported setup/runtime falls back to normal Media Foundation rendering without a silent 8-bit downgrade.
- Report original P010 presentation and actual decoded 10-bit depth without claiming final compositor/scanout output depth. HEVC 10-bit 4:2:0 Zero copy is now live-confirmed on tested AMD and NVIDIA systems; Intel HEVC paths under the current decoder policy still require live testing.


- Standardize normal HEVC decoding: NVIDIA direct D3D11 including P010, AMD AMF 8/10-bit 4:2:0 by default, and Intel VPL NV12/P010/AYUV/Y410. Remove the AMF Main10 opt-in selector; AMD 4:4:4 remains disabled. Keep original-surface Zero copy limited to 4:2:0 NV12/P010.
- Log decoder selection and MF fallback attempts. A 4:4:4 decoder compatibility failure requests 4:2:0 on BUD/WebRTC, preserves the requested bit depth and saved preferences, and uses MF after the host changes format. Recovery remains bounded; unsupported host responses fail explicitly.
- Explain in Video settings that format choices are requests and compatibility fallback may change the result; Connection stats shows the actual stream.


## 0.9.4 - BUD relays, GPU video and HID input (2026-10-25)

This stable Windows release brings together the changes from v0.9.4-dev.1 through
v0.9.4-dev.7, following the maintainer's Intel, AMD and NVIDIA video testing.
The changes below cover everything delivered since the previous stable release,
**v0.9.3**. This remains the ParsecWebTurn client built around the original,
unmodified Parsec WASM core.

- **BUD transport:** connect to unmodified Parsec hosts directly over UDP or through configured TURN relays over UDP, TLS or TCP. Use the same Network settings as WebRTC, including Force TURN; startup retries prefer UDP, then TLS, then TCP, with port 80 first among TCP URLs. An established connection is not migrated and BUD never silently switches to WebRTC.
- **Responsive relay IO:** native event-driven BUD TCP/TLS sockets retain partial writes, bounded reusable buffers, TLS backpressure and write deadlines. TLS verifies the server certificate and hostname. WebRTC TURN/TCP reuses receive storage, and TCP/TLS ChannelData is submitted with its padding in one framed write. Newly queued BUD HID reports and control replies drain before the next network wait, without a batching delay or changes to protocol retransmission timers.
- **Vendor HEVC decoding (experimental):** normal 8-bit HEVC uses Direct D3D11 on NVIDIA, AMF on AMD and VPL on Intel, with the existing Media Foundation recovery/fallback. H.264 continues to use Media Foundation on every vendor. Burst handling and surface ownership fixes improve HEVC startup and avoid unnecessary Intel decoder fallback.
- **Zero copy presentation (experimental):** supported NVIDIA, AMD and Intel GPUs can present original 8-bit NV12 decoder surfaces through Windows Presentation API, preserving aspect ratio and falling back to normal GPU rendering when needed. This removes the application's conversion/output pass without decoded-frame CPU readback; internal Windows/driver copies and direct scanout are not guaranteed. Windows 11 and compatible drivers are required.
- **Correct color range:** translate full/limited-range metadata correctly between Media Foundation and D3D11, including vendor output. This fixes grey-looking whites and lifted blacks. Startup, later encoder requests and keyframe requests consistently retain the selected range and HEVC flags.
- **10-bit HEVC and RGB SDR output (experimental):** request Main 10 over WebRTC or BUD when supported, including the BUD startup fix for negotiated 10-bit video. Actual P010 decoder output can feed a 10-bit RGB swap chain through the existing GPU conversion pass, with compatible 8-bit RGB output fallback. Stats distinguish decoded depth from application output. Host/driver support determines the received stream; this is SDR, not HDR, and original-surface Zero copy stays 8-bit only.
- **Audio controls:** add a dedicated Audio tab with optional minimum/maximum playback buffering, supported low-period shared WASAPI output with fallback, bounded fresh PCM queues and BUD-only raw PCM permission. Keep automatic buffering and Opus as defaults, align the menu's initial custom limits with the pinned core, and handle PCM/Opus transitions safely. Raw PCM also requires host support and can fall back to Opus; WebRTC remains stereo Opus.
- **Multichannel audio (experimental, BUD):** optionally request the Windows output's compatible 5.1/7.1 channel layout, decode Opus/PCM with per-packet format information and report the actual layout. Stereo remains the default and fallback. This feature has synthetic tests and silent WASAPI validation only; live surround playback and channel ordering are not confirmed.
- **USB HID mouse and keyboard (experimental, BUD):** use the stock host's USB/Yubikey passthrough for absolute desktop positioning, native relative camera control, five mouse buttons, dragging, scrolling and physical keyboard input. Mouse and keyboard share one device lifecycle and bounded queues; selected HID devices replace ordinary Parsec forwarding independently. Local menus remain accessible and focus loss/disconnect releases held input. No helper executable or host patch is needed; one physical or virtual display and matching keyboard layouts are supported.
- **Dragging and cursor transitions:** restore the host-reported endpoint when leaving relative/hidden capture so the next HID click does not return to the old drag anchor. Keep the cursor hidden until position restoration and replace its image before retiring the old handle. Prompt BUD capture-state wakeups reduce transition flicker while preserving camera input, event order, focus handling and local controls.
- **Two-way text clipboard:** forward native clipboard changes, focus restoration and paste-key requests through the original core while respecting core/host permissions. Preserve UTF-8 boundaries, avoid echoes and distinguish unavailable reads from copied empty text. Client-to-host pasting has been verified live; file/image transfer is not added.
- **Display timing and burst catch-up:** add experimental Automatic, Low latency and VSync choices. Automatic preserves previous behavior; explicit DXGI modes request a one-presentation queue. Interruptible display/input readiness waits fix the reported Low latency stutter while preferring fresh decoded input over a pending presentation. BUD can skip obsolete display work without dropping reference decoding. No extra decoded-frame copy, RGB target or rendering thread is introduced; original-surface timing stays Windows-managed.
- **More useful diagnostics:** Connection stats adds native BUD route/RTT/traffic/delivery counters, actual audio codec and queue/buffer details, decoded/RGB precision, automatic host video/audio timing and bounded local video-stage measurements. Correct encode/frame-interval labels and add Present submission interval. The embedded overlay receives native Decode and Network values. Timings describe their reported host durations or CPU-observed stages, not GPU completion, scanout or end-to-end latency; unavailable values remain unknown.
- **Optional event logging:** Network settings can create a bounded asynchronous UTF-8 JSON Lines log beside settings.json, with UTC connection/route transitions, transport/decoder/HID failures and audio/profile state. Rotation retains one previous file. Logging is off by default and excludes credentials, media, clipboard content and continuous mouse/key activity; temporary input trace capture has been removed.
- **Settings, recovery and maintenance:** separate Network, Video, Audio and Input with consistent tab spacing, shared saving and preserved unsaved edits. Disable Zero copy and 10-bit controls while HEVC is off, fix wrapped settings/stats text and guide unsupported host launch choices. Reuse bounded media/input storage, shorten shared locks and release retired video reuse pools. Add concise bug/feature/question issue forms, consolidate active native tests, remove retired experiments, shorten the README, clarify the unchanged-core provenance and credit lowlat's reused open-source components.

**Vendor HEVC decoding, Zero copy, 10-bit HEVC, display timing, multichannel
audio and USB HID input remain experimental options in this stable release.**
WebRTC, H.264, stereo Opus and Automatic display timing remain the defaults.
BUD, custom audio buffering, raw PCM permission and host timing no longer carry
an experimental label. For the most conservative setup, keep the defaults and
leave experimental options disabled.

Successful testing applies to the supported configurations tested, not every
GPU/driver combination. Confirmed Main 10 host encoding is documented for the
tested NVIDIA setup; other encoders may send 8-bit. Stats show the stream and
output actually received. Multichannel audio remains supported by synthetic
tests only.

## 0.9.4-dev.7 - Input and low-latency presentation fixes (2026-10-22)

Changes since v0.9.4-dev.6. Existing defaults and experimental feature labels
are unchanged; this is a Windows x64 development release.

- **HID dragging:** restore the local pointer to the host-reported endpoint on leaving relative/hidden capture, preventing a stale anchor from overwriting the next absolute HID position. Preserve native relative camera input, focus handling and local controls.
- **Cursor transitions:** retain cursor hiding through position restoration and select a replacement image before freeing the old handle. BUD host capture-state changes wake the existing input callback promptly, keeping FIFO ordering and normal callback cadence. Live testing reported substantially reduced flicker; delayed host cursor responses can still be visible.
- **BUD output scheduling:** drain newly queued HID reports and control replies through the existing UDP/TCP/TLS carrier before waiting again. Preserve bounded backpressure, ordering, ACK/retransmission timers and WebRTC behavior. No fixed end-to-end latency gain is claimed.
- **Low latency presentation:** interruptible DXGI/input readiness waits replace regular idle readiness polling where available. Feed fresh input before retrying a pending picture, preserve the one-presentation limit and BUD burst catch-up, and add no decoded-frame copy, RGB target or worker thread. Reported stutter was resolved in live testing on the tested 60 FPS / 59.95 Hz DXGI setup; this is not a universal smoothness or measured scanout-latency claim. Automatic, VSync and original-surface Zero copy retain their presentation policies.
- **Connection stats:** add latest/average/peak Present submission interval. This is CPU-observed DXGI submission cadence, including idle periods, not GPU or physical display timing.
- **Settings and documentation:** make the four tabs equal-width with consistent spacing and a clearer gap before Back/Save. Shorten the README and update current feature/validation descriptions.
- **Maintenance:** remove temporary HID/cursor trace capture and build switches; retain focused regression coverage. No new runtime dependency or continuous input-event logging.

Extensive Intel/AMD/NVIDIA validation now covers normal rendering, Zero copy
and H.265/HEVC, including 10-bit, across supported combinations. The next stable
release is targeted for the second half of next week (October 29-November 1,
2026), after those combinations prove stable and blocking issues are resolved.

## 0.9.4-dev.6 - Playback diagnostics and display timing (2026-10-21)

Changes since v0.9.4-dev.5.

**Vendor-specific HEVC decoding, Zero copy, 10-bit HEVC, display timing,
multichannel audio and USB HID input remain experimental.** BUD, custom audio
buffering, raw PCM permission and host timing no longer carry the experimental
label. Defaults remain WebRTC, H.264 and automatic playback. For stability-first
use, leave experimental options disabled or use the stable v0.9.3 release.

- **Embedded overlay metrics:** supply measured native decoder input-to-output time and fresh BUD/WebRTC RTT through the original seven-output web import. Keep unavailable values unknown and preserve the WASM binary. The web import has no bitrate output; native Connection stats remains the source for traffic rates.
- **Feature status:** remove the experimental label from BUD, custom audio buffering, raw PCM permission and host timing. Keep existing defaults and accept the former settings keys when loading saved profiles. Other experimental options keep their labels; historical release notes describe their original status.
- **Connection stats layout:** measure both text columns when sizing rows so wrapped labels and values do not overlap; shorten the display catch-up counter label.
- **Local video timing:** bounded CPU measurements of native compressed-queue wait, decoder setup/service and correlated input-to-output time, surface handoff, presentation preparation and Present calls. No GPU execution/scanout claim, pixel readback or per-frame logging.
- **Display timing (experimental):** Automatic preserves existing behavior; Low latency and VSync request a one-presentation DXGI queue with nonblocking readiness checks and compatible fallback. Original-surface Zero copy keeps Windows-managed timing. Hardware/live latency gains remain unverified.
- **BUD burst handling:** retain reference decoding while omitting obsolete display updates when later compressed messages are queued. Deterministic tests compare identical decoded references and reduced display work; WebRTC delivery is unchanged.
- **Memory diagnostics:** distinguish resident/private committed process memory in stats and record a few startup milestones through existing optional event logging. The reported intermittent high-memory startup remains under investigation; no forced working-set trimming. Retired video pipelines release their compressed-buffer reuse pool while preserving final statistics.

- **Two-way text clipboard:** forward native clipboard changes, stream focus restoration and paste-key requests to the original core. Preserve clipboard preference/host permissions, avoid echoing incoming host copies, and keep unavailable reads distinct from copied empty text. Client-to-host pasting has been confirmed in live use.
- **Host timing labels:** correct the reversed encode-duration and video-frame-interval labels. Values remain in milliseconds without rescaling; audio interval anomalies remain under investigation.
- **Detailed host timing:** automatically request video/audio timing while Connection stats is visible or event logging is enabled. Show reported processing durations, video capture/encode segments and frame/block intervals with latest/average/peak values, report counts and freshness. Bounded collectors bypass application control queues in BUD and WebRTC; malformed or unavailable diagnostics do not fail the stream. No separate switch or per-frame file logging. Live BUD report reception observed; WebRTC validation and audio interval anomalies remain under investigation.
- **Multichannel audio (experimental, BUD; synthetic tests only):** optional automatic 5.1/7.1 requests based on the Windows output layout, per-packet Opus/PCM format handling, native multichannel playback, and actual channel information in Connection stats. Stereo remains the default and the host may fall back to stereo. Validation covers synthetic Opus/PCM tests and silent WASAPI output only; live host-to-client operation, channel ordering and audible surround playback remain unverified. WebRTC retains stereo.
- **Relative mouse transition:** hide the local cursor before Raw Input registration and confinement, while retaining cursor restoration if acquisition fails. This narrows the visible transition; the reported intermittent flash still needs live verification.

- **Maintenance and reporting:** add bug/feature/question issue forms, move the active TURN integration fixture under native tests, and remove the obsolete separate BUD test-file logger and launcher instructions. Connection failure stages remain in the normal event log.

## 0.9.4-dev.5 - Experimental USB HID input (2026-10-20)

Changes since v0.9.4-dev.4.

**USB HID input, BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC
and the Audio options remain experimental.** For stability-first use, select
WebRTC with H.264, leave experimental options disabled, or use the published
stable v0.9.3 release. HID input requires BUD and USB/Yubikey passthrough on
the stock host; compatibility depends on the host and application.

- **Experimental USB HID mouse:** use the stock host's USB/Yubikey passthrough with BUD for absolute desktop positioning, native relative camera movement, five buttons, dragging and vertical/horizontal scrolling. Relative capture follows the host cursor state and hides the local pointer. One physical or virtual display is supported.
- **Experimental USB HID keyboard:** forward physical key presses, releases and modifiers through the same host USB path. Matching active layouts, including Hungarian on both computers, are supported; the host layout is not changed automatically. Simultaneous keys use a bitmap without a six-key array limit.
- **Simultaneous HID input:** mouse and keyboard share one USB device lifecycle and bounded transport queues, with separate reports and fair scheduling. Buttons keep the same owner across desktop/camera mode changes. Ordinary Parsec forwarding is disabled independently for each selected HID input, preventing duplicate input.
- **New Input settings tab:** select HID mouse and keyboard independently, save the preferences and reconnect. Both options require BUD and are disabled by default. Local menus remain usable; focus loss and disconnect release held input and capture. Only the usual EXE and settings.json are needed, without a host modification or additional launcher.
- **HID operational logging:** the existing optional client-events.jsonl records activation, state changes and setup/transfer/queue failures, without continuous mouse movement, coordinates, clicks, key history, typed text or USB payloads.

## 0.9.4-dev.4 - Relay IO improvements and 10-bit SDR output (2026-10-17)

Changes since v0.9.4-dev.3.

**BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC and the Audio
options remain experimental.** For stability-first use, select WebRTC with H.264,
leave experimental options disabled, or use the published stable v0.9.3 release.

- **Native BUD TCP/TLS IO:** relay sockets use the existing platform wake and native socket wait, with overlapped IOCP on Windows, bounded reusable storage and cancellation. Queued TURN frames drain from reused staging storage without a batching delay; received frames are parsed in place. Direct BUD remains UDP, and TLS retains certificate and hostname verification through rustls.
- **BUD TLS backpressure:** retain pending frames until both TURN framing and encrypted TLS output have drained to the socket. This prevents new protocol output from building up behind earlier TLS records and preserves write deadlines during slow sends.
- **More efficient WebRTC TURN streams:** reuse a bounded receive buffer for plain TURN/TCP and write ChannelData with its padding as one framed write on TCP/TLS. TLS retains its existing receive path. No batching delay is added; UDP routing, ICE selection and SCTP behavior are unchanged.
- **Experimental 10-bit RGB SDR output:** actual Main 10/P010 decoder output can now use a 10-bit RGB swap chain when the GPU and driver support conversion and presentation. The existing GPU conversion writes directly into the output buffer, without an additional decoded-frame copy or conversion pass. Unsupported output falls back to 8-bit RGB. This is SDR, not HDR, and does not establish physical display or scanout depth.
- **Clearer video precision status:** Connection stats and optional event logging distinguish decoded color depth from application RGB output depth and report output fallback. Live Main 10 with 10-bit RGB output is currently verified with NVIDIA host encoding and AMD Radeon 780M client decoding; the tested AMD host path sent 8-bit HEVC. Original-surface Zero copy remains 8-bit only, and 4:4:4 remains unsupported.
- **Leaner native components and acknowledgements:** remove unused direct H.264 decoder code and its unused GPU protocol-core dependency; H.264 continues to use Media Foundation on all GPU vendors. The README now explicitly credits lowlat inspiration and reused components while clarifying that ParsecWebTurn is an independent project.

## 0.9.4-dev.3 - BUD TCP/TLS relays and audio controls (2026-10-16)

Changes since v0.9.4-dev.2.

**BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC and the new Audio
options are experimental.** For stability-first use, select WebRTC with H.264,
leave experimental options disabled, or use the published stable v0.9.3 release.

- **Experimental BUD over TURN TCP/TLS:** use the existing TURN URLs for BUD as well as WebRTC, with UDP preferred, then TLS and plain TCP (port 80 preferred among TCP URLs). Failed startup attempts can try the next configured relay; direct BUD remains UDP. TLS checks the server identity. BUD TURN/TLS playback has been confirmed with a stock host using AMD AMF HEVC and Direct D3D11 HEVC.
- **Reliable TCP/TLS startup and responsive IO:** handle pending nonblocking TCP connections correctly, disable Nagle before sending TLS/application data and retain partial writes under backpressure. The event-driven carrier uses bounded reusable buffers and immediate input wakeups without a batching delay. Event logging identifies failed transport stages and IO error codes without recording credentials or packet contents.

- **Experimental Audio settings:** add a separate Audio tab with optional minimum/maximum playback buffering, supported low-period WASAPI initialization with fallback, bounded fresh PCM playback, and BUD-only raw PCM permission. Automatic playback and Opus remain the defaults. Raw requires host support and can still fall back to Opus; WebRTC audio remains Opus.
- **Audio visibility:** show actual audio codec, local output buffer and limits, device capacity, decoded PCM backlog and underruns in Connection stats. Operational logging includes audio configuration and playback-error counters without media payloads.

- **HEVC-dependent video options:** disable the Zero copy checkbox while Prefer H.265 / HEVC is off, matching the 10-bit HEVC option. Turning HEVC back on restores access to the saved preference.
- **BUD Main 10 startup fix:** remove an obsolete 8-bit-only parser check that disconnected BUD when Main 10 video arrived. Advertised Main 10 is passed to the existing MF/P010 GPU path; host/driver compatibility still needs testing. 4:4:4 remains unsupported.

## 0.9.4-dev.2 - Color range fixes and event logging (2026-10-16)

Changes since v0.9.4-dev.1.

**BUD, vendor-specific HEVC decoding, Zero copy and 10-bit HEVC are experimental.**
For stability-first use, select WebRTC with H.264 and leave experimental options
disabled, or use the published stable v0.9.3 release.

- **Correct video black and white levels:** translate limited/full-range metadata correctly between Media Foundation and D3D11, including vendor HEVC output. This fixes lifted blacks and grey-looking whites in normal GPU rendering. Full-range support is declared consistently at startup and in later encoder requests.
- **Experimental 10-bit HEVC:** an optional Video setting requests Main 10 with WebRTC or BUD when the selected GPU exposes the required P010 decode/conversion capabilities. Later encoder and keyframe requests preserve the selected flags. Main 10 uses the existing GPU Media Foundation path and conversion pass; RGB display output remains 8-bit, and original-surface Zero copy remains 8-bit only. Host and driver support are required: the request does not guarantee a 10-bit stream. Connection stats shows the actual decoded color depth. 4:4:4 remains unsupported.
- **Optional operational event log:** enable event logging in the Network tab to record connection and route changes, transport failures, decoder recovery/errors and profile-save failures beside settings.json. UTF-8 JSON Lines records have UTC timestamps; background file writes use a bounded queue and approximately 5 MiB rotation with one previous file. Credentials and media payloads are excluded. Logging is disabled by default and applies immediately after saving.
- **Readable settings and statistics:** fix clipped event-log help in Settings and size the Connection stats explanation to its wrapped text, including the longer BUD description and narrower windows.
- **Clearer core provenance:** README documentation explains that the embedded proprietary Parsec WASM core remains unmodified and that Rust supplies the native framework rather than a rewrite of that core.

## 0.9.4-dev.1 - Experimental BUD and GPU video paths (2026-10-15)

Changes since the stable v0.9.3 release.

**BUD, vendor-specific HEVC decoding and Zero copy are experimental.** For
stability-first use, select WebRTC with H.264 and leave the experimental options
disabled, or use the published stable v0.9.3 release. H.264 remains the default
and uses Media Foundation on every GPU vendor.

- **Experimental BUD transport:** enable Experimental BUD in Settings to use the native UDP protocol with a regular Parsec host. Direct UDP, automatic fallback to a configured UDP TURN relay and Force TURN are supported. WebRTC remains the default and retains TURN over UDP, TCP and TLS; BUD supports UDP only and does not silently switch to WebRTC.
- **Experimental vendor-specific HEVC decoding:** when Prefer H.265 / HEVC is enabled, normal 8-bit HEVC playback uses Direct D3D11 on NVIDIA, AMF on AMD and VPL on Intel. These paths keep decoder output on the GPU and feed the existing presentation path directly. The existing GPU Media Foundation fallback/recovery remains available when a vendor path cannot be used.
- **Experimental Zero copy presentation:** the Video tab can select original NV12 decoder-surface presentation through Direct D3D11 and Windows Presentation API on supported NVIDIA, AMD and Intel GPUs. This removes the application's conversion/output pass and introduces no intermediate decoded-frame copy or CPU pixel readback. Windows 11 and compatible hardware/drivers are required; internal Windows/driver copies and direct scanout are not guaranteed.
- **More reliable HEVC startup and recovery:** handle short arrival bursts without exhausting presentation surfaces or unnecessarily abandoning the working Intel decoder. Original-surface presentation preserves the picture's aspect ratio and can recover to normal GPU rendering when unavailable.
- **BUD connection statistics:** show native RTT, decoded FPS, incoming/outgoing traffic, direct or relay route, host encode time when available and BUD delivery counters. Irrelevant WebRTC ICE/DTLS fields are hidden in BUD mode, and the last valid host encode time no longer flickers between reports.
- **Network and Video settings tabs:** connection servers, credentials, routing and transport settings are separated from GPU, codec and presentation choices. Both tabs share Save settings and retain unsaved edits when switching tabs.
- **Clearer playback status:** Connection stats uses shorter Zero copy, decoder fallback and playback-error descriptions while retaining detailed diagnostic information separately.
- **Safer host launch selection:** choosing the unsupported native-client launch option now explains that the web launch option is required instead of crashing the application. The web launch option is also used when BUD is selected in Settings.
- **Less input and shared-lock overhead:** reuse bounded input-event storage, pass native input arguments without temporary vectors and copy queued control messages outside the shared connection lock. Event ordering, immediate input wakeups and WebRTC reliability remain unchanged.

## 0.9.3 - Native Windows client (2026-10-14)

Changes since the previous stable release, v0.7.0.

- **Native Windows client:** the Rust client replaces Tauri/WebView2. Run a single portable EXE without a browser runtime. The embedded Parsec core is precompiled, so it needs no download or compilation at startup.
- **H.265 / HEVC playback:** enable Prefer H.265 / HEVC to request an 8-bit stream. H.264 remains the default and fallback. Playback uses Windows Media Foundation and D3D11; HEVC requires compatible host, GPU, driver and Windows decoder support.
- **Video GPU selection:** choose the adapter for stream decoding and rendering on multi-GPU systems, or keep Automatic. Virtual display-only adapters no longer appear as duplicate GPUs.
- **More responsive controls:** input wakes processing immediately. Native relative mouse input and cursor warp handle desktop drags correctly; immersive Mouse, Keyboard and Both modes capture the selected input while the app has focus. Custom Parsec hotkeys are respected.
- **Flexible connection routing:** use Cloudflare or custom STUN/TURN servers over UDP, TCP or TLS. STUN-only excludes relays; Force TURN relay keeps the connection on a relay even when a direct route is available, including over a VPN.
- **More reliable TURN connections:** forced relay setup signals readiness without waiting for every transport endpoint. TURN channel bindings renew before the five-minute boundary, addressing the reported ExpressTURN interruption. Multiple server addresses can be entered on separate lines.
- **Compatible overlay and fullscreen:** software drawing is the default for the interface, resolving the reported Intel overlay flicker while streamed video retains its GPU path. The native menu hides in fullscreen.
- **Reliable session controls:** overlay disconnect returns to the computer list. Settings opens and closes more reliably; coordinated control painting and scroll limits reduce redraw flicker.
- **Stream efficiency and recovery:** reuse media buffers and GPU video views, shorten shared locks, disable extra picture processing and request low-latency decoding. Bounded keyframe recovery helps restart video after temporary decoder errors or overload. Results depend on the driver and host.
- **Native sound and saved preferences:** Opus playback uses native Windows audio with device-supported low-latency output and fallback. Sign-in and Parsec preferences persist in an encrypted Windows-user profile; existing server settings remain compatible.
- **Connection stats and About:** separate native statistics show actual direct/TURN routing, relay details, traffic, RTT, decoded video format, decoder, GPU, audio format and app resource usage. About compares the embedded core version with Parsec's public version without installing a new core.
- **Stable update notifications:** background checks and Help > Check for updates show stable releases and their notes. Download the ZIP through your browser and replace the EXE manually while keeping your settings.

## 0.9.3-dev.1 - Connection and input improvements (2026-10-12)

- **Force TURN relay:** a new Settings checkbox keeps the next connection on a TURN relay even when a direct route is available, including over a VPN. Conflicting STUN-only settings and missing custom TURN servers are checked before connecting.
- **TURN transport preference:** relay connections prefer UDP, with TLS and then plain TCP available as timed fallbacks when the preferred route has not connected.
- **Working immersive input:** Mouse confines the pointer to the client window, Keyboard forwards reserved shortcuts while the client has focus, and Both combines them. Capture is released on focus loss and disconnect; Windows security shortcuts remain controlled by Windows.
- **Smoother relative mouse control:** host cursor signals now activate native Raw Input correctly, including during desktop drags. Changing direction no longer requires moving back from outside the client. Cursor Warp restores the host cursor position when leaving relative mode and respects its overlay setting.
- **More compatible overlay:** software drawing is now the default for the interface, resolving the reported Intel overlay flicker. Video decoding and presentation retain their GPU path without adding decoded-frame CPU readback.
- **Reliable disconnect:** disconnecting from the overlay returns to the computer list instead of freezing on a black screen. Callbacks from an ended stream cannot reveal its old video surface.
- **Cleaner fullscreen:** the Settings, Connection stats and Help menu disappears in fullscreen and returns in windowed mode.
- **Custom hotkeys:** Parsec hotkey assignments take priority when they conflict with native F8/F11, Settings or Connection stats shortcuts. Hotkey recording and saving remain handled by the original core.
- **Less allocation and lock contention:** audio and video reuse bounded input buffers, and media hand-off holds the shared connection lock for less time.
- **Video recovery:** after a temporary decoder error or input overload, the client can request a fresh keyframe and restart decoding. Recovery has bounded retries and depends on the host supplying the requested keyframe.
- **Low-latency HEVC:** the 8-bit HEVC decoder also requests low-latency operation through Windows transform attributes when its codec API does not accept the existing request. The benefit depends on the decoder and driver.

## 0.9.2-dev.1 - H.265 / HEVC video (2026-10-11)

- **H.265 / HEVC support:** enable **Prefer H.265 / HEVC** in Settings to request an 8-bit HEVC stream on your next connection. The client checks decoder support on your selected GPU first; H.264 remains the default and is used when HEVC is unavailable or the host chooses H.264.
- **Native HEVC playback:** HEVC uses the Windows decoder and the existing D3D11 video path. Successful video playback has been verified with a Parsec host.
- **HEVC details in Connection stats:** see the received codec and profile, decoder, resolution and graphics adapter. These describe the stream actually received, rather than just your selected preference.

## 0.9.1-dev.1 - Faster startup and more responsive controls (2026-10-10)

- Mouse and keyboard input now wakes processing immediately instead of waiting for the next regular tick. Local testing showed substantially shorter input processing times; actual responsiveness depends on your system and connection.
- The bundled Parsec core is precompiled during the build, so the app no longer compiles it at startup.
- The GPU selector filters out virtual display-only adapters that could appear as duplicate graphics cards.
- The statistics window now has the title **Connection stats** in its Windows title bar.

This first published native preview also includes the connection settings, live statistics and other native-client features listed under 0.9.0 and 0.8.0 below.

## 0.9.0 - More connection options and live stats (dev, 2026-10-10)

### New

- **Video GPU selector:** choose the graphics adapter used for native stream decoding and rendering in Settings. The choice is saved and applies to your next connection; Automatic keeps the Windows default.
- **Choose your own connection servers.** Configure custom STUN servers and TURN relays, including Cloudflare, coturn, eturnal and ExpressTURN. TURN connections support UDP, TCP and encrypted TLS, including servers on port 443.
- **STUN-only mode for LAN/VPN use.** Discover a direct connection without offering a TURN relay. Leave the STUN field empty to use Parsec's default server.
- **Keep your previous server settings.** The native client can read the earlier WebView2 client's `settings.json`, including Windows-protected credentials.
- **A separate Connection stats window.** See connection delay, traffic, video FPS and resolution, audio format, encryption status and app CPU/GPU usage. The window identifies confirmed direct or TURN connections and the relay server when available. Unavailable details are left unknown or omitted.
- **Client-side decoder details.** See the video decoder, renderer and graphics adapter. Decoder-specific GPU surfaces are shown as evidence of likely hardware decoding, rather than a guarantee.
- **Help menu with version information.** Check both the ParsecWebTurn version and the embedded Parsec core version.

### Improved

- **More responsive input:** mouse and keyboard events now wake their processing loop immediately instead of waiting for its next regular tick. The regular interface callback and video presentation keep their existing pacing. Actual latency improvement depends on system load and the Parsec core.
- **Faster startup:** the bundled Parsec core is compiled during the build and loaded directly when you open the app. No download or compilation is needed at startup.
- The Video GPU selector excludes virtual display-only adapters that can appear under the same name as a real GPU. Separate real GPUs remain selectable, even when their names match.
- Disable the video driver's automatic extra picture processing and reuse GPU video views and stream-format details. Cached views are released when the stream changes or the window is resized. Presentation queue behavior is unchanged.
- A clearer, Parsec-inspired **Settings** page with descriptions, provider-specific fields and dark controls.
- Fixed overlapping text, clipped buttons and scrolling/repainting problems in Settings. Improved the stats window's resizing, scrolling and handling of ended sessions.
- Sign-in and Parsec preferences now save correctly between launches.
- Disconnecting from the overlay or closing the app sends a disconnect notification to the host instead of leaving it waiting for a timeout.
- The window keeps the **ParsecWebTurn** name so it is clearly distinct from the official client.
- Windows gives the audio and video workers multimedia scheduling priority when available. Audio output requests the shortest device-supported playback period, with automatic fallback to the previous audio path. These changes target local playback delay; the benefit depends on your device and workload.
- Removed obsolete WebView2 files and kept diagnostic tools out of the normal app.

### Before you use this build

- This is still a Windows native development build. Vulkan, D3D12, Linux and Android support are not included.
- Audio uses Opus. Selecting RAW in the embedded interface did not produce a RAW stream in testing; RAW playback is not supported.
- Enabling TURN can result in a relay being selected even when a direct path is possible. Use **STUN only** if you want to exclude relays.
- Client-to-host clipboard paste remains a known issue in the tested Parsec session; it also reproduced in standalone Edge.

## 0.8.0 - Native Rust client (dev)

- Replace the WebView2 launcher with the native Rust client using the pinned Parsec WASM core (150-104a).
- Embed the core in one executable; no separate WASM file, launcher script or WebView2 runtime is required.
- Keep sign-in and client preferences in an encrypted, current-Windows-user profile across restarts. Previous Tauri configuration and profiles remain untouched.
- Present GPU-resident decoded video through the native Windows graphics path; add native Opus/WASAPI audio, keyboard, pointer and the original Parsec overlay. Relative pointer capture remains controlled by the overlay.
- Enable legacy RSA-1024 host identity compatibility while retaining signature and certificate fingerprint verification.
- Remove the prototype's forced Cloudflare STUN setting and exclude diagnostic commands, synthetic fixtures and test-only readback from the normal executable.

Custom server settings and native connection statistics were added in 0.9.0 above.

## [0.7.0] - 2026-10-02

### Added

- Separate STUN and custom TURN address fields with automatic migration of existing mixed address lists.
- Add **STUN only - no TURN relay** for direct connections. Cloudflare mode can use its public STUN endpoint without an API token or TURN credential generation. Custom STUN addresses also work alongside Cloudflare TURN.
- Show reported WebRTC DTLS state, version, cipher suite and SRTP protection profile in Connection stats.
- Show CPU usage for the application and its WebView2 processes, plus WebView2 GPU engine and video-decode utilization when available.
- Show detected audio codec, sample rate and channel count, retaining these values during silence. The sender's configured bitrate remains unknown when the receiving APIs do not expose it.

### Fixed

- Configure clipboard-read permission for the exact Parsec web origin using the documented WebView2 profile API, fixing empty local Clipboard API reads.

### Known limitations

- Client-to-host paste still fails in the tested Parsec session and also reproduces in standalone Edge. The clipboard permission change does not resolve Parsec's end-to-end clipboard synchronization.
- With TURN enabled, native ICE selects the connection route; a reachable direct path does not guarantee that it selects a direct connection. Use STUN-only when TURN must be excluded.

## [0.6.0] - 2026-10-01

### Added

- Background GitHub stable-release notifications and App > Check for updates.
- Native update window with release notes and a direct link to the official Windows ZIP, opened in the default browser only after a click.
- Bounded metadata-only HTTPS checks, stable numeric version comparison and exact repository ZIP link validation.

### Changed

- Hide detailed video statistics, including codec and resolution, when optional Media diagnostics are disabled; saved preference changes apply to their visibility immediately.
- Audit the runtime against public web specifications and documented platform APIs; record ongoing requirements in AGENTS.md and findings in docs/STANDARDS.md.
- Replace private Tauri frontend IPC access with the documented global core API; keep native capability restrictions.
- Make experimental Chromium Media diagnostics explicit opt-in, disabled for both new and legacy settings. Standard WebRTC/WebCodecs telemetry remains the baseline.
- Remove production Chromium launch switches and legacy vendor-prefixed WebRTC/statistics fallbacks.
- Preserve native WebCodecs callback dictionary conversion, inherited/frozen callbacks and callback capture semantics.
- Update notifications do not download or run executables, create a helper, replace the app or restart it. Updates are installed manually and follow the browser's organization policies.
- Correlate selected ICE endpoints with the ICE transport and gathered local/remote candidates to identify additional direct paths, including VPN connections. Positive TURN evidence retains priority; incomplete or ambiguous evidence stays unverified.
- Label confirmed direct routes as "Direct - no TURN"; VPN routing underneath WebRTC does not imply TURN use. Candidate addresses remain inside the WebView and are not included in telemetry.

## [0.5.0] - 2026-09-30

### Changed

- Replace the Go launcher and external Edge app window with a Rust/Tauri 2 application using Windows WebView2.
- Integrate settings, the Parsec window, developer tools and connection statistics into the same application.
- Embed the ICE override and UI in the executable; no extension directory or generated credential script is needed.
- Preserve v0.4.0 settings and CurrentUser DPAPI encryption. WebView2 uses a new profile and requires a fresh Parsec sign-in.
- Port Cloudflare requests, custom TURN, encrypted caching, validation and local ice.json fallback to Rust.
- Keep the direct/relay policy selected by Parsec; no forced relay mode.
- Replace ICE configuration spreading with a native WebIDL dictionary adapter, preserving inherited/getter fields, frozen inputs and native argument errors for constructors and setConfiguration.
- Validate STUN and TURN URI syntax separately and normalize case-insensitive scheme names.
- Build and validate the Windows distribution with Cargo and a committed lockfile.

### Added

- Live WebRTC traffic rates, connection RTT, direct/relay route and transport diagnostics.
- Show selected local/remote ICE candidate types; ambiguous candidate pairs remain unknown.
- Read Chromium Media codec, decoder, hardware decoding, profile and visible resolution, including stream reconfiguration.
- Native F11 fullscreen and Ctrl+Shift+W windowed recovery; block automatic web fullscreen and Escape locking.
- Proper App/View submenus and dark native windows.
- Video codec, decoder, FPS, resolution and packet-loss details when provided by the web client.
- Count actual WebCodecs decoded frames for data-channel video FPS; distinguish unavailable video packet loss from zero loss.
- Capture fullscreen recovery shortcuts before Parsec's handlers, including when the native menu is hidden.
- Mark peer-reflexive ICE paths as unverified instead of assuming direct routing.
- Recognize Chromium peer-reflexive TURN paths by the selected relay transport; show configured TURN usage, matched server and route evidence. Use the selected ICE transport pair when report selection is ambiguous.
- Restricted native command access: the remote Parsec page can submit bounded statistics but cannot read or change saved settings.

### Fixed

- Hide connection settings during automatic startup; show the ready form on first run, explicit settings launch or configuration/connection errors.
- Closing the Parsec window exits the application instead of reopening connection settings; internal window replacement during reconnect remains supported.

### Requirements

- Microsoft Edge WebView2 Runtime must be present. The runtime is separate from the Edge browser; the app does not silently install it.
- HEVC decoding and the previously reported long-session freeze remain separate issues; this migration does not claim to fix them.

## [0.4.0] - 2026-09-30

### Added

- Custom STUN/TURN provider configuration for standard services such as coturn and eturnal.
- Multiple server URLs with a common username/password; static and externally generated credentials, UDP/TCP/TLS endpoints and STUN-only configurations are supported.
- Windows DPAPI protection for saved custom TURN passwords.
- Provider switching retains Cloudflare and custom configuration without requiring re-entry of secrets.
- Custom provider, legacy migration, encrypted storage, no-network startup and native form-save tests.

### Changed

- Suppress Edge's automatic translation offer in the dedicated app profile while preserving other browser preferences.
- Modern settings window with a dark header, Parsec-inspired pink accents, a spaced connection card and inline validation.
- Provider-specific fields are shown only when relevant; custom mode requires no Cloudflare key, TTL or API request.
- Generalize startup errors to cover both providers.

### Security

- Custom credentials are encrypted in settings, but the generated browser injection necessarily contains the active TURN credential. For static custom accounts this credential is long-lived. Do not share the generated injection or profile.
- TURN REST shared secrets are not accepted as client passwords; generate the temporary username/password on your server and enter those values instead.

## [0.3.0] - 2026-09-29

### Added

- Optional DPAPI-encrypted TURN credential cache with conservative expiry checks and invalidation on Key ID, API token or TTL changes.
- Dedicated Edge profile detection and launcher locking to prevent stale configuration reuse and concurrent writes during startup.
- Go tests covering ICE/API validation, Windows DPAPI, credential caching, file replacement, profile locking and injection generation.
- JavaScript regression tests for construction, subsequent configuration updates, inheritance, duplicate injection and credential-free logging.

### Fixed

- ICE overrides now also apply to `RTCPeerConnection.setConfiguration()`.
- Invalid ICE entries and missing TURN authentication are rejected before Edge starts.
- Browser-blocked port 53 endpoints are removed from ICE configurations.
- TURN usernames/passwords are no longer printed to the console.
- Windows messages use real newlines; fallback errors retain both the API and fallback failure.
- Settings, cache and generated injection use temporary files with Windows replacement semantics.
- Local builds embed the same Common Controls v6 manifest as release builds.

### Changed

- Split the launcher into small files for settings, UI, DPAPI, Cloudflare, ICE, cache, files, profile, extension and Edge operations.
- Replace version-specific release workflows with one tag-triggered release workflow and a shared build/package script.
- Commit dependency checksums, pin the resource generator and store the icon locally; remove build-time icon downloads/conversion and `go mod tidy`.
- Add a TLS TURN port 443 fallback to the example configuration.
- Inject the executable version into the API User-Agent at build time.

### Compatibility

- Existing settings files still work; credential caching is disabled unless enabled in Settings.
- Credentials are refreshed at launch, not automatically within an open browser session. Close the dedicated Edge profile before relaunching and choose a TTL longer than your expected session.

## [0.2.1] - 2026-09-29

### Fixed

- Fixed `TTM_ADDTOOL failed` when opening the native settings window on affected Windows systems.
- Embedded a Windows application manifest enabling Microsoft Common Controls v6, which the Walk GUI toolkit requires for reliable tooltip/control initialization.


## [0.2.0] - 2026-09-29

### Added

- First-run native Windows settings dialog.
- Direct Cloudflare Realtime TURN credential generation using TURN Key ID + API token.
- Automatic fresh short-lived TURN username/password generation on every launch.
- Configurable credential TTL up to 172800 seconds / 48 hours.
- Windows DPAPI Current User protection for the stored Cloudflare API token.
- `--settings` and `/settings` options to reopen configuration.
- Local `ice.json` fallback when the Cloudflare API cannot be reached.
- Parsec icon embedded in the Windows executable.
- Explicit third-party/trademark notice for the icon.

### Changed

- `ice.json` is no longer required for normal use.
- Normal startup now obtains `iceServers` from Cloudflare before launching Edge.
- README expanded with Cloudflare setup and security model.

### Security

- The long-lived API token is not stored in plaintext.
- The token is encrypted using Windows DPAPI and bound to the current Windows user context.
- Short-lived TURN credentials may still appear in generated `extension/inject.js` for the duration of their validity.
- Backend-free mode assumes the local Windows user/device is trusted.

## [0.1.0] - 2026-09-29

### Added

- Initial public release of ParsecWebTurn.
- Portable Windows x64 launcher with no administrator rights required.
- Microsoft Edge app-mode launcher for `https://web.parsec.app/`.
- Automatic WebRTC ICE override at `document_start`.
- Cloudflare STUN/TURN support through external `ice.json` configuration.
- Separate local Edge profile stored beside the application.
- Example TURN configuration in `ice.example.json`.
- Manifest V3 helper extension scoped to `https://web.parsec.app/*`.
- GitHub Actions build workflow.
- MIT license.
