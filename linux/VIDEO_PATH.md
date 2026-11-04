# Linux H.264 video path audit

Source review, 2026-11-04. Applies to the Linux Flatpak playback preview only.
This establishes application behavior, not driver/compositor/scanout behavior.

## Decoded images

`src/video_vulkan/session.rs` creates the original device-local NV12 images with
`VIDEO_DECODE_DST_KHR | VIDEO_DECODE_DPB_KHR | SAMPLED`. The same original image
is the decode destination and reference resource; unsupported combinations fail
capability checks instead of selecting a copy-based fallback.

The native image handle is wrapped with wgpu HAL `texture_from_raw` and
`create_texture_from_hal`. These wrappers do not create a second decoded image
or copy its contents. The Y and UV views in `src/video_vulkan/mod.rs` refer to
planes of that same allocation. Their lifetime retains the native image.

No application decoded-image copy, blit, CPU pixel readback, intermediate RGBA
video texture, cross-device transfer, or separate conversion render pass exists
in this path. Images are shared between queue families on the same Vulkan
device. Layout barriers and completion waits synchronize access; they are not
image copies. Completion waits remain conservative and can limit throughput.

## Presentation and dependencies

`src/video_vulkan/present.wgsl` samples original NV12 planes and performs range
expansion, YUV-to-RGB conversion, cropping and scaling while writing the output
render target. This costs GPU sampling and output writes. The egui callback
runs inside the UI's render pass, rather than rendering a separate video texture.

In pinned egui-wgpu 0.33.3 (`src/winit.rs`, `Painter::paint_and_update_textures`),
normal rendering targets the acquired surface texture. eframe 0.33.3 defaults
to multisampling disabled, so no MSAA resolve texture is used. Screenshot
capture uses a separate texture/copy/readback only if requested; the Linux
client does not request screenshots. UI font/buffer uploads are unrelated to
decoded video images.

The application copies compressed access-unit bytes into its bounded input
queue and then into mapped codec input memory. This is encoded input, not a
decoded-frame copy. No decoded image is mapped into CPU memory.

This implementation follows the project's GPU-resident presentation rule. It
does not implement or claim the application's strict **Zero copy mode**, because
presentation performs shader conversion. Hidden driver copies, Wayland
composition and direct scanout have not been measured or established.

## Queue scheduling

`src/playback.rs` uses the existing shared Rust `video_output::Queue`, including
its 32-packet / 8 MiB bounds, encoded-buffer recycling, timing counters and
keyframe recovery policy. Linux previously used its own eight-packet / 16 MiB limits. Each UI
update now decodes its initial bounded backlog, in order, then presents the
latest output. Incoming packets during that batch belong to the next update.
This removes the previous one-packet-per-refresh throughput limit without
discarding individual references in a continuous chain, adding threads or copying
decoded surfaces. Offline tests cover 60 FPS input versus 59.94 Hz rendering,
burst ordering, new arrivals during decoding, and stopping.

The live laptop still exhausted the bounded queue after this scheduling change.
The offline scheduling test does not establish native decoder throughput or
compositor scheduling. Preview 0.1.1 therefore adds bounded timing counters:
queue occupancy/oldest age, received/decoded counts, active decode age, peak
queue wait and decode callback time. On failure it also reports graphics and
decode fence wait durations using the existing shared Rust timing model.
These are CPU wall-clock observations, not GPU execution/scanout measurements.
They contain no payloads or credentials and are printed only on failure.
Buffer limits, synchronization, transport behavior and GPU surface handling
remain unchanged by this diagnostic step.

The 0.1.1 live report isolated the packet limit: eight queued packets occupied
only 69 KiB and arrived within 7.6 ms (oldest 7.9 ms). The active decode was
2.5 ms old; completion fence wait averaged 3.7 ms, graphics wait 0.1 ms.
This supports a short input burst rather than sustained decode starvation at
that failure. It does not establish where the network/host burst originated.
Preview 0.1.2 therefore removes the Linux-only eight-packet cap in favor of the
existing common limits. It reduces the byte ceiling from 16 MiB to 8 MiB;
it does not reserve a full queue or intentionally wait to fill it. Each update
drains its initial backlog and renders its latest output, preserving references.
A repeated nine-packet burst regression and byte-budget/release test cover the
observed failure pattern. Native throughput still requires live verification.

Preview 0.1.2 still filled the queue after switching to another window: 32
packets spanned 518.7 ms, oldest age 535.4 ms, with no active decode. Decode
completion wait still averaged 3.7 ms. This supports UI-side suspension rather
than sustained native decode starvation at this failure. The compositor's
internal reason for throttling remains unmeasured.

Preview 0.1.3 handles this explicitly. A full backlog retires the complete old
reference chain and suspends video input consumption until the application
receives focus. The network/audio loop remains active. Returning focus starts
the existing shared keyframe gate, one-second request rate limit and five-second
deadline, using each transport's existing encoder-refresh message. Cached codec
parameters are restored ahead of the resumed input; deltas are discarded until
an IDR arrives. This reference-chain boundary does not consume decoder-failure
attempts. The existing parser/native session handles the IDR and format changes;
no GPU code, surface copies, threads or Windows/Android behavior were changed.

This is a background-video suspension policy, not continuous background video
decoding. Tests cover a long hidden interval, focus return, cached-parameter
replay with an IDR lacking parameters, delta gating, repeated window switches,
the shared recovery timeout and failed refresh requests. Live validation of
BUD and WebRTC recovery remains necessary.

## Linux WebRTC binding (0.1.4)

The Linux streaming constructor accidentally supplied `playback.is_none()` as
the shared transport's legacy RSA-1024 compatibility flag. Consequently Test
WebRTC used the existing Windows/Android host policy, but Play WebRTC did not.
Both now use the same explicit Parsec compatibility policy. DTLS cipher suites,
signature verification, SDP fingerprint checks and the crypto libraries are
unchanged.

The native WebRTC reader already bypassed control receipts for video, but its
Linux build had only the inspector and no playback sink. The new `cfg(linux)`
binding in `src-native/src/attempt.rs` sends complete detached video messages
directly to the existing Linux compressed queue, outside the connection lock.
Audio continues through the shared audio pipeline. It adds no decoded-image
copies or queues. Windows and Android retain their existing compiled adapters.
Regression tests exercise the real playback constructor's compatibility flag
and detached video delivery/cancellation without a host or GPU.

The shared reader's Linux sink is a small UI-independent `NativeSink` trait;
the Linux frontend supplies its implementation. Android's Linux-hosted Rust
tests therefore do not acquire an egui/playback dependency. New queued control
receipts unpark the already existing Linux network consumer, as account
signaling and BUD already do. The 16-message control bound, SCTP ordering,
reliability, ICE/DTLS timers and cipher negotiation are unchanged. The Linux
failure report includes the failed channel and bounded queue counters, without
SDP, credentials or payload text.

## Current cleanup and media boundaries (0.1.6)

The legacy Test WebRTC/Test BUD UI actions, headless media sink and eight-second
sample/deadline branch have been removed. Actual playback always binds its
native video sink. The remaining capability page and --probe-video command are
supported driver diagnostics. Synthetic H.264 fixtures and test sink/queue
helpers are compiled only under tests; they are not a live fallback decoder.
Failure timing counters remain bounded and useful for regression diagnosis.

| Layer | Shared Rust | Required platform adapter / remaining difference |
| --- | --- | --- |
| WebRTC | attempt, protocol/control, detached media readers, inspector, audio decoder | Native video sink and OS presentation/output |
| BUD | SDK/header parsing; Windows and Linux additionally share bud_ingress, bounded video-first pump and encoder/keyframe rules | Android still has its own pump/gating adapter; it is not identical to bud_ingress |
| Video queue | video_output bounds, recovery, timing and buffer recycling; codec parameter cache | Vulkan session/interop, D3D11/MF/vendor decoders, MediaCodec/Surface |
| Audio | audio_stream Opus/PCM validation, decode and budgets | Linux PipeWire and Android AAudio use the shared callback ring; Windows uses its WASAPI adapter |

The Linux scope remains H.264 8-bit 4:2:0 and stereo audio. Windows' broader
codec/depth/chroma and channel support has not been copied into Linux by
advertising capabilities that the Linux backend cannot yet play. The Linux
manifest retains only dependencies used by its own or imported common modules;
shader validation (naga) is a dev-dependency. No new runtime dependency is added.

The Vulkan profile now includes VkVideoDecodeUsageInfoKHR with STREAMING in
both capability queries and all matching session/image/bitstream resource
creation. Vulkan defines this as a use-case hint without latency guarantees:
https://docs.vulkan.org/features/latest/features/proposals/VK_KHR_video_decode_queue.html
A codec profile (such as H.264 High) describes decoding compatibility; it does
not by itself select a low-latency mode. The profile follows the stream's SPS.

Other active latency policies are progressive decode, rejection of explicitly
declared H.264 reordering, immediate bounded backlog processing and selection
of the latest completed output. The 32-packet/8 MiB queue is a ceiling, not an
intentional prebuffer. DPB surfaces retain codec references; their allocation
count is not a presentation queue length. The 100 ms fence timeout is a failure
bound, not an inserted sleep. The shared 100 ms audio budget is also a ceiling,
not a target fill or device latency. PipeWire requests the Game role and 48 kHz
stereo S16; its quantum is negotiated by the server, with no explicit node.latency
request currently. Vsync/presentation depth uses egui-wgpu defaults. Per-frame
graphics/decode completion waits are still conservative. These are remaining
latency optimization/measurement points, not unnecessary image-copy paths.

Source audit confirms no application-side decoded image copy, blit, readback,
intermediate RGBA video texture or cross-device transfer. NV12 plane sampling
and conversion into the final render target remain necessary GPU work. Driver
internal copies and compositor/scanout latency remain unverified. Windows and
Android source files are unchanged by this Linux cleanup.

## Launcher and normal input (0.1.7)

The Windows launcher palette and labels are now in app_style.rs, consumed by
both native widget adapters. Windows COLORREF conversion retains the exact
previous values. Linux imports the existing computer-picker model for host
selection/refresh and peer validation. Native Wayland decorations remain on.
The logo is decoded/uploaded once as a UI asset, independently of decoded video.

The Linux event adapter translates egui events to the existing desktop_input
Engine and input_handoff Inbox (also used by Android). The connection worker
owns the shared engine; normal control frames use BUD send_frame or WebRTC's
control channel. UI input unparks that existing worker; no new input thread,
transport queue, timer or GPU copy is added. The current presented image extent
is atomically published for the shared coordinate mapping. The presentation
callback now returns its existing rectangle/extent, with unchanged GPU resources.
Focus loss, the local release shortcut, disconnect and overflow send the normal
release command. Host cursor flags drive the supported Wayland lock request.
Physical key events use the documented eframe create_native/winit ApplicationHandler integration, preserving ISO/numpad and side-specific modifiers. No private framework hooks or new event queue are used.

Flatpak includes ANV_DEBUG=video-decode by default, without modifying the host
Mesa installation or the non-Intel path. No HID enablement or input-device
sandbox permission is added. GPU synchronization/decode/presentation code is
unchanged apart from returning existing presentation geometry to the UI.

## Window, settings and cursor corrections (0.1.8)

winit's wayland-csd-adwaita feature restores the documented standard GNOME CSD
fallback without adding GTK, another renderer or an input-device permission.
Server-side decorations are still preferred when the compositor provides them.
Settings shares its existing Windows palette/tab labels and follows its top-tab
layout, with only implemented Network and Video content. No fullscreen control
is exposed in the launcher. The shared viewport module calculates initial
stream-window size from decoded extent, DPI and monitor bounds; the Linux
adapter sends native size commands only on a new stream extent, preserving
manual resizing. Wayland controls final position and usable desktop geometry.
The client cursor icon and native cursor-visible flag are reset together on
release/launcher entry; consumed remote mouse motion no longer hides hover
bookkeeping from egui. These changes do not alter decode or GPU surface handling.

## Desktop parity and centered presentation (0.1.9)

The application now consumes the canonical Windows logical GUI metrics/copy
from app_style.rs. The live video contains only the translucent draggable session
badge; its panel currently exposes Disconnect. Diagnostic text is not drawn over
video. Presentation uses the shared Viewport letterbox on both axes, and the same
rectangle defines normal mouse coordinates. Decoder surfaces, texture imports,
shader pipeline, queue bounds, pacing and transport/audio paths are unchanged.
This remains GPU-resident NV12 conversion, not the project's Zero copy mode.
The cursor patch is scoped to the Linux UI dependency graph and adds no media
work. Windows UI substitutions retain their exact previous values; Windows and
Android CI must pass independently of the Linux Flatpak checks.


## Android shared BUD ingress

The Android adapter now uses the same bud_ingress.rs session capabilities,
borrowed media parsing, encoder-generation resets, parameter/IDR gate and
bounded video-first/audio pump as Windows/Linux. MediaCodec/Surface and AAudio
remain Android adapters. There is no new media queue or decoded-image copy.
A full ingress batch is drained before the Android consumer parks; input/control
processing still runs between bounded batches. Rotation remains explicitly
unsupported on Android, matching the previous adapter. Raw audio and 4:4:4 are
not advertised by this change. Normal Parsec input/controller paths are unchanged.
