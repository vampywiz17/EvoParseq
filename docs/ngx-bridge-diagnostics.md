# NVIDIA RTX VSR bridge diagnostics

This page compares the last direct-SDK implementation on `dev` with the optional NGX bridge in PR #24. It is a **source-level comparison**, not proof that the NVIDIA driver supports the new DLL arrangement on a specific GPU.

## Contract retained from the direct-SDK client

| NGX input or operation | Original `dev` | Optional bridge |
| --- | --- | --- |
| D3D11 device | Selected decoder/render device | Same device pointer |
| D3D11 context | `GetImmediateContext()` | Same context pointer |
| App ID | `0` | `0` |
| SDK API version | `0x15` | `0x15` |
| Data path | `validated_directory()` | Same canonical directory |
| Feature search list | One entry: verified VSR cache directory | One entry: same directory |
| Capability parameters | `GetCapabilityParameters` | Same NGX function |
| Availability key | `VSR.Available` | Same key |
| Feature | `16` (VSR) | `16` |
| D3D10Multithread lock | Held by direct caller | Held by EXE during bridge calls |
| Shutdown | Release feature/parameters, then shutdown | Same sequence |

The intentional differences are **where the static SDK code runs** (inside an independently downloaded, hash-pinned bridge DLL rather than the EXE) and **optional NGX logging** (enabled only when EvoParseq's operational event log is enabled). No API version, feature ID, GPU selection, or SDK search-path argument has been changed in this diagnostic update.

## Events to inspect

- `video_ngx_module_probe`: emitted before and after NGX initialization. `runtime_loaded` checks whether Windows has loaded a module named `nvngx_vsr.dll`; `runtime_matches_verified_file` compares its resolved location to the SHA-256-verified per-user runtime file. It does **not** force-load the module. `ngx_driver_core_loaded` probes `_nvngx.dll`, if present. `null` indicates that a path comparison was not available.
- `video_ngx_sdk_log`: NVIDIA's NGX callback during initialization, limited to 48 events per attempt. Requires EvoParseq event logging. To preserve log privacy, records only the SDK log level and component, the message byte length, a SHA-256 fingerprint and a coarse diagnostic category. **No raw NVIDIA message text or filesystem paths are written.** Uses NGX logging level ON (not VERBOSE) and disables NVIDIA's other log sinks while the callback is registered.
- `video_vsr_fallback`: exact failing NGX initialization stage and SDK status. `VSR.Available is false` means the capability query succeeded but NVIDIA returned zero, not that the SDK failed to initialize.
- `video_vsr_setup_finished`: setup duration and whether the VSR instance exists.
- `video_vsr_active`: at least one GPU-side VSR evaluation completed.

Since the bridge ABI is now **v3**, download the Windows EXE and the optional NGX bridge artifact from the **same successful GitHub Actions run**. Mixing v2/v3 builds is unsupported. The EXE checks the bridge's SHA-256 before loading it.

## NVIDIA download behavior

The NGC metadata endpoint may return a HTTPS `Location` header in either a redirect or a successful metadata response. It can also return the archive bytes directly. EvoParseq does not auto-follow redirects during that **initial discovery request**, then resolves an HTTPS archive URL when supplied. The final download client allows only HTTPS redirects. In all cases the 63,493,768-byte archive must match the pinned SHA-256 before any extraction; the extracted 19,140,144-byte runtime also has its own independent pinned hash.

## Testing on RTX hardware

Enable operational event logging, connect with AI Upscaling off to verify ordinary playback, then enable Quality 1 with a genuine source-to-output upscaling ratio. Compare `video_ngx_module_probe`, `video_ngx_sdk_log`, `video_vsr_fallback`, and `video_vsr_active` in the JSONL log. The zero-copy/regular GPU fallback should continue to work on any NGX failure.

A green CI run checks compilation, static bridge packaging, offline download selection and integrity tests. Only a real RTX device/driver run can establish whether NVIDIA reports `VSR.Available = 1` and frames are evaluated.
