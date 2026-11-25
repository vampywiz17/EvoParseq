# EvoParseq native client

The default build is the native Rust application, with WebRTC/lowlat, Win32 UI,
GPU video decoding/presentation, audio, input and account/session handling.

```powershell
cargo test --locked --manifest-path src-native/Cargo.toml
cargo build --locked --release --manifest-path src-native/Cargo.toml
```

Windows builds require MSVC and the Windows SDK. Run `scripts/build.ps1` for the
checked portable package. See the [root README](../README.md) and
[architecture](../docs/native-core.md) for features and validation limits.
