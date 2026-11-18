> [!NOTE]
> EvoParseq is human-led and developed with substantial AI assistance. The
> maintainer sets the architecture and priorities, reviews changes, and tests
> builds on real hardware.

<p align="center">
  <img src="assets/evoparseq-wordmark.svg" width="360" alt="EvoParseq">
</p>

# EvoParseq

EvoParseq is an independent remote desktop client for **Parsec and Phaze** on
Windows, Linux Flatpak, and Android TV. A shared Rust core handles accounts,
networking, and session logic. Each platform uses its native video, audio,
and input APIs. Both services work with their stock hosts.

## Phaze preview

This branch adds experimental Phaze support alongside Parsec. Select **Phaze**
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

Parsec remains available through **WebRTC** or explicit **BUD** selection.
Capabilities vary by platform and transport.

- **Windows** supports GPU selection, compatible HEVC 10-bit SDR and 4:4:4,
  Zero copy on supported paths, optional GPU upscaling, multi-display, and BUD
  USB HID. Zero copy presents original decoder surfaces directly.
- **Linux Flatpak** uses native Wayland, Vulkan Video, and PipeWire. It supports
  GPU selection and multi-display on WebRTC or BUD. HEVC support depends on
  the GPU and driver. [Linux setup and video paths](linux/README.md)
- **Android TV** supports MediaCodec playback, D-pad navigation, remote mouse,
  physical input, and gamepads. [Android TV setup and capabilities](android-tv/README.md)

For Parsec multi-display on Windows, the recommended virtual display manager
is [ParsecDisplay](https://github.com/nomi-san/parsec-vdd). Create the outputs
on the host using the Parsec Virtual Display Driver already installed there.
Other compatible virtual displays also work.

## Download and run

| Platform | Phaze branch builds |
| --- | --- |
| Windows 11 x64 | [Windows CI](https://github.com/vampywiz17/EvoParseq/actions/workflows/native-core.yml?query=branch%3Acodex%2Fphaze-protocol) |
| Linux x86_64 | [Flatpak CI](https://github.com/vampywiz17/EvoParseq/actions/workflows/linux-flatpak.yml?query=branch%3Acodex%2Fphaze-protocol) |
| Android TV ARM64 and ARMv7 | [Android TV CI](https://github.com/vampywiz17/EvoParseq/actions/workflows/android-tv.yml?query=branch%3Acodex%2Fphaze-protocol) |

Open a successful run and download its artifact. Extract the Windows ZIP
and run **EvoParseq.exe**, install the Linux **.flatpak** bundle, or install
the signed Android TV **.apk**. Linux requires a Wayland session and a
compatible Vulkan Video driver. Published packages are listed on the
[releases page](https://github.com/vampywiz17/EvoParseq/releases).

## TURN relay

For Phaze, the default route is direct UDP. To require a relay, configure a
custom TURN server or Cloudflare TURN in **Settings → Network**, select
**Force TURN relay**, and reconnect. The client-to-relay leg can use UDP, TCP
or TLS, and the relay-to-host leg uses UDP. A failed forced relay ends the
connection attempt. Check **Connection stats** for the selected route.
[Cloudflare TURN credentials guide](https://developers.cloudflare.com/realtime/turn/generate-credentials/) ·
[Android TV relay setup](android-tv/README.md)

## Troubleshooting and development

For a failed connection, include the platform, build, service, route, and
steps to reproduce. Enable **Event logging** where available and include
the relevant part of `client-events.jsonl`. Keep tokens and `settings.json`
private.

[Architecture](docs/native-core.md) ·
[Changelog](CHANGELOG.md) · [License](LICENSE)

EvoParseq is a community project independent of Parsec, Unity, Phaze, and
lowlat. The Phaze adapter is an original open-source Rust implementation.
Thanks to **Duy (Nguyen Duy)** and [lowlat](https://github.com/nomi-san/lowlat)
for feedback and MIT-licensed components retained with their license notices.
