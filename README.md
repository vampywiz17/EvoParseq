> [!NOTE]
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

# EvoParseq

EvoParseq is an independent remote desktop client for **Parsec and Phaze** on
Windows, Linux Flatpak, and Android TV. A shared Rust core handles accounts,
networking, and session logic. Each platform uses its native video, audio,
and input APIs. Both services work with their stock hosts.

The stable [**EvoParseq v0.9.8**](https://github.com/vampywiz17/EvoParseq/releases/tag/v0.9.8)
release includes Windows x64, the signed Android TV APK and an experimental
Linux x86_64 Flatpak. Development continues on `dev`.

## Code quality

Live SonarQube results for `main`. Select a badge to open the dashboard.

| Metric | `main` |
| --- | --- |
| Quality Gate | [![main: Quality Gate](https://sonarcloud.io/api/project_badges/measure?project=vampywiz17_EvoParseq&branch=main&metric=alert_status)](https://sonarcloud.io/dashboard?id=vampywiz17_EvoParseq&branch=main) |
| Coverage (overall code) | [![main: Coverage (overall code)](https://sonarcloud.io/api/project_badges/measure?project=vampywiz17_EvoParseq&branch=main&metric=coverage)](https://sonarcloud.io/dashboard?id=vampywiz17_EvoParseq&branch=main) |
| Security (overall code) | [![main: Security (overall code)](https://sonarcloud.io/api/project_badges/measure?project=vampywiz17_EvoParseq&branch=main&metric=security_rating)](https://sonarcloud.io/dashboard?id=vampywiz17_EvoParseq&branch=main) |
| Reliability (overall code) | [![main: Reliability (overall code)](https://sonarcloud.io/api/project_badges/measure?project=vampywiz17_EvoParseq&branch=main&metric=reliability_rating)](https://sonarcloud.io/dashboard?id=vampywiz17_EvoParseq&branch=main) |
| Maintainability (overall code) | [![main: Maintainability (overall code)](https://sonarcloud.io/api/project_badges/measure?project=vampywiz17_EvoParseq&branch=main&metric=sqale_rating)](https://sonarcloud.io/dashboard?id=vampywiz17_EvoParseq&branch=main) |
| Duplicated lines (overall code) | [![main: Duplicated lines (overall code)](https://sonarcloud.io/api/project_badges/measure?project=vampywiz17_EvoParseq&branch=main&metric=duplicated_lines_density)](https://sonarcloud.io/dashboard?id=vampywiz17_EvoParseq&branch=main) |

The metric badges describe **overall code**, not just newly changed lines.
The Quality Gate applies the configured **new-code** requirements; a passing
gate does not mean that overall coverage is 100% or that all existing findings
have been resolved. The dashboard shows both scopes. GitHub may briefly cache
badge images after a new analysis.

Sonar also analyzes checked-in dependencies, including upstream test fixtures
and configuration files. Their findings count toward the overall results,
especially the Security rating, so the badges are not a score of EvoParseq's
own implementation alone and can give a misleading impression without that
context. Dependency findings still require review; upstream origin does not
automatically make a finding harmless or a false positive.

## Phaze preview

Development builds include experimental Phaze support. Select **Phaze**
on the sign-in screen, sign in with your Phaze account, and choose a host with
an active physical or virtual display.

| Platform | Phaze playback | Client controls |
| --- | --- | --- |
| Windows | H.264 and HEVC through the existing GPU pipeline. HEVC 4:4:4 can be requested on compatible hardware. | Session overlay, output switching, bitrate and FPS requests, mouse and keyboard, and text clipboard. |
| Linux Flatpak | H.264 and HEVC 8-bit 4:2:0 through Vulkan Video, with PipeWire audio. | Settings follow the Windows layout. The overlay provides output switching, bitrate and FPS requests, and local playback controls. Native Wayland handles mouse, keyboard, and text clipboard. |
| Android TV | H.264 and HEVC 8-bit 4:2:0 through MediaCodec, with AAudio playback. | The existing TV interface supports D-pad navigation, a remote-controlled pointer, and physical mouse and keyboard. |

Phaze uses **AAC-LC stereo** by default. Select **uncompressed PCM** in the
audio settings if you prefer it, then reconnect. Windows and Linux can switch
among the host's listed outputs. A Phaze session streams one output at a time.

**Connection stats** shows the received codec and measured traffic, with a
local RTT estimate when fresh acknowledgment samples are available. The
desktop video stats distinguish bitrate and FPS requests from measured playback.

Phaze USB passthrough, gamepad forwarding, host resolution changes, and
simultaneous multi-display are pending. Phaze has no 10-bit host request.
Linux and Android TV Phaze playback uses 4:2:0.

Windows has been tested against a stock Phaze host with H.264, audio, mouse
input, and Cloudflare TURN over UDP. The Linux Flatpak and signed Android TV
APK passed GitHub CI. Live Phaze streaming on those two platforms still needs
device testing. [Phaze protocol and validation notes](docs/phaze-transport.md)

## Parsec

Parsec supports **WebRTC** and explicitly selected **BUD**. Use the host
owner's account to sign in and choose an online computer. The platform,
host encoder, and selected transport determine the available features.

| Area | Windows | Linux Flatpak | Android TV |
| --- | --- | --- | --- |
| Video | GPU selection, H.264 and HEVC, compatible 10-bit SDR and 4:4:4, Zero copy and optional AI upscaling | Vulkan Video with GPU selection, H.264 and HEVC 8/10-bit 4:2:0; GPU changes take effect after restart | MediaCodec Surface playback with H.264 and HEVC 8/10-bit 4:2:0 on compatible devices |
| Audio | Stereo Opus, configurable buffering, BUD PCM and experimental surround | PipeWire, stereo Opus, BUD PCM, and capability-gated surround | AAudio, stereo Opus, BUD PCM, and capability-gated surround |
| Input | Mouse, keyboard, text clipboard, gamepads, and optional BUD USB HID | Native Wayland mouse and keyboard, text clipboard, gamepads, and optional BUD USB HID | D-pad navigation and remote pointer, physical mouse and keyboard, and gamepads |
| Displays | Up to three simultaneous outputs on WebRTC or BUD | Up to three simultaneous outputs on WebRTC or BUD | One active stream output |

Windows and Linux use one decoder per active display and share one audio
output. The primary overlay opens additional host-listed outputs. Each
window has its own resolution controls, and Connection stats stays on the
primary window. For Windows hosts, the recommended virtual display manager is
[ParsecDisplay](https://github.com/nomi-san/parsec-vdd). Create the outputs on
the host using the Parsec Virtual Display Driver already installed there.
Other compatible virtual displays also work.

The Linux Parsec preview has been tested on Intel HD 520 (H.264 and HEVC),
AMD Radeon 780M (H.264), and NVIDIA RTX 3060 on Nobara (H.264). The tested
hybrid Pop!_OS system reproduced its NVIDIA presentation failure in native
`vkcube`. Android TV Parsec playback, Main10, and gamepad input were tested
on a Xiaomi TV Box S (3rd Gen) running Android 14.
[Linux setup and diagnostics](linux/README.md) · [Android TV setup and capabilities](android-tv/README.md)

### Linux Flatpak for Parsec

- **Wayland + Vulkan:** egui desktop UI follows the Windows client's shared
  wording, logical dimensions and layout, with native desktop window decorations.
  Login/computer picker and Settings have fixed sizes; the stream is resizable
  and uses centered aspect fitting. Connection stats is a separate fixed-size native window.
- **Shared Rust core:** account/2FA, discovery, session preferences, settings,
  WebRTC/BUD, media ingress, normal input and diagnostic models are reused.
  Linux adapters provide Vulkan Video/Vulkan, PipeWire, Wayland input and
  Secret portal credential protection.
- **Playback:** WebRTC and explicit BUD selection, progressive H.264 or capability-gated HEVC Main/Main10 4:2:0
  and stereo WebRTC Opus or BUD Opus/PCM with optional matched 5.1/7.1 output. Original NV12 and P010 decoder surfaces feed the same-device Vulkan
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

HEVC Main10/P010 decoding is now available after selected-device capability checks,
with SDR presentation. Main10 has been tested on the maintainer's Intel ARL system. Display timing VRR requests
adaptive presentation; physical VRR remains unverified.

### Android TV for Parsec

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

Install the signed **EvoParseq-TV.apk** from the
[stable release](https://github.com/vampywiz17/EvoParseq/releases/tag/v0.9.8)
or use **Settings > Updates** on an existing distribution-signed installation.
See the [Android TV setup, build and compatibility guide](android-tv/README.md).

## Video rendering modes

![Animated comparison of normal GPU rendering, Zero copy and optional AI upscaling.](assets/evoparseq-rendering-modes.gif)

| Windows mode | How it presents video |
| --- | --- |
| Normal GPU rendering | Keeps decoded frames on the GPU and runs a color conversion pass. Start here for compatibility. |
| Zero copy | Presents supported original decoder surfaces directly, without a separate conversion or post-processing pass. |
| AI upscaling | Uses NVIDIA RTX VSR or Intel VPL on compatible hardware. It adds GPU work and an output copy and may increase latency. |

The animation shows the order of processing. Check **Connection stats** for
the active decoder, received chroma and bit depth, application RGB output, and
rendering mode. Windows VRR is a presentation option that requires a
compatible display and driver. Panel activation and physical scanout remain
unverified. Linux uses GPU-resident Vulkan conversion, and Android hands
MediaCodec output to a Surface.
[Linux video path](linux/VIDEO_PATH.md)

## Download and run

| Platform | Requirements and installation | Development builds |
| --- | --- | --- |
| Windows | Windows 11 x64, Direct3D feature level 11.0 or later, and a compatible video driver. Extract the ZIP to a writable folder and run **EvoParseq.exe**. | [Windows CI](https://github.com/vampywiz17/EvoParseq/actions/workflows/native-core.yml?query=branch%3Adev) |
| Linux | An x86_64 Wayland session, PipeWire, and a working Vulkan Video driver. Install the **.flatpak** bundle. | [Flatpak CI](https://github.com/vampywiz17/EvoParseq/actions/workflows/linux-flatpak.yml?query=branch%3Adev) |
| Android TV | ARM64 or ARMv7. Sideload the signed **EvoParseq-TV.apk**. | [Android TV CI](https://github.com/vampywiz17/EvoParseq/actions/workflows/android-tv.yml?query=branch%3Adev) |

Download the artifact from a successful branch run. Published packages are
listed on the [releases page](https://github.com/vampywiz17/EvoParseq/releases).
Choose the service at sign-in and reconnect after changing connection
capabilities. Remembered sessions use the platform's protected storage.
Users upgrading from ParsecWebTurn can refer to the [migration guide](MIGRATION.md).

### Windows shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+Shift+M / Ctrl+Shift+O | Open the session menu and restore the hidden icon |
| Ctrl+Shift+W | Toggle fullscreen |
| Ctrl+Shift+R | Release or recapture remote input |
| Ctrl+Shift+Q | Disconnect |
| Ctrl+Shift+S | Open Connection stats |
| Ctrl+Alt+End | Request host Ctrl+Alt+Del where supported |

## TURN relay

![Animated TURN relay showing media from the host through Cloudflare to the client and input traveling back.](assets/turn-relay.gif)

TURN carries encrypted session traffic when NAT or a firewall makes a direct
route difficult. The client-to-relay leg can use UDP, TCP or TLS. The
relay-to-host leg uses UDP. TURN is built into EvoParseq. Its effect on
latency depends on ISP peering, relay location, and congestion. A better
relay route can match or improve on the direct or VPN RTT. Compare the
active routes in **Connection stats**.
[Cloudflare on TURN routing](https://developers.cloudflare.com/realtime/turn/faq/#does-cloudflare-realtime-turn-use-the-cloudflare-backbone-or-is-there-any-magic-cloudflare-do-to-speed-connection-up)

| Service | Routing behavior |
| --- | --- |
| Parsec | WebRTC or BUD can use the configured TURN service. Enable **Force TURN relay** to require a relay. |
| Phaze | Direct UDP by default. Select **Force TURN relay** to use TURN. A failed forced relay ends the connection attempt. |

### Set up Cloudflare Realtime TURN

1. In the [Cloudflare dashboard](https://dash.cloudflare.com/), open **Realtime → TURN Server** and create a TURN key.
2. In **Settings → Network → Cloudflare TURN**, enter the TURN key ID and API token from your own account. EvoParseq uses them to generate temporary credentials. Keep the API token private.
3. Save and reconnect. Select **Force TURN relay** if the session should require TURN, then check the resulting route in **Connection stats**.

The desktop settings also accept a custom TURN URL and credentials. Android TV
offers local HTTPS provisioning with fingerprint verification and TV approval.
[Cloudflare credentials guide](https://developers.cloudflare.com/realtime/turn/generate-credentials/) ·
[Android TV relay setup](android-tv/README.md#local-relay-setup)

### Free allowance and streaming estimates

Cloudflare Realtime includes **1,000 GB (1 TB) per month**, shared by TURN and
SFU. Additional egress costs **$0.05/GB**. TURN billing counts traffic sent
from Cloudflare to the TURN client, including TURN overhead.
[Cloudflare pricing](https://developers.cloudflare.com/realtime/sfu/platform/pricing/) ·
[TURN billing FAQ](https://developers.cloudflare.com/realtime/turn/faq/#what-is-cloudflare-realtime-turn-pricing-how-exactly-is-it-calculated)

| Average relayed traffic | Hours per month | Hours per day over 30 days |
| --- | ---: | ---: |
| 10 Mbit/s | 222 | 7.4 |
| 20 Mbit/s | 111 | 3.7 |
| 30 Mbit/s | 74 | 2.5 |
| 50 Mbit/s | 44 | 1.5 |

Estimates use decimal GB and assume the full monthly allowance is available.
When using video bitrate to estimate usage, allow room for audio and protocol
overhead. Use the **measured average** rather than the configured bitrate
limit. A mostly static desktop can use much less bandwidth than gameplay.
Direct sessions use no TURN allowance.

## Troubleshooting and development

For a failed connection, include the platform, build, service, route, and
steps to reproduce. For video problems, add the GPU, driver, and codec shown
in **Connection stats**. If Phaze connects but shows no picture, check that
the host has an active physical or virtual display.

Enable **Event logging** where available and include the relevant part of
`client-events.jsonl`. Keep credentials, tokens and `settings.json` private.

Build the Windows client with Rust MSVC and the Windows C++ Build Tools/SDK.

```powershell
cargo build --locked --release --manifest-path src-native/Cargo.toml
```

[Architecture](docs/native-core.md) · [Development references](docs/NATIVE_REFERENCES.md) ·
[Changelog](CHANGELOG.md) · [License](LICENSE)

EvoParseq is a community project independent of Parsec, Unity, Phaze, and
lowlat. The Phaze adapter is an original open-source Rust implementation.
Thanks to **Duy (Nguyen Duy)** and [lowlat](https://github.com/nomi-san/lowlat)
for feedback and MIT-licensed components retained with their license notices.
