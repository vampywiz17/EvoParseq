# Release malware scanning

`Release malware scan` has a blocking draft-publication gate and a published-release
audit. Pushes, pull requests, draft creation and ordinary build artifacts do not
upload anything. The explicit gate is started only for a prepared release.
The workflow uses the repository secret `VIRUSTOTAL`; its value must never be put
in source, logs, reports or command arguments. Only the scanner step receives it.

Prepare releases as drafts, upload **all final assets** (including the signed APK),
then start this workflow on main with the draft's numeric `release_id` input.
It scans and attaches evidence while the release remains a draft. Any malicious
or suspicious verdict immediately stops further submissions, fails the job and
leaves the release unpublished. Incomplete/error/no-supported-verdict results
also block publication. Only successful complete scans allow the workflow to
publish the draft and mark a stable release latest. No ignore list or override
input exists. The workflow's `GITHUB_TOKEN` publication does not retrigger the
published-release audit, avoiding duplicate submissions.

Do not manually publish first when a prepublication guarantee is required. A
release published directly through the GitHub UI/user token triggers an audit
after publication; that audit can fail but cannot prevent an earlier manual
publication. An asset added after publication requires rerunning its audit job.

The scanner runs trusted code from the default branch. Keep the workflow, script,
tests and measurement wiring in main and dev when preparing releases. It scans
the already-published assets; it never rebuilds or modifies the executables.

## Scope and published evidence

- Intended-public release `.exe`, `.apk` and `.flatpak` assets, verified against GitHub's
  SHA-256 digest and recorded size before submission. Assets need simple names
  containing letters, digits, dots, hyphens or underscores. No private repositories.
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
analyses make the job fail **after reporting the available results**. Remaining
files are marked not scanned after a blocking result. No engine
is silently ignored. Unsupported engines, timeouts and failures have separate
columns; `undetected` is not relabeled `harmless`. A red scan job never publishes
a draft. It does not unpublish an existing release, delete assets or automatically classify a finding as false
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
