EvoParseq v0.9.7 - Release security reports

This stable maintenance release builds on **EvoParseq v0.9.6**. Windows x64
packages and a distribution-signed Android TV APK are included. Application
behavior is unchanged; this release adds automated publication of malware-scan
evidence for the final release binaries. Linux alpha builds remain separate.

## What's new and improved

- **Published release security reports:** scan the final Windows EXE and Android
  APK with VirusTotal while the prepared release remains a draft. Add a SHA-256-linked results
  table to these notes and attach SECURITY-SCAN.md and SECURITY-SCAN.json.
  The report records per-engine findings and distinguishes unsupported engines,
  errors and timeouts. Publication is blocked by malicious or suspicious findings, incomplete
  analyses or errors, except the owner-approved sole Trapmine
  `suspicious.low.ml.score` finding on a Windows EXE. This narrow exception is
  explicitly disclosed with unchanged detection counts; it is not a confirmed
  false positive. Any other finding remains blocking.
- **Release automation validation:** add offline regression tests, including a
  real Flatpak extraction fixture, and measured Python coverage in the existing
  SonarQube quality gate. The scanner also supports future Flatpak releases and
  their embedded Linux executable; this stable release contains no Flatpak.
- **Version alignment:** Windows and Android report 0.9.7. The Android versionCode
  is 31 so existing distribution-signed installations can update in place.
  No decoder, rendering, audio, networking, input, permissions, signing-key or
  application-interface behavior changes are included.

## Compatibility and validation

Requirements and hardware limitations are unchanged from v0.9.6: Windows 11 x64
with Direct3D feature level 11.0+ and compatible drivers; Android 10 (API 29)+
with ARMv7 or ARM64. Existing feature status and tested-device limits remain in
README and the Android TV guide. The Windows and Android build/test workflows,
Code quality checks and strict SonarQube gate are required before publication.

VirusTotal results are a point-in-time record, **not a safety certification or
Google Play Protect approval**. No-detection results do not guarantee safety;
detections require investigation and are not automatically dismissed as false
positives. Uploads can be shared by VirusTotal with its partners. This release
does not claim to resolve the reported Android Play Protect warning.

## Requirements and upgrading

**Windows:** close the client and extract **EvoParseq-v0.9.7-win64.zip**, retaining
settings.json under the same Windows account. Native protected remembered sessions
and settings are retained. **ParsecWebTurn-v0.9.7-win64.zip** contains the identical
executable under the old filename. ZIPs exclude user settings and the optional
NVIDIA runtime. Protected credentials are not portable across accounts/machines.

**Android TV:** install **EvoParseq-TV.apk** or use **Settings > Updates** in an
existing distribution-signed installation. The signing certificate is unchanged;
normal updates retain settings and sign-in. **EvoParseq-TV-v0.9.7.zip** includes the
same APK, setup guide and license notices. Old debug-signed previews require the
already-documented one-time reinstall only if Android rejects their certificate.

Verify downloads with SHA256SUMS.txt. Source changes since the previous stable
release: https://github.com/vampywiz17/EvoParseq/compare/v0.9.6...v0.9.7
