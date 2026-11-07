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

Native Vulkan video additionally uses paired CallbackTrait before_submit /
after_submit hooks. The winit painter invokes these immediately around its
existing Queue::submit, after preparation and successful surface acquisition.
The video callback attaches GPU waits/signals only; it submits no work itself.
Other callbacks retain default no-op hooks. Multiple callbacks for the same
session are deduplicated until after that submission. No additional render pass,
image or queue submission is introduced by the hooks.

The Linux-only prepare_submit hook first gathers all per-image readers in the
existing render submission. before_submit then attaches their combined native
semaphore dependencies once per session; after_submit retires the gathered set.
This handles duplicate/clipped callbacks without duplicate timeline signals.
