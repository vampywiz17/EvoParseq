# EvoParseq Linux Flatpak preview

Current preview: **0.1.23**, developed on `dev`. This is an
experimental Linux client for unmodified Parsec hosts, not a stable release.
Windows and Android TV feature availability differs; see the [root README](../README.md).

## Current support

| Area | Linux implementation and scope |
| --- | --- |
| Desktop | Wayland, egui with a Vulkan renderer; shared Windows application layout and native window decorations |
| Account | Sign-in, 2FA, computer discovery, manual peer ID and encrypted remembered sessions |
| Transport | WebRTC and explicitly selected BUD; direct/STUN, custom TURN UDP/TLS/TCP, Cloudflare TURN, direct-only and Force TURN; no silent transport fallback |
| Video | Vulkan Video; progressive H.264 or HEVC Main, 8-bit 4:2:0, without frame reordering |
| Audio | Stereo WebRTC Opus; BUD Opus/raw PCM, stereo or verified default-output 5.1/7.1; PipeWire mute/volume and automatic output reopening |
| Input | Normal Parsec or opt-in BUD USB HID mouse/keyboard; shared controller lifecycle and rumble; focused text clipboard |
| Session controls | Host display selection, resolution, bandwidth, FPS, frame pacing, orientation, immersive mouse/keyboard, fullscreen and Disconnect |
| Diagnostics | Live overlay, native Connection stats window, shared Rust statistics and host-timing models |
| Distribution | Flatpak, Freedesktop Platform/SDK 25.08 and matching graphics driver extensions |

10-bit/4:4:4 playback, application Zero copy mode, AI upscaling, runtime GPU
switching, verified VRR control and simultaneous multi-display windows are
**not integrated on Linux**. Unsupported controls remain disabled. Selecting a
single host display is supported; HID mouse pins the verified primary output.

## Install and run

Download the `EvoParseq-Linux-Flatpak` artifact from a successful
[Linux CI run](https://github.com/vampywiz17/EvoParseq/actions/workflows/linux-flatpak.yml?query=branch%3Adev),
then extract `EvoParseq-Linux-v0.1.23-x86_64.flatpak` from the artifact ZIP.

```sh
flatpak install --user --reinstall EvoParseq-Linux-v0.1.23-x86_64.flatpak
flatpak run io.github.vampywiz17.EvoParseq
```

Sign in with the host owner's account, enter a current 2FA code when needed,
select a computer and connect. Settings > Network selects WebRTC or BUD;
**Save settings** persists that choice for the next connection. The old
standalone connection/audio test buttons have been removed.

The host system must provide Wayland, PipeWire, the kernel and a compatible GPU.
The default H.264 path requires Vulkan Video H.264 decode with sampleable NV12 output,
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
Connection stats uses its own OS-decorated, fixed-size window; its events are not
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
System/compositor shortcuts stay local. Focused text clipboard synchronization and opt-in BUD HID reuse the shared Rust
policy. Controller access uses Flatpak's scoped input permission; no broad device
access is requested.

## Overlay and Connection stats

The Windows-style translucent draggable session badge opens the shared panel.
Local panel clicks/drags release remote input and are not forwarded. The stream
menu contains Settings, Connection stats and Help; Settings restores a hidden
session badge. Help opens native About and Updates windows.

The overlay binds host display selection/Refresh, resolution, bandwidth, FPS,
frame pacing and Apply. Shared validation preserves untouched host streams,
and confirmation checks subsequent host replies. Command failures are shown
without disconnecting an otherwise working stream. Fullscreen, Settings,
Disconnect, Close, Hide button and local PipeWire mute/volume are connected.
Ctrl+Alt+Del is available only for a host reporting Windows and still requires
host authorization/service support. Orientation and supported immersive mouse/
keyboard modes are connected; keyboard shortcut inhibition requires compositor
support.

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

Original H.264/HEVC Vulkan Video NV12 decoder surfaces are sampled directly on the same
Vulkan device used by the UI. A presentation shader performs range expansion,
cropping, scaling and YUV-to-RGB conversion while writing the output target.
There is no application-side decoded-image copy, intermediate RGBA video
texture or CPU pixel readback. This is **GPU-resident rendering**, not the
project's strict **Zero copy mode**; driver/compositor copies and direct scanout
are not independently verified. See the [video-path audit](VIDEO_PATH.md).

Both decode profiles use Vulkan's STREAMING usage hint. Ordered bounded decoding
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
Output and daemon failures reopen the stream on the existing worker using the
shared bounded recovery policy, preserving mute/volume.

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
or independently verified hardware execution. HEVC is advertised only after checking Main/8-bit capabilities, the selected
decode queue and a supported NV12 reference/output layout. Session
creation additionally validates the received dimensions, level and DPB needs.

The [CI workflow](../.github/workflows/linux-flatpak.yml) vendors locked sources,
installs Freedesktop 25.08 plus Rust/LLVM SDK extensions, runs the regression
suites and builds the offline Flatpak using the [manifest](flatpak/io.github.vampywiz17.EvoParseq.yml).
The 0.1.14 regression suites include HEVC parameter/reference, unsupported-format
and queue-codec tests alongside the existing H.264, transport and input checks.
Hardware, compositor, host permissions and sustained playback still need live
validation. Report the preview version, GPU/driver, transport and reproduction
steps; do not publish `settings.json`, passwords or tokens.

References: [Secret portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Secret.html),
[Linux process memory](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html),
[PipeWire stream controls](https://docs.pipewire.org/group__pw__stream.html),
[Flatpak desktop integration](https://docs.flatpak.org/en/latest/desktop-integration.html#icons).
Bundled Adwaita artwork is by the GNOME Project, under its upstream dual
LGPLv3 / CC-BY-SA 3.0 license; license copies ship in the Flatpak.


## HEVC Main 8-bit (0.1.14)

In Settings > Video > Video format, choose **H.265 / HEVC (8-bit)**, save and
reconnect. H.264 remains the default. The HEVC choice is enabled only when the
selected Vulkan device/queue reports Main 8-bit decode, sufficient reference
capacity and sampleable NV12 output with a supported DPB layout.
The host may still choose H.264; Connection stats reports the received codec.
Existing dormant 10-bit/4:4:4 preferences never enable those Linux formats.

WebRTC/BUD negotiation reuses the common control flags and inspector. Encoded
queue entries carry the already identified codec into the decoder without a
second per-picture framing scan. The pinned shared HEVC parser owns picture
parsing, reference lists and DPB lifecycle. The Vulkan adapter only translates
these into H.265 standard structures and reads missing VPS/PTL/ordering and PPS
presence metadata on parameter-set arrival. H.264/HEVC share native resource
allocation, bounded completion waits, command recording and presentation.
There is no new decoded-image copy, intermediate image, render pass, worker or
software-decoding fallback.

This step supports progressive Main, 8-bit 4:2:0 with zero declared reordering.
Main10, 4:4:4, range extensions, multilayer coding and VPS HRD parameters are
explicitly rejected. Optional non-decoding SPS VUI metadata is not supplied to
the Vulkan session; the shared parser's range/crop data still drives presentation.
Offline fixtures include synthetic streaming and NVIDIA low-latency sequences;
H.264 and HEVC 8-bit playback have been reported working on the maintainer's
HD 520 laptop. Other devices and sustained performance still require live
validation.

The Vulkan contracts are documented by Khronos under
[H.265 decode](https://docs.vulkan.org/features/latest/features/proposals/VK_KHR_video_decode_h265.html)
and [video coding](https://docs.vulkan.org/spec/latest/chapters/videocoding.html).

## Network settings (0.1.15)

Settings → Network uses the Windows form and shared Rust validation, URL parsing,
credential generation/cache and route policy for both WebRTC and BUD. Add one
STUN or TURN URL per line, or select Cloudflare TURN and supply the key ID/token.
Blank password/token edits keep the saved secret; Forget removes it. Credentials
are protected with the Linux Secret portal; saving can prompt your desktop
keyring and never blocks video rendering. Temporary Cloudflare credentials are
cached in memory only when enabled. Changes apply to the next connection.

Automatic BUD routing first tries direct UDP, then the configured TURN carriers
in the Windows order (UDP, TLS, TCP). Each retry has fresh session credentials
and a fresh signaling attempt ID. Direct-only never provisions TURN; Force TURN
never falls back to direct. Actual route reporting still uses transport evidence.
The existing TCP/TLS QoS switch enables Linux outgoing socket marking for
WebRTC and BUD, on direct UDP and TURN UDP/TCP/TLS: DSCP AF21 (18) and
SO_PRIORITY 6. Off by default; reconnect after saving. Each option is optional:
refusal does not stop or reroute the connection. No administrator rights,
capabilities, traffic shaping, per-packet work or additional threads are added.
IPv4/IPv6 DSCP and local priority are read back independently in the
`linux_socket_qos` event. Readback confirms local socket configuration only;
network devices and the local queue discipline may ignore it. Incoming host
video is not prioritized by client-side marking. AF21 follows RFC 8837's high
priority data class, appropriate to outgoing input/control and feedback traffic.
Existing settings files retain the `experimental_tcp_qos` key. Windows qWAVE
and Android behavior are unchanged. Event logging can be enabled here.
References: [RFC 8837](https://www.rfc-editor.org/rfc/rfc8837.html),
[Linux socket options](https://man7.org/linux/man-pages/man7/socket.7.html).
No new media worker, image copy, readback or render pass is introduced.

The offline network fixtures cover protected-secret persistence, failed edits,
WebRTC relay policy, BUD carrier ordering and direct-only credential isolation.
Live relay interoperability still needs validation with your TURN provider and
network; successful offline fixtures do not establish that every firewall route
is reachable.

## Audio, input and presentation integration (0.1.16)

- Settings → Audio reuses Windows' shared fields, validation and JSON schema.
  WebRTC remains stereo Opus. BUD may advertise PCM only when explicitly enabled;
  the host must also enable RAW (`network_raw_audio=1`). PCM is not compression
  and uses more bandwidth. Actual received codec/channels remain visible in stats.
- Match output layout probes the default PipeWire sink once before BUD negotiation.
  Only explicit supported 5.1/7.1 positions are advertised. Unknown/unsupported
  layouts use stereo. Channel order is declared explicitly to PipeWire; a stereo
  host fallback fills FL/FR and silence in the other stream channels.
- The existing lock-free callback supports multichannel PCM without allocation or
  locking in its render path. Minimum buffering governs startup/refill; maximum
  bounds the decoded PCM queue. PipeWire device/graph latency is additional and
  is not falsely included in that budget or claimed as measured.
- Output/daemon disconnections reopen on the existing PipeWire worker, using the
  common Windows recovery backoff. Old queued PCM is retired. Mute/volume are
  restored after recovery; the video transport is not restarted for audio loss.
- BUD USB HID reuses native framing, normalization and the shared host-primary
  verification gate. The host needs the matching Parsec USB driver. Unavailable
  drivers/unknown primary output do not silently fall back to normal mouse input.
  Releases remain allowed; host display switching is disabled with HID mouse.
- Controllers use the runtime's SDL2 controller API (SDL2-compat/SDL3 on 25.08)
  only for devices/motors. Activation, dead zones, focus release, hotplug and wire
  reports reuse the Windows Rust core; no additional polling worker is created.
  Press a face button to join. Device support and rumble depend on the controller
  driver. Flatpak **1.15.6+** is required for the scoped `input` device permission;
  no `--device=all`, root access or external controller helper is requested.
- Text clipboard uses the existing eframe Wayland clipboard adapter, synchronizing
  on stream activation and paste. Remote copies are accepted only in the active,
  focused stream. Common size/UTF-8 bounds, echo suppression and focus revocation
  apply. No background clipboard monitor or content logging is added.
- Orientation uses the existing presentation shader and original decoder surfaces;
  there is no intermediate image or additional render pass. Immersive mouse uses
  native confinement; immersive keyboard requests Wayland shortcut inhibition.
  Keyboard choices are disabled if that protocol is unavailable. The compositor
  retains authority over reserved shortcuts. Ctrl+Shift+M releases input.
- Settings → Video wires supported H.264/HEVC-8, session-button visibility and
  Automatic/Low latency/VSync preferences. Low latency requests one queued surface
  presentation; this is a driver/compositor hint, not a measured scanout guarantee.
  It applies on the next connection through a narrow Linux-only egui-wgpu patch
  using public `wgpu::Surface::configure`. Unsupported VRR/10-bit/4:4:4/AI/Zero copy
  choices remain disabled; the current adapter is displayed read-only.
- Event logging is initialized and guarded at Linux startup. The Network checkbox
  controls the shared bounded JSONL log beside settings.json, with rotation and
  shutdown flush. No clipboard text, credential, media payload or arbitrary host
  error body is logged.

Automated fixtures validate the common policy and adapter contracts. Live PCM,
multichannel, HID, clipboard, controller/rumble and compositor behavior still need
validation on the user's hardware; successful compilation is not hardware proof.

## Information and GitHub updates (0.1.17)

Help → About shares Windows' content, fonts and layout descriptors, with Linux
version/platform and installation-specific text. Help → Updates explicitly checks
GitHub releases (including clearly labelled Linux previews). Only an asset named
`EvoParseq-Linux-v<major.minor.patch>-x86_64.flatpak` is eligible. The Linux package
version is compared independently of the Windows/Android release tag. Older,
wrong-architecture, ambiguous or unverifiable packages are refused.

Downloads are bounded to the GitHub-reported size and SHA-256 digest, over HTTPS
with restricted GitHub redirects. Verified bundles stay in the app-private cache
and are passed as read-only file descriptors through XDG OpenURI/OpenFile to the
system software installer. The user reviews/confirms the installation there and
restarts EvoParseq afterwards. No host shell escape, broad D-Bus permission,
silent replacement or automatic restart is used. This verifies the published
GitHub digest, not an independent publisher signature. A desktop software manager
capable of handling `.flatpak` bundles and an OpenURI portal backend are required;
if unavailable, the UI reports the error and retains the verified cached bundle.

CI artifacts alone are not releases. Until a compatible versioned Flatpak is
attached to a GitHub release, Updates correctly reports no newer Linux package.
The workflow produces the versioned asset ready for that publication; it does
not publish a release automatically.

## AMD / NVIDIA test preparation

Selection and eligibility are capability-based, without a vendor allowlist.
Vulkan Video capabilities select separate or layered DPB storage and coinciding
or distinct decode output for both H.264 and HEVC Main 8-bit. Intel retains its
existing coinciding per-image path. NVIDIA's layered references share one image;
AMD's distinct output is written directly by the decode command and sampled by
the existing presentation pass. No application-side decoded-frame copy, CPU
readback, extra render pass, VA-API or software fallback is added. Distinct output
necessarily needs separate output allocations and decoder memory bandwidth.
The existing bounded parser slots, output lease and GPU synchronization remain.

Settings > Video > Video GPU lists Vulkan hardware adapters and saves the chosen
device in the existing settings schema. Restart EvoParseq after changing it:
decode and UI rendering intentionally share one Vulkan device. A missing or
ineligible explicit adapter produces an error instead of selecting another GPU.
Automatic retains capability-based startup selection. The probe reports the
selected resource layout; driver eligibility is not playback validation.

```sh
flatpak update
flatpak run io.github.vampywiz17.EvoParseq --probe-video
flatpak run io.github.vampywiz17.EvoParseq --video-adapter NVIDIA
flatpak run io.github.vampywiz17.EvoParseq --video-adapter 'AMD Radeon'
```

`--video-adapter` matches the case-insensitive adapter name, tries only matching
adapters at startup, and never silently substitutes another vendor for playback.
It overrides the saved Video GPU for that launch; it is not live GPU switching. Use the exact name from
the probe if a broad substring matches several devices.

AMD uses the runtime's compatible Vulkan driver (normally Mesa RADV); NVIDIA
requires a supported host driver and the Flatpak graphics extension matching
that host driver. Do not bundle or force a different proprietary driver version.
The Intel-specific `ANV_DEBUG` setting does not select a GPU or enable video on
AMD/NVIDIA. Test H.264 first, then HEVC Main 8-bit, over BUD and WebRTC; include
window resizing, fullscreen, focus return and reconnect. AMD/NVIDIA physical
playback has **not** been validated by this change.

See the [branch cleanup review](AUDIT.md) for retained tests/dependencies and
platform-boundary checks.

## GPU synchronization (0.1.18)

On timeline-capable Vulkan devices, decode and the existing presentation pass
exchange same-device timeline semaphore signals. Two bounded compressed-input /
command-buffer slots are retired by nonblocking fence/status queries; the UI
leaves compressed references in the existing ingress queue while both are busy.
GPU decode completion (not command submission) advances the native decoded count.
Images, DPB mapping and the NV12 presentation shader are unchanged. There is no
additional decoded-image copy, readback, render pass, worker or periodic timer.
Graphics-to-decode synchronization conservatively covers the preceding video
presentation, including repeated redraws. It does not yet overlap a decode which
reads reference surfaces with a render of those same surfaces in another layout.

The Linux-only wgpu-hal 27.0.4 patch adds external semaphore waits; the Linux-only
egui-wgpu submit hook attaches waits/signals immediately around the actual render
submission, after surface acquisition and preparation. CPU waits remain for
initialization, parameter/format changes and teardown, and the original synchronous
path is retained when timeline semaphores are unavailable. CPU wall-clock wait
statistics are not GPU execution-time measurements. Reduced CPU usage or latency
must be established on physical hardware; no speedup is asserted from compilation.

For an A/B comparison on the same device/stream, the existing CPU-synchronized
path can be selected for one launch with:

```sh
flatpak run --env=EVOPARSEQ_VULKAN_CPU_SYNC=1 io.github.vampywiz17.EvoParseq
```

The normal launch uses GPU synchronization when supported. Compare the same
codec, resolution, FPS, bitrate and window state; record CPU, decoded/presented
FPS and latency together. Backpressure wakeups reuse the existing transport
control loop and only query GPU progress while compressed input is deferred.
The offline suite also includes a native/wgpu timeline interoperability fixture
on a headless Vulkan adapter when available (software Vulkan is sufficient).
This validates semaphore handoff, not hardware video decoding or display latency.

## Per-surface lifetime tracking (0.1.19)

The existing two compressed-input/command/query slots are unchanged. The original
18-image pool is also unchanged: the parser's existing spare output slot keeps
exactly one selected picture leased until its replacement is selected. Intermediate
outputs are released immediately; format changes release/drain the old selection.
Reference ownership remains in the common parser, independently of selection.

Each native image records its last decode/layout-transition timeline value and
last submitted graphics-reader value. Decode waits only for readers of its actual
references and destination. Rendering waits only for the selected image's latest
decode/layout transition. A reference image cannot overlap sampling while its
layout changes; no simultaneous-read shortcut or extra image copy is introduced.
The submission hook gathers all video readers before attaching a single deduplicated
wait/signal pair. Stale callbacks for an unselected image do not draw. Repeated and
skipped presentation preserve the same bounded ownership rules.

CPU fixtures exercise long H.264/HEVC sequences and IDR restarts while retaining
one selected output; timeline policy tests cover unrelated images, reference-image
transitions, duplicate/multiple readers and repeated redraw. The headless Vulkan
handoff fixture consumes the same per-surface dependency plans as production.
Hardware playback and performance still need live validation.
