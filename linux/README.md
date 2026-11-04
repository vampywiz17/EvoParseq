# EvoParseq Linux Flatpak preview

Current preview: **0.1.13**, developed on `codex/linux-flatpak`. This is an
experimental Linux client for unmodified Parsec hosts, not a stable release.
Windows and Android TV feature availability differs; see the [root README](../README.md).

## Current support

| Area | Linux implementation and scope |
| --- | --- |
| Desktop | Wayland, egui with a Vulkan renderer; shared Windows application layout and native window decorations |
| Account | Sign-in, 2FA, computer discovery, manual peer ID and encrypted remembered sessions |
| Transport | WebRTC and explicitly selected BUD; no silent transport fallback |
| Video | Vulkan Video; progressive H.264, 8-bit 4:2:0, without frame reordering |
| Audio | Stereo Opus through PipeWire, with local mute and volume controls |
| Input | Normal Parsec mouse/keyboard and host-requested relative pointer capture |
| Session controls | Host display selection, resolution, bandwidth, FPS, frame pacing, fullscreen and Disconnect |
| Diagnostics | Live overlay, native Connection stats window, shared Rust statistics and host-timing models |
| Distribution | Flatpak, Freedesktop Platform/SDK 25.08 and matching graphics driver extensions |

HEVC playback, 10-bit/4:4:4, application Zero copy mode, AI upscaling, controllers,
HID, clipboard synchronization, rotated video and explicit immersive-mode
selection are **not integrated on Linux**. TURN provider/credential/Force TURN
controls are also not integrated into the Linux Settings UI. Existing saved
fields for these features do not enable them. Unsupported controls in the
shared Settings/session layout remain disabled.

## Install and run

Download the `EvoParseq-Linux-Flatpak` artifact from a successful
[Linux CI run](https://github.com/vampywiz17/EvoParseq/actions/workflows/linux-flatpak.yml?query=branch%3Acodex%2Flinux-flatpak),
then extract `EvoParseq-Linux.flatpak` from the artifact ZIP.

```sh
flatpak install --user --reinstall EvoParseq-Linux.flatpak
flatpak run io.github.vampywiz17.EvoParseq
```

Sign in with the host owner's account, enter a current 2FA code when needed,
select a computer and connect. Settings > Network selects WebRTC or BUD;
**Save settings** persists that choice for the next connection. The old
standalone connection/audio test buttons have been removed.

The host system must provide Wayland, PipeWire, the kernel and a compatible GPU.
The driver must expose Vulkan Video H.264 decode with sampleable NV12 output,
sufficient reference/DPB capacity and compatible same-device rendering. Vulkan
rendering support alone does not establish Vulkan Video decode support.
Flatpak packages the runtime and uses its matching graphics driver extensions;
it cannot supply missing GPU features or replace the host kernel driver.

Playback has been live-tested on the maintainer's HP EliteBook 840 G3 with
Intel HD Graphics 520. Compatibility still depends on the actual driver and
profile checks; this is not a guarantee for all Skylake systems. A software-only
VM can test sign-in and discovery but is not a Vulkan Video playback target.

The Flatpak **already defaults to `ANV_DEBUG=video-decode`** for the experimental
Mesa Intel video path. No extra launch option is needed. This is vendor-specific,
not a generic Vulkan requirement, and can be overridden with Flatpak's `--env`.

## Desktop and input

Windows is the canonical desktop UI. Shared Rust descriptors supply labels,
logical font sizes, dimensions, ordering and layout for the launcher, Settings,
session panel, menus and Connection stats. Native font rasterization may differ.
Login/computer picker and Settings use fixed content sizes; streaming restores
resizing, fits the first decoded frame within monitor bounds and keeps video
and mouse mapping centered on resize.

Compositor decorations are used when available; winit's Adwaita decorations are
the Wayland fallback. The package supplies host cursor-theme paths and bundled
Adwaita cursors for systems without a usable cursor theme/cursor-shape protocol.
Connection stats uses its own OS-decorated, resizable window; its events are not
forwarded to the host.

Click the video to activate normal remote mouse/keyboard input.
**Ctrl+Shift+M releases input.** Focus loss, disconnect and queue overload release
host keys/buttons. Geometry, key framing, button mapping, fractional relative
motion and the bounded input handoff reuse shared Rust implementations. Host
cursor flags can request Wayland relative pointer locking. Remote window-drag
endpoints are restored before unlocking to avoid leaving the cursor at the
initial drag position.

Physical key events preserve ISO/numpad keys and left/right modifiers. The
host keyboard layout and IME handle them; local IME/text payloads are not sent.
System/compositor shortcuts stay local. Clipboard shortcuts operate on the host
clipboard; they do not synchronize the local clipboard. No HID device or extra
Flatpak device permission is used for input.

## Overlay and Connection stats

The Windows-style translucent draggable session badge opens the shared panel.
Local panel clicks/drags release remote input and are not forwarded. The stream
menu contains Settings, Connection stats and Help; Settings restores a hidden
session badge. Unintegrated Help actions remain disabled.

The overlay binds host display selection/Refresh, resolution, bandwidth, FPS,
frame pacing and Apply. Shared validation preserves untouched host streams,
and confirmation checks subsequent host replies. Command failures are shown
without disconnecting an otherwise working stream. Fullscreen, Settings,
Disconnect, Close, Hide button and local PipeWire mute/volume are connected.
Ctrl+Alt+Del is available only for a host reporting Windows and still requires
host authorization/service support. Orientation and immersive-mode selectors
remain disabled.

Preview **0.1.13** fixes settings/display replies containing the stock host's
terminal NUL byte. It matches the Windows adapter and prevents valid replies
from leaving the controls at **Waiting for host**. Framed-response tests cover
BUD and WebRTC, with/without a terminator and malformed replies.

Connection stats uses the shared Sample/Rates/Presentation models. Its four
Overview cards are arranged **two per row**. Available measurements include
transport route/RTT/traffic, decoded FPS, format/profile, application output
precision, compressed queue occupancy, existing decoder/completion waits,
host timing reports and audio queue/error/drop counters. The reported display
refresh comes from cached compositor monitor information only when a single
connected output makes attribution unambiguous.

Diagnostic presentation is cached at most once per second while the overlay
or Connection stats is requested. Host timing collection stops when both are
hidden. **Process CPU and RSS are read only while Connection stats is open,
at most once per second**, using `getrusage` and `/proc/self/status`; no new
sampler thread, probe process, GPU query or render pass is added.

Unavailable measurements remain explicit: there is no GPU utilization sampler,
Windows-equivalent private working-set/commit counter, independently verified
scanout/hardware execution, or sampled PipeWire device/graph latency and
underrun counter. Scheduled Vulkan draws are not monitor scanout; CPU completion
waits are not GPU execution time. RSS is an estimate and CPU is normalized to
available logical CPUs. Audio queue latency excludes unknown device/graph delay.

## Video and audio paths

Account, protocol, session preferences, settings, input and diagnostics use the
existing Rust core. BUD uses the shared Windows/Linux/Android media ingress,
including video-first bounded batches, borrowed payload parsing, encoder
changes and keyframe gating. Linux adapters supply native output operations.
WebRTC retains the shared host-certificate compatibility policy, SDP fingerprint
and signature verification; it feeds the bounded Linux queue from the detached
reader. No Linux-specific cipher negotiation is introduced.

Original Vulkan Video NV12 decoder surfaces are sampled directly on the same
Vulkan device used by the UI. A presentation shader performs range expansion,
cropping, scaling and YUV-to-RGB conversion while writing the output target.
There is no application-side decoded-image copy, intermediate RGBA video
texture or CPU pixel readback. This is **GPU-resident rendering**, not the
project's strict **Zero copy mode**; driver/compositor copies and direct scanout
are not independently verified. See the [video-path audit](VIDEO_PATH.md).

The H.264 profile uses Vulkan's STREAMING usage hint. Ordered bounded decoding
preserves references and presents the latest completed picture. Conservative
completion waits synchronize Vulkan Video and wgpu. If the UI stops consuming
video, a full backlog suspends video rather than ending the connection/audio;
returning focus requests a fresh keyframe through the shared recovery gate.
Continuous background video decoding is not implemented. No end-to-end latency
guarantee is claimed.

PipeWire uses the shared Opus/audio buffering core. Local volume/mute commands
run on the existing PipeWire main loop through public stream controls, without
adding application PCM gain processing to the audio callback. A narrow helper
supplies the required C variadic terminator omitted by the pinned Rust wrapper.
Automatic reopening after audio-device failures is not yet integrated on Linux.

## Settings and protected sign-in

The shared Windows JSON schema/Manager is reused, including `nativeCore`,
unknown top-level fields and opaque protected values for unintegrated features.
Transport selection, remembered sessions and integrated session video preferences
are persisted. The file is `settings.json` in `$XDG_CONFIG_HOME/EvoParseq`
(normally `~/.config/EvoParseq` outside Flatpak). Flatpak places it in its private
app configuration tree. `--data-dir` selects a dedicated profile directory
accessible within the sandbox.

Writes use an atomic same-directory rename, private 0600 files and a private
0700 profile directory. Invalid JSON is preserved. Remember sign-in stores the
email and encrypted session token, **never the password or 2FA code**. The XDG
Secret portal obtains a per-application secret from the desktop keyring;
HKDF-SHA256 derives a key and AES-256-GCM protects each blob with a fresh nonce
and application-specific authentication domain. There is no plaintext fallback.

A missing/locked/unavailable portal leaves the new sign-in in memory and reports
that it could not be saved. Portal I/O runs on the existing account worker.
Startup validates a restored session through discovery; explicit sign-out
removes it and revokes the session. The JSON schema matches Windows, but Windows
DPAPI and Linux protected credentials are bound to their platform/user and are
not portable credentials.

## Diagnostics and development

```sh
flatpak run io.github.vampywiz17.EvoParseq --probe-video
```

The probe reports driver capabilities and sampleable output formats. A positive
report permits a playback attempt; it does not establish sustained throughput
or independently verified hardware execution. A reported HEVC capability does
not enable HEVC playback in this H.264-only backend.

The [CI workflow](../.github/workflows/linux-flatpak.yml) vendors locked sources,
installs Freedesktop 25.08 plus Rust/LLVM SDK extensions, runs the regression
suites and builds the offline Flatpak using the [manifest](flatpak/io.github.vampywiz17.EvoParseq.yml).
The 0.1.13 build passed 143 tests across its Linux/shared regression suites.
Hardware, compositor, host permissions and sustained playback still need live
validation. Report the preview version, GPU/driver, transport and reproduction
steps; do not publish `settings.json`, passwords or tokens.

References: [Secret portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Secret.html),
[Linux process memory](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html),
[PipeWire stream controls](https://docs.pipewire.org/group__pw__stream.html),
[Flatpak desktop integration](https://docs.flatpak.org/en/latest/desktop-integration.html#icons).
Bundled Adwaita artwork is by the GNOME Project, under its upstream dual
LGPLv3 / CC-BY-SA 3.0 license; license copies ship in the Flatpak.
