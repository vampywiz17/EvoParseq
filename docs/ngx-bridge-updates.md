# Signed NVIDIA NGX bridge updates

EvoParseq uses a stable adapter filename:
`%LOCALAPPDATA%\\EvoParseq\\vsr\\1.1.0\\EvoParseq-NGX-Bridge.dll`.
The existing `nvngx_vsr.dll` is still downloaded and independently verified
against the exact pinned NVIDIA SDK 1.1.0 archive. No change to that trust boundary.

## Startup and compatibility

- The installer runs before the UI and before any NGX library is loaded. The
  already-approved opt-in setting is respected; declined users see no download.
- `dev` and `stable` read distinct HTTPS manifests from the project's `dev`
  and `main` branches. **The manifest must be Ed25519 signed**, with the
  verification public key embedded into the EXE at build time.
- Reject invalid signatures, changed project hostname, wrong channel, unexpected
  release asset path, oversized downloads, unknown ABI (currently 3), or a
  malformed digest. Check the downloaded DLL SHA-256 **before** loading it.
- Compare the signed SHA-256 and file size with the installed file. When unchanged
  no bridge download or replacement occurs, even after many EvoParseq test builds.
- Store the signed manifest alongside the stable DLL. On network errors, retain
  a verified offline copy. An older remote sequence cannot downgrade the
  currently cached signed manifest.
- On older installations, a DLL whose name contains the EXE's bootstrap SHA-256
  can be migrated into the stable filename without downloading it again.
- A running second EvoParseq instance may lock the DLL. Windows replacement
  failure must leave the existing verified DLL intact and retry next startup.
- Ordinary GPU playback is always available if the optional adapter is missing.
  No repetitive startup error modal appears.

The stable filename is **not** the trust anchor. Ed25519 authenticates the
release metadata, and SHA-256 binds it to exact adapter bytes. An SHA-256
field inside the DLL cannot validate the DLL itself.

## Release and development process

**Do not distribute or automatically publish the separately built NGX Bridge
until the NVIDIA SDK's redistribution rights and notices are reviewed.**
The Windows CI stages the DLL, its SHA-256 and `bridge-metadata.json` as
artifacts; publishing is a separate release-operator action.

1. Create a private signing key **offline**, never in the Git repository:
   `python scripts/ngx-bridge-manifest.py keygen --private ngx-signing-key.pem`
   (requires `python -m pip install cryptography`). Protect and back up the
   key. The command outputs the base64 public key.
2. Set GitHub Actions repository **variable**
   `NGX_MANIFEST_PUBLIC_KEY_B64` to that public key. Windows CI embeds it
   using `EVOPARSEQ_NGX_MANIFEST_PUBLIC_KEY_B64`. Without the variable,
   builds keep the previous compiled-hash bootstrap behavior and do not
   enable signed remote updates.
3. After release/licensing approval, attach the CI-produced
   `EvoParseq-NGX-Bridge.dll` to an approved project GitHub Release
   (for example `v0.9.9-dev.2`). Do not bundle it in the public EXE ZIP.
4. Sign the **exact published DLL**, using a strictly increasing sequence:
   `python scripts/ngx-bridge-manifest.py sign --private ngx-signing-key.pem
   --channel dev --sequence 1 --version 0.9.9-dev.2 --tag v0.9.9-dev.2
   --dll EvoParseq-NGX-Bridge.dll --out ngx-bridge/channels/dev.json`
5. Verify with the public key and commit only the **signed envelope** to
   `ngx-bridge/channels/dev.json` on `dev` (stable releases use
   `ngx-bridge/channels/stable.json` on `main`). Never commit the
   private key or sign mutable third-party URLs.
6. Later NGX Bridge updates may be published independently of the EXE, **only**
   if the ABI remains compatible. Increment `sequence`; for unchanged
   bridge bytes the local DLL is retained.

The public manifests are never emitted automatically by PR CI, so no PR can
silently publish an NVIDIA-linked binary or use a production signing key.
A manifest update requires explicit release and signing approval.

## Validation checklist

- Same signed manifest and DLL: no DLL download.
- Newer signed manifest with the same hash: update metadata, retain DLL.
- Newer compatible hash: download, verify, replace prior to LoadLibrary.
- Corrupt signature, unexpected URL, unsupported ABI: refuse the update.
- Lost connection or unavailable GitHub: use locally verified DLL.
- Existing old-style hash-named DLL: migrate once, preserve the source.
- Locked DLL: keep the previous working version and retry on next launch.
- Signed manifest absent or public key not provisioned: never trust mutable
  metadata; use only the bootstrap SHA-256.
