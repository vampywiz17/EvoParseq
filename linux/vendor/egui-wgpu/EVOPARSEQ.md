# EvoParseq Linux integration

Upstream egui-wgpu 0.33.3, MIT OR Apache-2.0 (emilk/egui).
Only the Linux Cargo manifest applies this path patch. Windows and Android use
the upstream library unchanged.

The public WgpuConfiguration gains an optional runtime_frame_latency atomic.
The existing winit painter reads it before acquiring a surface image. A changed
request reconfigures the existing surfaces through public wgpu Surface::configure;
there is no new decoder image, intermediate image, render pass or worker.
0 restores the upstream default, 1/2 request that maximum presentation latency.
This is a driver/compositor hint, not a measured queue depth or VRR claim.
