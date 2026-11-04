# EvoParseq Linux preview

The Wayland desktop uses egui on Vulkan. Authentication, 2FA, host discovery,
BUD/WebRTC protocols, Opus decoding and H.264 parsing/reference management reuse
the existing Rust implementations. Credentials remain in memory for this preview.

Test WebRTC and Test BUD check connection, H.264 keyframe reception and an
eight-second PipeWire audio sample. Play WebRTC and Play BUD start the native
H.264 video preview with PipeWire sound. Disconnect remains available during playback.

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
implemented in this preview. Input forwarding,
HEVC playback and persistent sign-in are later steps.

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
