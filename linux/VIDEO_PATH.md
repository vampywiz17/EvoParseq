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

`src/playback.rs` uses the existing shared Rust `video_output::MAX_FRAMES` (32)
and `MAX_BYTES` (8 MiB) limits. These constants and the Windows/Android paths
are unchanged; Linux previously used its own eight-packet / 16 MiB limits. Each UI
update now decodes its initial bounded backlog, in order, then presents the
latest output. Incoming packets during that batch belong to the next update.
This removes the previous one-packet-per-refresh throughput limit without
dropping compressed references, growing the buffer, adding threads or copying
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
