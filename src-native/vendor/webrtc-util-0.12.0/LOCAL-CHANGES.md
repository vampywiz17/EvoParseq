# EvoParseq local dependency configuration

Base: crates.io webrtc-util 0.12.0 (crate SHA-256
`d1c0c7e0c8f280f2bbfae442701465777ac07adaf46ce0c5863cd58e13fe472a`); upstream provenance is retained in
`.cargo_vcs_info.json`. All Rust sources are byte-identical to that release.

The non-Windows nix 0.26.2 dependency disables default features and explicitly
requests `net` (which includes `socket`). Its only nix call sites are in
`src/ifaces/ffi/unix/mod.rs`: getifaddrs and socket address interpretation.

The previous nix defaults also enabled unused AIO file operations, bringing in
pin-utils and RUSTSEC-2026-0441 (unmaintained). Narrowing the feature set removes
that unused dependency without a library upgrade, advisory exception, transport
change or replacement for the existing interface enumeration. Windows retains
the same dependency features and source. Preserve both upstream licenses.

Apply this same patch at each application/test workspace root; patch tables in
dependencies are not inherited by Cargo. Android and Linux use this same source.
