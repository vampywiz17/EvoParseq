# EvoParseq for Android TV

Android preview: **0.9.6-dev.2** on `dev` and `codex/linux-flatpak`. First live device: Xiaomi TV Box S
(3rd Gen) EU, Android 14 (32-bit ARM userspace). Google TV / Android TV interface,
D-pad navigation, two-factor sign-in and shared Rust account/session logic.

## Streaming preview

- Connect to an unmodified official Parsec host using **WebRTC direct/STUN**
  (default) or **BUD direct UDP**.
  Configure Cloudflare or custom STUN/TURN in Network settings. WebRTC uses its
  existing ICE selection; BUD tries direct UDP and configured TURN carriers in
  UDP/TLS/TCP order (or TURN only when forced). Neither transport silently falls
  back to the other. No browser, WASM or stock SDK is bundled.
- Hardware **MediaCodec to SurfaceView Surface**, using asynchronous NDK callbacks.
  Decoded video never crosses JNI, becomes a Bitmap, or is read back to CPU memory.
  This does not prove driver/compositor zero copy or direct scanout.
- H.264 8-bit 4:2:0 is the default. Request HEVC 8/10-bit 4:2:0 in Settings when
  supported. The host chooses the actual format; Main10 is not HDR.
- Shared Rust Opus decoder, bounded queues and keyframe recovery; stereo 48 kHz
  AAudio pull output. The callback never waits on the session worker or allocates.
- Sign-in is remembered using AES-256-GCM and a non-exportable Android Keystore
  key. Only the session token is saved, encrypted in app-private no-backup storage;
  passwords and 2FA codes are never saved. Sign out removes local credentials and
  requests server-side revocation. Invalid/expired credentials require sign-in.
- Back/Disconnect, backgrounding, audio focus loss and Surface destruction stop
  the stream. Reconnect explicitly when a new Surface is created.
- Network/Video settings are grouped for D-pad use. Fixed FPS requests host
  `fullFPS`; dynamic FPS allows the host to reduce frame production when idle.
- The overlay shows sampled Surface submission FPS, selected-pair RTT, received
  payload bitrate and actual codec/depth. It hides after five idle seconds;
  OK/Menu/Back restores it, with Hide and Disconnect buttons. RTT queries pause
  while hidden; no GPU timing or pixel readback is introduced.
- Shared Rust controller state/lifecycle and full-state protocol on BUD and WebRTC.
  Android reads standard gamepad/joystick events; Windows uses native XInput.
  Up to four pads; press a face button after hiding the overlay to join the host.
  Local menus/focus loss release controllers; removal sends an explicit unplug.
  Xbox-style buttons, D-pad, sticks and triggers are supported. Sony raw touch,
  gyro and adaptive-trigger passthrough are not implemented. Device compatibility
  and vibration require live testing. Android vibration uses the controller's
  own vibrator, if exposed, without assuming separate motor identities.
  No HDR, 4:4:4, AI upscaling or rotated BUD output is advertised.

Live video is confirmed on the Xiaomi box for H.264 and HEVC 8-bit, and HEVC
Main10 with NVIDIA encoding. The AMD host test returned 8-bit. The user found
BUD at 50 Mbps stable in the later build, while sustained WebRTC 50 Mbps still
caused growing delay and video-channel closure on Ethernet and Wi-Fi. WebRTC
around 30 Mbps was stable. The earlier proposed universal 30-35 Mbps hardware
ceiling was not supported once the two transports were compared.
A later apparent audio regression was traced to the box's own audio output;
it was not established as a client regression.

Temporary callback/PCM peak counters and verbose test overlays have been removed.
Useful errors, bounded recovery and native-peer/queue regression tests remain.
Decoded-output catch-up still releases every codec index exactly once and never
skips compressed reference pictures. Empty WebRTC messages remain distinct from
stream EOF, as required by RFC 8831. Test both transports, audio, reconnect and
remembered sign-in on the actual device; builds alone do not validate hardware.

For fixed-FPS sessions, the video Surface receives Android's documented frame-rate
hint using the host-reported configured rate, not sampled FPS or the requested
preset. Dynamic/unknown cadence clears the hint. Only seamless mode changes are
allowed; there is no new frame scheduler, buffering, copy or forced HDMI mode.
The overlay exposes Android's reported display Hz and decoded-output catch-up
skip count to help distinguish cadence mismatch from catch-up. This is not a
measurement of physical scanout; the user reported smoother lateral motion
after this change, without a quantified latency measurement.

## Local relay setup

Open **Settings > Network > Set up from phone or computer**. The TV and browser
must be on the same private LAN. Scan the QR code or enter the displayed HTTPS
address. This is an expiring server on the TV, not an Internet setup service.

The certificate is generated locally for this setup session and is not publicly
trusted. Before entering any credentials, inspect its SHA-256 fingerprint in the
browser's certificate details and compare it with the TV. Only trust this exact
local certificate after verifying the fingerprint; do not disable browser TLS
checking globally. If your browser cannot show the certificate details, use a
browser/device that can. A pairing code alone does not authenticate the HTTPS
page against an active intermediary.

Enter the TV's six-digit code, choose direct/STUN, Cloudflare (TURN key ID and API
token), or custom TURN (URLs, username/password), then send. **Approve and save
on the TV** before anything is applied. Credentials are encrypted separately
from account sign-in using Android Keystore, excluded from backup and never
included in status/log output. Network settings take effect on the next stream.
Closing setup, leaving the app, starting a stream, or the five-minute deadline
stops the server. No port forwarding, public binding or remote setup is provided.

Local setup and BUD streaming have been live-tested by the user.
HTTPS, origin/Host checks, bounds, wrong-code rejection and pending approval have
isolated tests; they do not replace real browser/TV and TURN tests.

## Shared architecture

The Android crate compiles the existing account, authenticated signaling,
WebRTC/ICE/DTLS/SCTP and BUD, control framing, audio decoder, video inspection/queue and
request-validation sources directly. Android-specific code only handles JNI,
Surface/MediaCodec, AAudio and TV UI/lifecycle. Windows retains its own media and
input backends. No decoded-frame conversion/copy layer is added to Windows.

## Build / install

The `EvoParseq Android TV preview` workflow runs common Rust tests/Clippy,
ARM64/ARMv7 builds and Clippy, and Android release assemble/lint. It produces a
side-load APK signed with the persistent distribution key, not a Play Store
release. Older debug-signed previews require the one-time migration described
under In-app updates below.

Prerequisites: JDK17, Gradle8.13, SDK36/build-tools36.0.0, NDK28.2.13676358,
Rust and cargo-ndk4.1.2. See `.github/workflows/android-tv.yml` for the exact
per-ABI Opus CMake toolchain and build commands. No local signing keys or
`local.properties` belong in git.

CI APK: `EvoParseq-TV.apk` (from `app/build/outputs/apk/release/app-release.apk`).
Local `assembleDebug` builds remain debug-signed and cannot replace the signed
distribution build. Both ARM64 and ARMv7
native libraries and Opus are included; no extra installation/root is required.

References: [TV design](https://developer.android.com/design/ui/tv),
[MediaCodec](https://developer.android.com/ndk/reference/group/media),
[AAudio](https://developer.android.com/ndk/guides/audio/aaudio/aaudio).

EvoParseq is an independent community project, not affiliated with or endorsed by
Parsec/Unity, Xiaomi, Google or lowlat. Parsec account/service terms still apply.

## Controller preview validation

Pair an Android-recognized USB/Bluetooth gamepad with the TV, connect, wait for
(or hide) the overlay, then press A/B/X/Y. Use the TV remote to reopen controls;
controller buttons are forwarded only while the overlay is hidden. After focus
loss, release held buttons before joining again. On Windows, use an XInput
controller and focus the stream with local menus closed. DirectInput-only pads
need a separate future Windows adapter; no remapping driver is installed here.
Home/Guide remains OS-owned. The unchanged official host needs its normal
gamepad driver and guest permission.
Test both transports, button chords, sticks/triggers, unplug/reconnect, focus
loss, local menu isolation and device-supported rumble. Synthetic tests/builds
are not evidence of live controller compatibility.

## Desktop input preview

The stream controls can enable remote mouse navigation: D-pad moves, a short OK
clicks left, two short OK presses double-click, holding OK clicks right, and
holding OK while pressing D-pad drags with the left button. Back/Menu opens local
controls; Home remains Android-owned. The remote that enabled mouse navigation
has priority; other standard D-pad remotes also work. A separate gamepad keeps
its own directional controls.
Some remotes cannot report simultaneous
OK and direction presses; dragging requires a remote that supports that chord.

Physical USB/Bluetooth keyboards and mice use normal Parsec input, on BUD and
WebRTC. Mouse capture is enabled by default while the overlay is hidden, and
can be disabled in the overlay. Ctrl+Shift+M releases input and opens controls.
Focus loss and device removal release held input. Physical keyboard scan codes
preserve key positions; matching host/client keyboard layouts are recommended.
This does not enable the Windows BUD HID passthrough mode.

Mouse gestures, keyboard encoding and bounded input handoff are shared Rust;
Android only translates OS events/capture and draws a small cursor overlay.
The MediaCodec-to-Surface video path is unchanged. Remote D-pad mouse navigation,
clicks and long-press right click have been live-tested. Continue device-specific
checks for physical input, relative camera control and focus transitions.


## In-app updates (0.9.6-dev.2)

Settings > Updates checks published EvoParseq GitHub releases. Stable is the
initial channel; Include prereleases also accepts `-dev.N` releases. The check
selects the newest supported version with an Android APK, so a Windows-only
release does not hide an available Android update. Updates are manual checks,
not a streaming/background poll. Download and release policy run in shared Rust
on the existing account worker; Android owns package inspection and installation.

Choose Download update, then Install update. Android may first ask you to allow
EvoParseq to install apps; return and select Install update again. The system
installer asks for confirmation. Cancelling retains the installed client.
Successful in-place updates retain settings and remembered sign-in. No root,
ADB, broad storage access or browser is required.

Downloads are bounded to 128 MiB, HTTPS-only from the fixed repository and
approved GitHub asset redirect hosts. The GitHub asset SHA-256 digest and size
are checked before Android receives the file. The adapter requires the same
package ID and current signing certificate, a higher Android versionCode, a
matching versionName/release tag and a compatible minimum SDK. Android's system
installer performs final package/signature verification. No downgrade or
cross-certificate update is offered. Prepared downloads stay in private app
storage and are removed after staging; stale uncommitted sessions are cleaned
on startup. No credentials enter the update requests.

CI now produces **EvoParseq-TV.apk**, signed with the persistent distribution
key from `ANDROID_TV_KEYSTORE_BASE64` and `ANDROID_TV_KEYSTORE_PASSWORD` repository
secrets. The key is not committed, included in artifacts or printed in logs.
Missing signing secrets fail the release build; an ephemeral debug key is never
substituted. Local debug builds remain available for development.

Existing debug-signed preview installations generally cannot update in place
to this distribution key. Install the signed build once after removing the old
one; removal clears that installation's private data, so sign in and configure
it again. Do not uninstall an already distribution-signed installation for a
normal update.

For each published update, increment versionCode and set versionName to the
release tag without its leading `v`. Upload the signed **EvoParseq-TV.apk** as
a release asset (not just an Actions artifact), with GitHub's `sha256:` asset
digest available. Publish the matching stable tag or mark a `-dev.N` release as
a prerelease. The updater does not publish releases or install Actions artifacts.
Keep the signing key/password backed up securely; losing it prevents ordinary
updates of existing installations. The Android APIs used are documented under
[PackageInstaller](https://developer.android.com/reference/android/content/pm/PackageInstaller)
and [application signing](https://developer.android.com/studio/publish/app-signing).
