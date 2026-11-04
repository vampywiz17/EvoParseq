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

`src/playback.rs` retains the existing eight-packet and byte limits. Each UI
update now decodes its initial bounded backlog, in order, then presents the
latest output. Incoming packets during that batch belong to the next update.
This removes the previous one-packet-per-refresh throughput limit without
dropping compressed references, growing the buffer, adding threads or copying
decoded surfaces. Offline tests cover 60 FPS input versus 59.94 Hz rendering,
burst ordering, new arrivals during decoding, and stopping.
