> [!WARNING]
> **EvoParseq is developed with substantial AI assistance.** AI generates most of the
> implementation code, while the project’s concept, architecture, scope, platform choices,
> and development priorities are defined by its human maintainer.
>
> Development remains human-led. The maintainer directs the work, tests the application on
> real hardware, investigates reported problems, and oversees patches and fixes.
> Responsibility for reviewing and accepting changes stays with the maintainer.
>
> The project owner has a formal IT education and works in Kubernetes and DevOps. The
> primary motivation for using AI is to save time and make effective use of LLM
> capabilities, allowing more effort to go into design decisions, integration, and practical
> testing.

<p align="center">
  <img src="assets/evoparseq-wordmark.svg" width="360" alt="EvoParseq">
</p>

<p align="center"><strong>An independent Parsec client with a shared native Rust core.</strong></p>

EvoParseq connects to existing, **unmodified Parsec hosts**. Native Rust handles
sign-in, computer discovery, signaling, transport, GPU video, audio and input.
It does not bundle or run the proprietary Parsec WebAssembly client core and
requires no browser, WebView or helper executable.

**This is an independent community project, not affiliated with or endorsed by
Parsec, Unity or lowlat.** A Parsec account and a compatible host are required.

**This is the development branch: Windows, Android TV and experimental Linux Flatpak support.**
The latest stable Windows/Android release is
[**EvoParseq v0.9.7**](https://github.com/vampywiz17/EvoParseq/releases/tag/v0.9.7)
on `main`. The current dev release,
[**v0.9.8-dev.2**](https://github.com/vampywiz17/EvoParseq/releases/tag/v0.9.8-dev.2),
includes Windows ZIPs, the signed Android TV APK and the experimental Linux Flatpak.
All three report version **0.9.8-dev.2**; development continues here on `dev`.
Linux development artifacts are also available from successful
[dev Linux CI runs](https://github.com/vampywiz17/EvoParseq/actions/workflows/linux-flatpak.yml?query=branch%3Adev).
Windows previews are installed manually; signed Android builds can check the
prerelease channel in Settings > Updates. Platform capabilities differ; see below.

## Linux Flatpak preview

- **Wayland + Vulkan:** egui desktop UI follows the Windows client's shared
  wording, logical dimensions and layout, with native desktop window decorations.
  Login/computer picker and Settings have fixed sizes; the stream is resizable
  and uses centered aspect fitting. Connection stats is a separate fixed-size native window.
- **Shared Rust core:** account/2FA, discovery, session preferences, settings,
  WebRTC/BUD, media ingress, normal input and diagnostic models are reused.
  Linux adapters provide Vulkan Video/Vulkan, PipeWire, Wayland input and
  Secret portal credential protection.
- **Playback:** WebRTC and explicit BUD selection, progressive H.264 or capability-gated HEVC Main 8-bit 4:2:0
  and stereo WebRTC Opus or BUD Opus/PCM with optional matched 5.1/7.1 output. Original NV12 decoder surfaces feed the same-device Vulkan
  presentation shader; no application-side decoded-image copy or CPU readback.
  Shader conversion still costs GPU work: this is GPU-resident rendering,
  **not the application's Zero copy mode**.
- **Controls:** normal or opt-in BUD HID mouse/keyboard, controllers/rumble,
  focused text clipboard, orientation and immersive input. A
  draggable session icon, host display selection, resolution/bitrate/FPS/pacing
  requests, fullscreen, local PipeWire mute/volume and Disconnect. Host settings
  replies and Apply confirmation use the shared Rust models.
- **Diagnostics:** live overlay and Connection stats, with four Overview cards
  in a 2-by-2 layout. CPU/RSS are sampled at most once per second and only while
  Connection stats is open. No GPU utilization sampler or extra render pass is
  introduced; unavailable measurements remain explicitly unavailable.
- **Updates:** Help → About and GitHub Flatpak checks; verified Linux packages open
  in the system installer for user confirmation. CI artifacts must first be
  published as versioned release assets to be discoverable.
- **Network:** shared Windows STUN/custom TURN/Cloudflare configuration,
  direct-only and Force TURN for WebRTC and BUD; protected relay secrets and
  optional in-memory Cloudflare credentials. The QoS switch enables Linux DSCP AF21
  and SO_PRIORITY 6; network equipment may ignore these hints.
- **Persistence:** Windows-compatible `settings.json` schema and encrypted
  remembered sessions through the Linux Secret portal. Passwords and 2FA codes
  are never saved; protected credentials are not portable between platforms.

Linux currently requires a working Wayland desktop, PipeWire and a driver
exposing the necessary Vulkan Video H.264 decode and sampleable NV12 surfaces.
A Flatpak runtime cannot add missing GPU/kernel capabilities. Intel HD 520
H.264 and HEVC 8-bit playback have been reported working on the maintainer's HP EliteBook 840 G3, with Intel HEVC reconfirmed under Nobara Live.
AMD Radeon 780M H.264 playback has also been tested. These observations do not
establish support for every GPU, driver or codec. The package defaults to
`ANV_DEBUG=video-decode` for Mesa Intel's experimental video path.

**NVIDIA H.264 playback is also live-confirmed** on RTX 3060 with Nobara Linux,
on the same hybrid AMD/NVIDIA configuration (1920 × 1200, 8-bit 4:2:0).
NVIDIA HEVC and longer stability testing remain pending. On the tested hybrid Pop!_OS
system, RTX 3060 / driver 595.84 fails Vulkan presentation before decoding.
The failure also reproduces in native `vkcube` outside EvoParseq/Flatpak, including
with only the NVIDIA ICD selected. This establishes an independent system-level
reproducer. It resembles a known class of
[NVIDIA/Wayland presentation issues](https://github.com/gfx-rs/wgpu/issues/5382);
the exact cause and contribution of hybrid presentation remain unconfirmed.
The successful Nobara test shows that hybrid hardware alone is not a blocker;
no alternative decoder or cross-GPU copy fallback has been introduced.

HEVC Main10/P010 decoding is now available after selected-device capability checks,
with SDR presentation and live validation pending. Display timing VRR requests
adaptive presentation; physical VRR remains unverified.

**Not yet integrated on Linux:** 4:4:4 playback, the application's
Zero copy mode, AI upscaling, runtime GPU switching and verified VRR control.
The full Settings layout is present; unimplemented controls remain disabled.

Linux now implements up to three simultaneous host-listed display windows on
one BUD or WebRTC connection, with one decoder per active display and one audio
output. Additional windows have independent resolution controls and input;
Connection stats remains on the primary window. Multi-display playback is now
live-confirmed by the maintainer. GPU selection is saved with the shared Windows settings
schema and matching policy; changing the Vulkan GPU requires an app restart.

See [Linux installation, current scope and diagnostics](linux/README.md) and
the [video-path audit](linux/VIDEO_PATH.md). The Windows feature list below
should not be read as Linux feature parity.

## Android TV preview

- Google TV / Android TV interface with D-pad navigation, 2FA, grouped settings
  and encrypted remembered sign-in using Android Keystore.
- Shared Rust account, session, WebRTC/BUD, controller and input models; Android
  adapters provide MediaCodec Surface video, AAudio and OS input/lifecycle.
- Direct or STUN/TURN connections, Force TURN and temporary **local-only HTTPS**
  relay setup from a phone/computer, with fingerprint verification and TV approval.
- H.264 and HEVC 8/10-bit 4:2:0, fixed/adaptive FPS, hidden-by-default session
  controls after five idle seconds, plus FPS/RTT/bitrate/codec/route information.
- Standard gamepads; optional remote D-pad mouse, short/long OK clicks and
  supported OK + direction dragging. Physical mouse/keyboard use normal input
  with optional capture. Windows HID passthrough is not an Android feature.
- AAudio device recovery, optional custom buffering, BUD PCM and capability-gated
  5.1/7.1 output; diagnostic logging with save/share export. These additions need
  live audio-device validation; WebRTC remains stereo Opus.

Live-tested on Xiaomi TV Box S (3rd Gen), Android 14, including BUD at 50 Mbit/s,
NVIDIA-encoded Main10, remote mouse and a GameSir controller. WebRTC at sustained
50 Mbit/s still closed its video channel in this setup; around 30 Mbit/s was
stable. This is a transport-specific observation, **not a universal hardware limit**.
MediaCodec submits frames to Surface without CPU pixel readback; physical scanout
and internal driver/compositor copies are not verified.

Use the [preview release](https://github.com/vampywiz17/EvoParseq/releases/tag/v0.9.6-dev.5)
or successful [dev Android CI artifacts](https://github.com/vampywiz17/EvoParseq/actions/workflows/android-tv.yml?query=branch%3Adev)
for previews; the stable Windows ZIP is not an Android package.
[Android TV setup, build and limits](android-tv/README.md).
The Windows features below remain available on this branch.

## Features

- **Native account and sessions:** optional 2FA, encrypted remembered sign-in,
  background-refreshing computer picker and manual peer ID. Owner connections
  are supported; friend/invitation authorization is not implemented.
- **WebRTC or BUD:** direct or custom TURN connections over UDP/TLS/TCP,
  Force TURN, STUN-only and Cloudflare TURN credentials. No silent transport
  switch. Optional experimental Windows TCP/TLS QoS marks outgoing TURN traffic.
- **GPU video:** H.264 and H.265/HEVC, GPU selection, Media Foundation and
  experimental vendor HEVC decoders, 10-bit SDR output, original-surface
  zero-copy, and compatible NVIDIA/Intel HEVC 4:4:4 at 8 or 10 bits.
- **Session overlay:** draggable icon; resolution, bandwidth, FPS, constant FPS,
  orientation, fullscreen and immersive input controls. Switch the current stream
  between host-listed physical or virtual displays.
- **Windows multi-display (dev):** open up to three host-listed extended outputs
  on one BUD/WebRTC connection, with independent resolution controls and one audio
  pipeline. Normal input, activating clicks and cross-display window dragging are
  live-tested with two screens. Additional windows have their own draggable controls;
  Connection stats stays on the primary window. Configure Parsec VDD outputs on the
  host first; this feature does not create virtual displays or override host policy.
- **Audio:** stereo Opus, configurable buffering, BUD raw PCM and experimental
  multichannel output. Automatically reopen the Windows audio output after
  transient device errors or default-device changes, retiring stale audio.
  On Windows, the Session overlay offers immediate local mute and 0–100% volume
  control for either transport, without changing the host's volume. Mute preserves
  the chosen level; these controls reset when the client is restarted.
- **Controllers (development):** shared Rust gamepad state/protocol for up to
  four controllers, Windows XInput and Android gamepad events. Wired Windows
  and GameSir Android forwarding are live-tested; device-specific rumble and
  disconnect behavior need broader validation.
- **Input:** two-way Unicode text clipboard; normal mouse/keyboard forwarding;
  BUD USB HID mouse and keyboard, including relative camera input.
- **Diagnostics:** route, RTT, traffic, FPS, actual decoder/format, audio queues,
  host/local timings and system resource use. Optional bounded event logs record
  failures without continuous mouse/key activity.
- **Presentation:** Automatic, Low latency, VSync and DXGI VRR,
  with reference-safe catch-up after bursts. VRR requires an enabled compatible
  display/driver; DXGI support alone does not verify panel activation. Zero copy
  retains Windows-managed timing. WebRTC uses bounded recent queue history.

**Recommended VDD for Windows multi-display:**
[ParsecDisplay / parsec-vdd](https://github.com/nomi-san/parsec-vdd) is the
project's recommended virtual display manager because it uses Parsec's official
Virtual Display Driver. Use the driver already installed by Parsec on the host;
there is no need to reinstall the driver when setting up ParsecDisplay. Create
the required virtual displays on the host before opening them in EvoParseq.
Multi-display also works with other VDD solutions that expose compatible host
displays; using Parsec VDD is a recommendation, not a requirement.

Host permissions, encoders and graphics drivers determine the actual stream.
**Connection stats shows the received depth/chroma and application RGB output**;
a selected preference alone does not prove that the host accepted it.

The Session panel compares the client's configured display refresh with the host
FPS setting and shows measured decoded FPS separately. It can suggest a
Windows-listed refresh rate at the current client resolution for more even
fixed-refresh cadence; it never changes the display automatically. VRR activation
is not verified. A confirmed 120 FPS request does not prove 120 distinct frames:
the captured host display/mode must support it, and static content can lower FPS.

## Video rendering modes

![Animated comparison of normal GPU rendering, Zero copy and optional AI upscaling.](assets/evoparseq-rendering-modes.gif)

Start with **Normal GPU rendering** for compatibility. **Zero copy** skips a
separate picture-conversion pass on supported systems; **AI upscaling** can
improve enlarged video but uses more GPU resources and may add latency.
The animation illustrates processing order, not measured speed. Check
**Connection stats** for the mode actually in use.

## Windows download and run

1. Download `EvoParseq-v0.9.5-win64.zip` from the
   [stable release](https://github.com/vampywiz17/EvoParseq/releases/tag/v0.9.5).
   For this branch's Windows preview (including controllers), use artifacts from successful
   [dev CI runs](https://github.com/vampywiz17/EvoParseq/actions/workflows/native-core.yml?query=branch%3Adev).
2. Extract to a writable folder and run **EvoParseq.exe**. Settings and optional
   encrypted sessions are stored beside it in `settings.json`.
3. Sign in with the host owner's account, choose a computer and connect.
   Configure Network, Video, Audio and Input in Settings. Save and reconnect
   after changing transport or connection capabilities; session video requests
   can be applied while connected.

**Windows 11 x64 and a GPU with Direct3D feature level 11.0 or newer are required**,
with compatible D3D11 video-support drivers. The DirectX version shown on
dxdiag's System tab is the installed runtime version; check **Feature Levels**
on its Display tab for the GPU's capabilities. DirectX 10.x hardware, such as
the GeForce 210, is unsupported. Support below feature level 11.0 is not planned,
as it would require developing and maintaining a separate video path in parallel.

Zero-copy
requires Windows 11 and suitable drivers. Ordinary client use needs no admin
rights. The stock host must already be configured for hosting.

**Upgrading from ParsecWebTurn:** keep `settings.json` under the same Windows
account and sign in again. The old web-core login is not imported. A legacy-named
transition ZIP supports existing executable shortcuts. After the repository rename,
older updaters may reject the new download URL; use the release page for this
one-time manual upgrade. See [migration](MIGRATION.md).
Help > Check for updates offers newer **stable** EvoParseq packages; prerelease
installation is manual.

| Shortcut | Action |
| --- | --- |
| Ctrl+Shift+M / Ctrl+Shift+O | Open session menu and restore hidden icon |
| Ctrl+Shift+W | Toggle fullscreen |
| Ctrl+Shift+R | Release / recapture remote input |
| Ctrl+Shift+Q | Disconnect |
| Ctrl+Shift+S | Open Connection stats |
| Ctrl+Alt+End | Request host Ctrl+Alt+Del |

These exact shortcuts stay local; their trigger keys are not also forwarded to
the host. In Keyboard or Keyboard + Mouse immersive mode, other supported keys
and combinations (including Alt+Tab, Alt+Enter, Windows-key chords and F8/F11)
are forwarded while the stream has input focus. Extra Alt/Windows modifiers do
not activate a Ctrl+Shift client shortcut. Local menus and released input retain
normal Windows handling; Ctrl+Shift+R releases or recaptures remote input.

Ctrl+Alt+Del requires the installed Parsec host service and permissions; a
portable host may not execute it.

## TURN relay: connect through restrictive networks

![Animated TURN relay: video and audio travel from the host through Cloudflare to the client; input travels back.](assets/turn-relay.gif)

TURN forwards encrypted session traffic when NAT or a firewall prevents a direct
connection. The stock Parsec host stays unmodified. The client-to-relay leg can
use UDP, TLS or TCP; the relay-to-host leg uses UDP. Direct connections remain
available unless **Force TURN relay** is enabled.

**A relay does not necessarily mean higher latency.** When the route through
the relay is better than the available direct or VPN route, TURN can deliver
comparable or even lower round-trip time (RTT). This depends on ISP peering,
relay location and congestion; it is not a guaranteed improvement. Compare the
actual routes in **Connection stats**. Cloudflare describes its routing in
[TURN and anycast](https://blog.cloudflare.com/webrtc-turn-using-anycast/).
**No separate VPN, tunnel client, driver or relay helper needs to be installed
on the client PC:** TURN support is built into the application.

**Cloudflare Realtime TURN is our recommended relay.** A free Cloudflare account
is sufficient, including an account already used for Zero Trust Free. TURN is a
separate **Realtime** service: no paid Zero Trust plan or Cloudflare Tunnel is
required.

1. Sign in to the [Cloudflare dashboard](https://dash.cloudflare.com/), select
   your account, and open **Realtime → TURN Server → Create** (or search for
   **TURN Server** in the dashboard).
2. Give the TURN key a name and create it. Copy its **TURN Key ID / Token ID**
   and **API Token** from the resulting credentials panel.
3. In the client's **Settings → Network**, select **Cloudflare TURN**, enter
   the key ID and API token, then save and reconnect. The client generates
   temporary TURN credentials automatically. Enable **Force TURN relay** if you
   want to require the relay; **Connection stats** shows the route actually used.

Keep the API token private; do not attach it or `settings.json` to issues.
See Cloudflare's [setup guide](https://developers.cloudflare.com/realtime/turn/generate-credentials/)
and [dashboard example](https://github.com/cloudflare/speedtest/tree/main/example/turn-worker#creating-a-new-realtime-turn-app).

**Free allowance: 1 TB (1,000 GB) per month**, shared with Realtime SFU.
At a constant average relayed bitrate, this is approximately:

| Average relayed traffic | Hours/month | Hours/day over 30 days |
| --- | ---: | ---: |
| 10 Mbit/s | 222 | 7.4 |
| 20 Mbit/s | 111 | 3.7 |
| 30 Mbit/s | 74 | 2.5 |
| 50 Mbit/s | 44 | 1.5 |

**Video uses a variable bitrate:** with little or no screen movement, such as
mostly static desktop work, it can use much less bandwidth than during active
gameplay. Read the table using the **actual average relayed bitrate**, not the
configured bandwidth limit. Lower average usage makes the allowance last longer;
these figures are estimates, not fixed daily streaming limits.

Estimates use decimal GB and exclude audio/control/protocol overhead, which
reduces the available streaming time. Actual usage varies with the stream;
direct traffic does not consume TURN allowance. Cloudflare meters traffic sent
from its edge to the TURN client. Usage above the shared allowance is currently
**$0.05/GB**; check [current pricing](https://developers.cloudflare.com/realtime/sfu/platform/pricing/)
and the [TURN billing FAQ](https://developers.cloudflare.com/realtime/turn/faq/).

## Optional AI upscaling

The Video tab offers one **AI upscaling (experimental)** selector: NVIDIA RTX
VSR has Off/Low/Medium/High/Ultra; Intel VPL AI has Off/On. Other GPUs disable the
control. Each vendor's saved preference is retained.

Both paths upscale on the GPU in normal DXGI mode. They add GPU processing,
resources and an output copy, may increase latency, and are **not Zero copy**.
Unsupported configurations retain ordinary rendering. No CPU pixel readback
or generated frames are introduced.

- **NVIDIA:** RTX Video SDK 1.1.0; RTX 3060 playback is live-confirmed.
  The runtime is outside the EXE. On first launch with an NVIDIA GPU, setup asks
  permission before downloading the pinned official NVIDIA payload. Archive
  and DLL hashes are checked; verified caches are reused. The DLL lives under
  `%LOCALAPPDATA%/EvoParseq/vsr/1.1.0`, without admin rights. Declining leaves
  normal video available; enabling VSR later can authorize setup on next launch.
  SDK-enabled packages include NVIDIA's license; no CUDA toolkit is required.
- **Intel:** documented VPL 2.17 API with the installed Intel driver runtime,
  without a bundled/downloaded DLL or model. **Hardware operation is unverified.**
  Strict checks require the AI filter and same-device D3D11 surfaces; rejected
  runtime/configuration details remain in event logs. Currently limited to
  8-bit SDR upscaling; 10-bit, 4:4:4 and Zero copy keep their existing paths.

On Windows, Display timing (including VRR), Zero copy presentation and BUD USB HID
input no longer carry an experimental label. Hardware requirements and fallbacks
still apply. Linux feature labels and defaults are unchanged.

## Experimental features and limits

**Vendor HEVC, 10-bit HEVC, 4:4:4 and multichannel audio remain experimental.** For conservative operation, use H.264,
stereo Opus and Automatic timing with experimental options disabled.

- **Video:** live decoded frames remain on the GPU. Normal rendering uses the
  conversion/output pass; zero-copy presents original 8-bit NV12 or capability-checked 10-bit P010 surfaces.
  HEVC 10-bit 4:2:0 Zero copy is live-confirmed by the maintainer on tested AMD
  and NVIDIA systems. Intel HEVC, including 8/10-bit 4:4:4 normal rendering,
  has also been live-tested on the maintainer's UHD 770 setup; this does not
  establish support on every Intel GPU or driver.
  Driver/compositor copies and direct scanout are not independently verified.
- **10-bit:** RGB10 output is SDR, not HDR. NVIDIA host encoding is confirmed;
  other host encoders may refuse it or send 8-bit. An initial encoder failure
  asks you to disable 10-bit and reconnect. AMF Main10 output is live-validated on AMD Radeon 780M and is now the AMD default.
- **4:4:4:** 8/10-bit playback is live-validated on NVIDIA RTX 3060. Intel UHD 770 8/10-bit playback is also live-confirmed. Support remains
  capability-gated; a measured UHD 770 driver-query exception is logged.
  AMD selection is disabled.
  H.264 stays 4:2:0; 4:4:4 does not combine with original-surface zero-copy.
  Compatible host encoding, account permissions and service terms still apply.
- **HID:** requires BUD and the stock host's USB/Yubikey passthrough plus its
  supported USB driver. HID mouse uses the host primary display and disables
  display switching; forwarding pauses until that selection is reported.
  Use matching host/client keyboard layouts. No host patch is needed.
- **Audio:** raw PCM is BUD-only and requires host permission
  (`network_raw_audio=1`). WebRTC remains stereo Opus. Multichannel audio has
  synthetic coverage only; live surround playback/channel order is unconfirmed.
- **Displays and timing:** host virtual-display limitations can delay switching.
  Reported timings are CPU/host durations, not GPU completion, scanout or total
  latency. Adaptive catch-up, QoS benefit and physical audio-device recovery
  still require broader live testing; no measured latency gain is claimed.

Microphone/camera forwarding, HDR and multi-monitor HID mapping are not implemented. See the [changelog](CHANGELOG.md) for shipped changes.

## Troubleshooting and development

Enable **Event logging** in Network settings for `client-events.jsonl` beside
settings.json. Include the build, GPU/driver, transport and reproduction steps
with a log or screenshot when reporting issues. Logs omit credentials, media,
clipboard text and continuous input. **Do not publish settings.json or tokens.**
Saved credentials use Windows CurrentUser DPAPI; copying them to another machine
or Windows account does not transfer access. `--data-dir "C:\YourFolder"` selects
another writable data directory.

Build with Rust MSVC and Windows C++ Build Tools/SDK:

```powershell
cargo build --locked --release --manifest-path src-native/Cargo.toml
```

CI packages `src-native/target/x86_64-pc-windows-msvc/release/evoparseq.exe` as
EvoParseq.exe. Application logic lives in `src-native/src`; platform, transport
and video adapters reuse the existing bounded workers and GPU path.

[Architecture](docs/native-core.md) · [Settings comparison](docs/native-settings-comparison.md) ·
[Development references](docs/NATIVE_REFERENCES.md)

## Credits and independence

Thanks to **Duy (Nguyen Duy)** and [lowlat](https://github.com/nomi-san/lowlat)
for inspiration, feedback and selected MIT-licensed protocol, transport and GPU
components. Their licenses and local-change notices are retained. EvoParseq is
independently developed and maintained; it is not an official lowlat client.

Our code uses the [MIT license](LICENSE). Applicable Parsec service terms and
third-party licenses still apply. Development did not use Parsec's proprietary
source code, and EvoParseq does not patch the official host or client. Parsec and
Unity names and marks belong to their owners; no affiliation is implied.

## Star history

<a href="https://www.star-history.com/#vampywiz17/EvoParseq&amp;Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=vampywiz17/EvoParseq&amp;type=Date&amp;theme=dark">
    <img alt="EvoParseq GitHub star history" src="https://api.star-history.com/svg?repos=vampywiz17/EvoParseq&amp;type=Date" width="800">
  </picture>
</a>
