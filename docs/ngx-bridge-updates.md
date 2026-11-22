# Signed NVIDIA NGX Bridge updates

The adapter filename is always **`EvoParseq-NGX-Bridge.dll`**. Unlike
the earlier shared root DLL, every independently verified adapter is stored
in its own immutable content-addressed directory:

```text
%LOCALAPPDATA%/EvoParseq/vsr/1.1.0/
    nvngx_vsr.dll
    bridge-manifest-dev.json
    bridge-manifest-stable.json
    bridge-cache/<sha256>/EvoParseq-NGX-Bridge.dll
```

The SHA-256 belongs in the signed manifest and cache *directory*, not the
DLL's filename. Different EXE builds or channels can therefore reuse the
same bytes without downloading again; an old build cannot replace a new
adapter or require re-downloading it on each switch. Cached adapters are
never overwritten in place while another Windows process may have them
loaded. The pinned NVIDIA SDK runtime is still independently verified.

## Trust and startup

- The automatic check runs before NGX loading, only on Windows RTX systems
  with the user's saved download consent. Normal GPU video works without NGX.
- Signed `dev`/`stable` manifests are independently selected from the
  project's `dev` and `main` branches. The client authenticates the
  Ed25519 signature with a public key embedded in the EXE.
- ABI must be 3; URL must name a fixed
  `https://github.com/vampywiz17/EvoParseq/releases/download/<tag>/EvoParseq-NGX-Bridge.dll`
  asset; SHA-256, size, channel and monotonic sequence are enforced.
- Each startup compares the manifest SHA-256 to the matching immutable
  cached DLL. An identical DLL is **never downloaded again**, even if its
  manifest version changed or the EXE was rebuilt.
- Signed manifests are cached separately for `dev` and `stable`. An
  invalid, unavailable or downgraded remote manifest cannot replace the
  last trusted local signed manifest. A verified cached DLL remains usable
  offline.
- A release EXE with no signed-manifest key (or no published manifest) uses
  only its **compiled SHA-256** to select the matching immutable cached
  adapter. Previously installed `evoparseq-ngx-bridge-<sha256>.dll` files
  are accepted and migrated to the content-addressed directory after
  verification. No unsigned bootstrap can modify a signed adapter.
- Failed signed-update checks still permit an EXE-pinned bootstrap asset
  to be installed. NVIDIA runtime installation is independent of bridge
  update errors and is not skipped.
- Updates are serialized with an OS-owned Windows named mutex. Temporary
  files are exclusively created with unique process/nonce names. The mutex
  is automatically released if an updater process crashes. DLLs in an
  existing content-addressed directory remain immutable.
- No repeated startup download-error dialog is shown. Optional failures
  remain visible in the event log.

## Operator-only release procedure

**This PR does not publish an NVIDIA-linked DLL.** Redistributability and
required NVIDIA license notices must be checked separately. CI only stages
the bridge DLL, checksum and `bridge-metadata.json`.

The signing tool deliberately uses fixed paths rather than arbitrary
file paths supplied on the command line (preventing unsafe path traversal):

- Private key: `~/.config/evoparseq/ngx-signing-key.pem` (outside Git).
- Input DLL: `dist/ngx-bridge/EvoParseq-NGX-Bridge.dll`.
- Signed output: `ngx-bridge/channels/dev.json` or `stable.json`.

1. Install the Python `cryptography` library, then run
   `python scripts/ngx-bridge-manifest.py keygen`. Protect the private
   key, store an offline backup, and never commit it.
2. Configure GitHub Actions repository variable
   `NGX_MANIFEST_PUBLIC_KEY_B64` from the printed public key.
   The Windows CI compiles that key and the channel into new EXEs.
   Without the public key, only the compiled SHA-256 bootstrap is accepted.
3. After an NVIDIA license review, publish the **separate**
   `EvoParseq-NGX-Bridge.dll` release asset (do not bundle in the normal
   EXE ZIP), and copy that exact reviewed binary into the fixed input path.
4. Sign a monotonically increasing sequence:
   ```powershell
   python scripts/ngx-bridge-manifest.py sign --channel dev --sequence 1 --version 0.9.9-dev.2 --tag v0.9.9-dev.2
   ```
5. Verify with the public key:
   ```powershell
   python scripts/ngx-bridge-manifest.py verify --channel dev --public-key BASE64_PUBLIC_KEY
   ```
6. Commit only the signed `dev.json` on `dev`; for stable releases,
   commit `stable.json` on `main`. Never commit private key material.

## Verification

- Unchanged SHA-256 across EXE versions: reuse the same cached DLL.
- Different signed SHA-256: install a separate immutable DLL, update
  only the corresponding channel's signed manifest.
- Old or new EXE: retain different verified content in parallel.
- Missing manifest: bootstrap with compiled SHA-256; do not block
  the independent NVIDIA SDK runtime installation.
- Failed download, bad signature, wrong ABI or unsupported URL: fall back
  to last verified signed or pinned adapter; normal video remains available.
- Parallel startup: update mutex and unique staging prevent collisions.
- Release publication requires operator approval and NVIDIA licensing.
