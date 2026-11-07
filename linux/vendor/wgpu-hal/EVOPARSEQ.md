# Linux-only external semaphore wait integration

Base: crates.io wgpu-hal 27.0.4, checksum
`5b21cb61c57ee198bc4aff71aeadff4cbb80b927beb912506af9c780d64313ce`.
Original MIT/Apache licenses are retained and installed in the Flatpak.

Only linux/Cargo.toml patches this crate. Windows/Android manifests and lockfiles
continue to use their original dependencies.

The Vulkan queue adds `add_wait_semaphore`, following the public upstream API:
https://wgpu.rs/doc/wgpu_hal/vulkan/struct.Queue.html#method.add_wait_semaphore
This is a small compatibility implementation for 27.0.4, not a wholesale upstream
version upgrade or a claim of byte-identical backport. Other HAL backends remain
unmodified. External waits are consumed by the next submit, with values aligned
with existing binary swapchain/relay waits and timeline signal values preserved.
The caller retains semaphore ownership and attaches dependencies immediately
before the intended submit. No private fields are accessed by the application.

Modified upstream files: src/vulkan/mod.rs, adapter.rs, semaphore_list.rs.
The latter's CPU-only submit-structure tests also run in Linux video_contract.
