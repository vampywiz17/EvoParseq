# EvoParseq Linux preview

The Wayland desktop uses egui on Vulkan. Authentication, 2FA, host discovery,
BUD/WebRTC protocols, Opus decoding and H.264 parsing/reference management reuse
the existing Rust implementations. Credentials remain in memory for this preview.

Play WebRTC and Play BUD start the native H.264 video preview with PipeWire
sound. Disconnect remains available during playback. The former standalone
eight-second connection/audio tests have been removed; Video capabilities and
the --probe-video command remain available for driver diagnostics.

The playback scope is progressive H.264, 8-bit 4:2:0, without frame reordering.
The host remains unmodified. The decoder's original NV12 surfaces feed the
presentation shader on the same Vulkan device as the UI. The shader performs
range expansion, cropping, scaling and YUV-to-RGB conversion in the output pass.
This is GPU-resident rendering. Application Zero copy mode and compositor/scanout
behavior require separate implementation or verification.

The initial backend uses bounded completion waits to synchronize Vulkan Video
with wgpu. Latency, sustained playback and driver compatibility require live
hardware validation. Encoded input uses the shared bounded Rust video queue.
If the UI stops consuming video (for example when another window covers it),
a full backlog suspends video without ending the connection or audio pipeline.
Returning focus requests a fresh keyframe using the shared recovery gate and
deadline, then resumes playback. Continuous background video decoding is not
implemented in this preview. HEVC playback and persistent sign-in are later steps.

WebRTC probes and playback use the same existing Parsec host certificate
compatibility policy as Windows/Android. Native video is delivered directly
from the shared detached WebRTC reader to the Linux queue. Signature and SDP
fingerprint verification remain enabled; cipher negotiation is unchanged.

BUD playback uses the same Rust media ingress as Windows: video-first batches
of eight messages per media stream, immediate draining of full batches, borrowed
payload parsing, encoder-generation handling and keyframe gating. Only native
output adapters differ. The Linux preview still requests H.264 8-bit and stereo
Opus; rotated playback is not implemented. No new GPU copies or decode worker
threads are introduced by this handoff.

The Flatpak uses Freedesktop 25.08 and matching driver extensions. Its build uses
the Rust and LLVM SDK extensions. The host supplies Wayland, PipeWire, the kernel
and a compatible GPU. The sandbox accesses the user's PipeWire socket for audio.

Install using flatpak install --user --reinstall EvoParseq-Linux.flatpak,
then launch using flatpak run io.github.vampywiz17.EvoParseq.

On Intel drivers exposing experimental Vulkan Video through Mesa's documented
switch, explicitly opt in for a test launch using
flatpak run --env=ANV_DEBUG=video-decode io.github.vampywiz17.EvoParseq.

The Video capabilities page lists driver-reported profiles and sampleable output
formats. A positive report establishes eligibility for a playback attempt.
The backend additionally checks that NV12 decode output and reference surfaces
can be shared with shader sampling, with sufficient DPB slots and reference capacity.

The decode profile includes Vulkan's STREAMING usage hint consistently in
capability queries, session creation and associated images/input resources.
The progressive H.264 path rejects explicitly declared frame reordering and
presents the latest completed output after decoding its bounded input batch.
This is a low-latency policy, not a measured end-to-end latency guarantee.
Conservative GPU completion waits, compositor presentation and PipeWire's
negotiated quantum still affect latency. See VIDEO_PATH.md for the audit.

The launcher uses the Windows client's shared Rust palette, labels and computer
picker policy. Native window decorations belong to the Wayland desktop; Full
screen uses the native fullscreen command. Windows and Linux colors/labels are
defined once in src-native/src/app_style.rs. No Win32 widgets are ported to Linux.

Normal Parsec mouse/keyboard control is available on BUD and WebRTC. Click the
video to activate it; Ctrl+Shift+M releases input. The translucent draggable
session icon opens the controls (Disconnect only for now). Focus loss, disconnect and queue overload release host keys
and buttons. Geometry, key framing, button mapping, fractional relative motion
and the bounded input inbox reuse the existing shared Rust implementations.
Host cursor flags request relative Wayland pointer locking. No HID devices or
host HID mode are enabled, and no additional Flatpak device permission is used.
Physical keys are received through eframe's documented external winit event
loop, preserving ISO keys, numpad keys and left/right modifiers. Host keyboard
layout and IME handle the physical input; local text/IME payloads are not sent.
System/compositor shortcuts remain local.
Clipboard shortcuts target the host clipboard; local clipboard text is never
synchronized. Pointer locking and physical input still require a live Wayland test.

The Flatpak defaults ANV_DEBUG=video-decode for the Intel Mesa Vulkan Video path.
Launch normally with flatpak run io.github.vampywiz17.EvoParseq. This documented
vendor switch enables experimental Intel driver support; it is not a generic
Vulkan requirement or a guarantee for every GPU/driver. It can be overridden
using Flatpak's --env option.

Preview 0.1.9 uses the Windows launcher/settings text, logical font sizes and
control rectangles from shared `src-native/src/app_style.rs`. Unsupported
controls are hidden; their positions are reserved so later integrations do not
create another platform-specific layout. For now only Network/Transport is
editable. Raw GPU capability output remains available via `--probe-video`,
not as a replacement Video settings page. Settings keeps its Windows 820px
canvas with scrollbars when necessary; Transport retains its Windows row.
Native font rasterization and installed font faces can differ between systems.

The video has no status text or input/fullscreen buttons. It uses the Windows
48px translucent draggable session badge and 32px logo, initially at (16,16).
The session panel exposes Disconnect in its Windows position, scrolling to it
while other features are not integrated. Badge/panel interaction releases host
input and never sends local control clicks or drags to the host.

The stream fits its first decoded frame, with existing DPI/monitor bounds.
On resize both presentation and mouse mapping use the common centered
letterbox geometry. This changes only the viewport, with no added media copy.

Standard winit Adwaita client-side decorations remain the GNOME Wayland
fallback; compositor-side decorations are used when available. Native window
buttons and resizing follow the desktop, independently of application content.
Cursor visibility/grab changes are sent only at capture transitions and release,
rather than overwriting decoration cursors every frame. A narrow Linux-only
patch to pinned egui-winit invalidates its cursor cache on surface entry/exit;
see `vendor/egui-winit/EVOPARSEQ.md`. Live Wayland cursor validation is required.


Preview 0.1.10 fixes login/picker and Settings window sizing to the Windows
client dimensions (744x551 and 984x721 logical content pixels), plus the local
menu strip. Min/max constraints and non-resizable native decorations apply only
to these pages. Streaming restores resizing and centered aspect fitting.
The complete Windows Settings, session panel and Connection stats presentation
is now included, with shared descriptors/copy in desktop_forms.rs. Navigation,
Disconnect and the existing transport selection work; other unintegrated
controls are disabled and measurements show N/A. No synthetic live values are
shown. The top menu follows Windows (launcher Help; stream Settings, Connection
stats, Help). About/update commands remain disabled until integrated.

The Flatpak now supplies XCURSOR_PATH entries for Flatpak's documented host icon
mounts. It also bundles GNOME Adwaita 46.2 cursor files (unmodified, upstream
checksum verified) as a fallback for older Wayland compositors without the
cursor-shape protocol. Before starting threads, a missing/incomplete cursor
theme selects the bundled fallback. No host-filesystem permission is added.
This addresses missing cursor assets, which a cursor-cache invalidation cannot
repair. https://docs.flatpak.org/en/latest/desktop-integration.html#icons
Adwaita artwork is by the GNOME Project (https://www.gnome.org), distributed
under the upstream dual LGPLv3 / CC-BY-SA 3.0 license; copies ship in the bundle.
Live Wayland validation remains required.
