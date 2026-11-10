# Release malware scanning

Published stable and prerelease GitHub releases trigger `Release malware scan`.
Pushes, pull requests, drafts and ordinary build artifacts do not upload anything.
The workflow uses the repository secret `VIRUSTOTAL`; its value must never be put
in source, logs, reports or command arguments. Only the scanner step receives it.

Prepare releases as drafts, upload **all final assets** (including the signed APK),
then publish.
An asset added after publication requires rerunning the release's scan job.
Publishing from automation using `GITHUB_TOKEN` does not normally trigger another
workflow: use a release publisher's appropriately scoped GitHub App/user token
when automating publication. Do not grant this upload token to pull request code.

The scanner runs trusted code from the default branch. Keep the workflow, script,
tests and measurement wiring in main and dev when preparing releases. It scans
the already-published assets; it never rebuilds or modifies the executables.

## Scope and published evidence

- Public release `.exe`, `.apk` and `.flatpak` assets, verified against GitHub's
  SHA-256 digest and recorded size before submission. Assets need simple names
  containing letters, digits, dots, hyphens or underscores. No private releases.
- Each Flatpak's `files/bin/evoparseq-linux` ELF is extracted using a temporary
  OSTree repository and scanned separately. It is never installed or executed.
  External Flatpak runtimes and GPU-driver extensions are outside this report.
- ZIP distribution wrappers are not submitted separately. Publish the raw EXE
  and APK alongside their wrappers, as in the existing release format.
- Duplicate content within a release is submitted once, with each filename shown.
- `SECURITY-SCAN.md` and `SECURITY-SCAN.json` are attached to the release. A marked
  section is inserted into its notes without replacing the existing description.
  The JSON records hashes, dates, analysis IDs, per-engine findings and failures;
  linked VirusTotal pages may subsequently change, unlike this report snapshot.

Requests are serialized and spaced at least 16 seconds apart. HTTP 429 has bounded
backoff. Each analysis has a 10-minute polling deadline and the job a 60-minute
limit. Large-file upload URLs support files up to 650 MiB. Quota exhaustion or
service failures are errors, never successful clean results. Other users of the
same API key can still consume its quota.

Detections, suspicious verdicts, missing supported verdicts and failed file
analyses make the job fail **after reporting the available results**. No engine
is silently ignored. Unsupported engines, timeouts and failures have separate
columns; `undetected` is not relabeled `harmless`. A red scan job does not unpublish
an existing release, delete assets or automatically classify a finding as false
positive. If assets change during scanning, stale results are not published.

## Privacy and limits

VirusTotal submissions can be shared with its partners and customers. Only public
release binaries are authorized: never profiles, tokens, signing keys, logs or
memory dumps. The public API's non-commercial use terms and quota apply.
No-detection results are not a safety guarantee and do not override Google Play
Protect, SmartScreen, or vendor-specific decisions. Investigate detections and
use the vendor's review process where appropriate.

Offline tests use mocked subprocesses and synthetic files; they never contact
VirusTotal or publish a release. Python execution coverage is included in Sonar.

References: [VirusTotal API](https://docs.virustotal.com/docs/api-overview),
[uploads](https://docs.virustotal.com/reference/files-scan),
[analysis statuses](https://docs.virustotal.com/reference/analyses-object),
[sharing](https://docs.virustotal.com/docs/how-it-works),
[GitHub workflow triggers](https://docs.github.com/en/actions/how-tos/writing-workflows/choosing-when-your-workflow-runs/triggering-a-workflow).
