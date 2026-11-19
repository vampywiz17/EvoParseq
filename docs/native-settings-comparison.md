# Stock client settings compared with native-core

Scope: the supplied stock-client screenshots and `codex/native-core`, 2026-10-24.
This comparison does not add the other listed features to the application.

| Stock setting | Native client | Further work |
| --- | --- | --- |
| Overlay / Hide Button | Draggable icon, translucent Session panel, persisted hide setting and Ctrl+Shift+M restore | Existing |
| Overlay warnings | Event log and detailed Connection stats; no compact warning banner | Can reuse existing telemetry, with rate limits |
| Window mode / fullscreen | Session preference and shortcut | Existing |
| Renderer | GPU video uses D3D11; native controls use Win32/GDI | Other graphics APIs require a separate renderer, not another setting |
| VSync | Automatic, Low latency and VSync display timing | Existing; original-surface presentation is managed separately by Windows |
| Decoder selection | Automatic AMF/VPL/direct D3D11/MF selection and recovery | Manual forcing/software decode are separate features; no such selector currently |
| H.265 | GPU capability check and H.264 host fallback | Existing |
| 10-bit color | Main 10 request, SPS verification, P010 and RGB10 SDR output | Host encoder compatibility still matters; unsupported hosts can disconnect |
| Prefer 4:4:4 | Experimental 8/10-bit HEVC (AYUV/Y410) direct GPU path on compatible NVIDIA/Intel clients; AMD disabled | 8-bit and combined 10-bit/RGB10 confirmed live over BUD on RTX 3060; Intel not live-validated |
| Audio codec | Opus; optional raw PCM over BUD | Existing; raw PCM is not offered on the restored WebRTC path |
| Audio buffer | Optional bounded custom minimum/maximum | Existing |
| Immersive mode | Mouse, keyboard or both | Existing |
| HID compatibility | Physical USB HID mouse/keyboard over BUD | Different from generic USB-device passthrough |
| Yubikey passthrough / selection | No local physical Yubikey enumeration/forwarding | Requires device enumeration, permissions and a complete USB exchange; no libusb dependency added |
| Microphone passthrough / selection | Not implemented | Requires capture, negotiated USB/audio device and bounded host delivery; existing HID transport alone is insufficient |
| Camera passthrough / selection (stock Warp) | Not implemented | Requires capture, encoding and host-device integration |
| Enhanced Pen (stock Warp) | Not implemented | Requires pressure/tilt events and the host pen contract |
| Swap Command/Ctrl for macOS | Not implemented; current executable targets Windows | Possible as an input mapping after host-platform identification |
| New UI preview | Not applicable: independent native UI | Independent native UI |
| Chat | No native chat panel | Application messages and a bounded UI would be needed |
| Sound ON/OFF | No native Session mute toggle | Can be added at the existing playback queue, without stopping video |
| Metrics graphs | Numeric live card and Connection stats only | Bounded history can reuse the same samples |
| Ctrl+Alt+Del | Host-authorized application request | Host Parsec service is needed; portable hosts may not execute it |
| Resolution, bandwidth, FPS, constant FPS | Native host settings query/update/confirmation | Existing; keep host resolution is supported |

The restored WebRTC route reuses this project's existing Rust ICE/DTLS/SCTP
implementation, TURN policy and GPU decoder workers. No lowlat WebRTC engine,
browser or second client runtime is introduced. BUD-only audio/HID preferences
stay inactive when WebRTC is selected. Live transport/driver verification is
separate from the synthetic regression suite.
