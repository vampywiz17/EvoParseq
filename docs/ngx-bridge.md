# Optional NVIDIA NGX bridge: single-EXE Windows delivery

The public EvoParseq Windows package continues to carry exactly **one EXE**.
No NVIDIA SDK object code is statically linked into `EvoParseq.exe`,
and **no NGX bridge DLL is bundled in the Windows ZIP**.

## Architecture

1. The Windows CI fetches NVIDIA RTX Video SDK 1.1.0 and builds
   `src-native/ngx-bridge` as a **separate** `evoparseq_ngx_bridge.dll`.
   This is the **only** target linking `nvsdk_ngx_s.lib`.
2. The CI copies it to `EvoParseq-NGX-Bridge.dll` and computes its SHA-256
   **before** building the public `EvoParseq.exe`. The hash becomes the
   compile-time value `EVOPARSEQ_NGX_BRIDGE_SHA256` in the EXE.
3. The release operator must attach **EvoParseq-NGX-Bridge.dll** as a
   separate asset to the *same version* of the EvoParseq GitHub Release,
   e.g. `v0.9.9-dev.1`. **Do not put this DLL in the normal Windows ZIP.**
   This separately distributed adapter contains proprietary-linked NVIDIA
   code, is not covered by the EvoParseq MIT license, and its redistribution
   must be permitted by the NVIDIA SDK license.
4. Only after an explicit user opt-in, EvoParseq downloads this release asset
   and NVIDIA's pinned `nvngx_vsr.dll`. The runtime stores both under the
   user's `%LOCALAPPDATA%/EvoParseq/vsr/1.1.0`.
5. EvoParseq validates the adapter against its **embedded SHA-256** and the
   NVIDIA runtime against the pinned SDK SHA-256, and stores the bridge with a hash-qualified filename (allowing concurrent
   EvoParseq versions), then uses an absolute-path
   `LoadLibraryExW` and resolves the adapter's private ABI v1.
6. The adapter forwards the same D3D11 texture and VSR quality/viewport data
   to NVIDIA NGX as the earlier static integration. The main program retains
   GPU surface ownership and the D3D10Multithread critical section.
   If the adapter is unavailable or invalid, normal video playback is
   unaffected; AI upscaling reports a failure rather than silently substituting
   another algorithm.

## Release checklist

- Confirm both the main EXE and the bridge were compiled **in the same CI
  build**, using the exact pinned SHA-256.
- Upload the separate `EvoParseq-NGX-Bridge.dll` from the
  `EvoParseq-optional-NGX-bridge` workflow artifact to the matching release
  tag, alongside the one-EXE Windows ZIP.
- Include the applicable NVIDIA license and notice with the adapter asset
  as needed by NVIDIA's redistribution terms.
- The Windows workflow scans the main EXE for `NVSDK_NGX_D3D11_Init`
  and `nvsdk_ngx_d3d11_lib.cpp` to catch accidental re-linking.
- Test on an RTX system with NVIDIA VSR enabled: consent, download,
  SHA-256 validation, D3D11 initialization, visible upscaling, reconnect,
  shutdown, and ordinary video with VSR off.

**Limitation:** the GitHub-hosted adapter must be uploaded to the matching
release before a fresh user's consent flow can succeed. GitHub Actions
compile tests are not a substitute for live RTX VSR validation.
