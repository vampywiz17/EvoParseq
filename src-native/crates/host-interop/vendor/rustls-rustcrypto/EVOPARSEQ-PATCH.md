# RustCrypto provider provenance

Upstream: https://github.com/RustCrypto/rustls-rustcrypto
Registry version: 0.1.0-pre.0
Registry archive SHA-256: c2856ab13701744c79101d343f8cebb5164eec3aafcf3ee1867b5b6033587daa
Upstream Git revision: 0494647098b7106a67dc5c44e3df83e5be23660c
License: MIT OR Apache-2.0; original license files are retained.

All src/ and tests/ files are copied unchanged from the open-source registry
package. No native Phaze code is included.

Local manifest-only changes:
- Disable accidental publishing of this local copy.
- Alias the maintained, pinned pastey 0.2.3 package as paste, following its
  documented drop-in migration. This removes unmaintained paste 1.0.15
  (RUSTSEC-2024-0436) without suppressing the dependency-policy check.

Cryptographic source, algorithms and TLS policy are unchanged. The provider
remains experimental and is not a production-security recommendation.
Original builder integration tests are retained and run by the backend CI.
