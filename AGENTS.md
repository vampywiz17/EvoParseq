# Development requirements

The user's standing requirement is to keep current and future development standards-compliant and maintainable.

- Use W3C/IETF standards for WebRTC, ICE, STUN and TURN behavior. Preserve native WebIDL validation and the client's connection policy.
- Use documented, supported Rust and target-platform APIs for platform functionality (currently Windows; Linux/Android when added). Platform APIs are not automatically web standards; describe that distinction accurately. This branch contains the independent EvoParseq native Rust client.
- Do not introduce production dependencies on private Chromium interfaces, internal browser pages or undocumented switches. The user permits documented vendor diagnostics as an optional extension: keep them disabled by default, clearly label their experimental/vendor status, isolate failures, and retain a standards-based fallback. Never use unstable diagnostic output as a required production contract or a definitive routing proof.
- Missing telemetry is unknown, not negative evidence. Report confirmed direct/TURN routing only when the selected active connection provides sufficient evidence; preserve uncertainty otherwise.
- Keep diagnostic tests isolated from real credentials and profiles. Do not expose production remote debugging ports, weaken browser security/privacy or require administrator rights for ordinary use.
- For each proposed integration, verify its documented support and fallback behavior before implementation. If a required result cannot be established through supported APIs, document the limitation instead of inventing a heuristic presented as certainty.
- For Parsec protocol research, consult `docs/NATIVE_REFERENCES.md`; it records external projects and the limits of applying their findings to this client.

- Native video must use documented codec and graphics APIs. Keep live decoded frames on the GPU, feed decoder textures directly to the presentation/conversion pass, and avoid unnecessary intermediate decoded-frame copies or CPU readback. Necessary color conversion/output writes or documented implementation/interop copies are permitted; explain why they are needed and do not advertise them as zero-copy. Synthetic offline diagnostics may perform explicitly isolated readback; never enable it in live sessions. Do not claim zero-copy or hardware execution beyond what the implementation and telemetry establish.

- Preserve the user's established zero-copy terminology in all future work: the application's Zero copy mode presents original decoder surfaces without an application-side decoded-frame copy or separate color-conversion/post-processing pass. Keeping pixels on the GPU alone is insufficient. Describe paths with GPU conversion, intermediate textures, AI processing or GPU copies explicitly as GPU-resident processing, including their costs; do not relabel them zero-copy because an external project uses a looser definition. Driver/compositor/scanout behavior remains unverified unless supported evidence establishes it. RTX VSR is GPU processing, not the application's Zero copy mode.

- lowlat is authorized only as an explicit alternative. WebRTC remains the default. lowlat integration must not alter the existing WebRTC connection/signaling, ICE/STUN/TURN selection, transport timers, SCTP reliability/ordering or settings behavior. Isolate the lowlat adapter, credentials and lifecycle; use only deliberate, reviewed common interfaces. A lowlat failure must not silently fall back or change the selected transport. Preserve the working WebRTC TURN/TCP and TURN/TLS routes, especially for restricted networks. Validate both transports independently before exposing a usable lowlat option.

## Sonar coverage and implementation design

- At least 80% Sonar coverage on new code is a baseline development requirement. Design each change with testable shared logic and meaningful regression tests from the start, and include those tests in the initial implementation. Plan coverage before coding so meeting the threshold does not routinely require a separate follow-up cycle.
- Verify measured coverage in CI. Preserve the configured threshold and measure actual behavior. Add targeted tests for uncovered behavior instead of lowering thresholds or excluding production code to pass the gate.

## Pull requests and Gitar OSS review

- Keep one open GitHub pull request into `dev` for each active feature branch. Push follow-up changes to the same feature branch.
- Keep an open `dev` → `main` pull request for the next stable release. Merge it only with explicit release approval after the required checks pass. Open the next pull request when new release changes are available.
- Gitar OSS may review changes and apply small, unambiguous fixes within the repository criteria in `.gitar/config/apply.md`. Eligible fixes include mechanical compilation errors, formatting, behavior-preserving Clippy findings, unused imports/variables and straightforward static-analysis or code-related CI findings. Preserve side effects, resource lifetimes and application behavior.
- Manual approval is required for changes affecting protocols; WebRTC/ICE/STUN/TURN; connection policy, routing, retries or transport timing; unsafe Rust or memory/resource management; concurrency or synchronization; platform APIs, media or input capture; dependencies; architecture or public interfaces; authentication, credentials or security; CI/release/deployment configuration; and agent instructions or automation permissions. These boundaries override the small-fix allowlist. Gitar must leave such findings open and propose a correction for review.
- Never weaken or skip tests, suppress warnings, lower the 80% Sonar new-code coverage threshold, exclude production code from analysis, bypass security checks or disable required checks to make CI pass. Codex reviews Gitar commits and verifies the relevant results in GitHub CI.
- Gitar auto-apply requires per-PR opt-in through `gitar-managed` or `gitar auto-apply:on`. Agents may opt eligible feature PRs in once the repository criteria are available to Gitar on the repository's default branch. A criteria change inside a PR does not authorize that PR's own automatic fixes.
- The `apply.md` selector governs automatic review fixes only. CI fixes/recovery, rule actions and explicit fix requests are independent Gitar mechanisms. Apply the same approval boundaries when requesting or reviewing those actions, and use specific, bounded fix requests for eligible CI corrections. Do not treat the selector as a technical approval gate for those mechanisms.
- Keep automatic approval and automatic merging under the existing explicit approval rules. Gitar fixes do not authorize a merge or release.

## UI and platform boundaries for future development

### Desktop GUI parity (standing user requirement)

- Windows is the canonical desktop GUI. Linux and future desktop ports must use the same visible wording, font sizes, control dimensions, spacing, ordering and layout. Put these definitions in shared Rust and have the platform adapters consume them; do not maintain approximate per-platform copies.
- Desktop presentation now includes the full Windows Settings, session overlay, Connection stats and menu skeleton, as explicitly requested by the user. Controls whose behavior is not integrated must remain disabled and unmeasured values must show N/A; never display invented live values or imply that an unwired setting takes effect. Keep diagnostic reports and platform-specific preview copy out of the product GUI.
- Session controls use the Windows translucent, draggable session icon and panel layout. Add individual controls in their Windows positions as their features become available.
- Window decorations, native resize cursors, window management and OS event translation follow the host desktop. These are platform adapters, not exceptions to application content parity. Verify layout, text sizes, centering and cursor transitions when changing desktop UI.
- This desktop rule does not replace the Android TV requirements below: TV presentation follows Android TV design and D-pad navigation while reusing shared application models and policy.

The user requires continuing the incremental separation established on the native-core branch:

- Put application state, parsing/validation rules, option dependencies, persistence decisions and presentation text/units in shared Rust models/controllers. Reuse the existing models; do not duplicate their rules in each UI.
- Keep native widget access, drawing, focus/capture, platform input translation, storage protection and OS/media API operations in explicit adapters/backends. Shared events and commands cross the boundary without an extra event queue or payload-conversion layer.
- Add only interfaces required by implemented behavior. Prefer static dispatch and existing services; do not introduce speculative frameworks, abstractions, SDKs or dependencies just to prepare for another platform.
- Preserve the working decoding/presentation/audio paths, ordering, bounds and wakeups. Architectural separation must not add decoded-frame copies, CPU readback, per-frame work, unnecessary buffers, threads or timers. Keep CPU and memory use low; do not claim a speedup without measurement.
- Preserve and test focus-loss releases, immersive capture, relative camera transitions, HID synchronization and clipboard permissions/echo suppression when modifying these boundaries. Use isolated fixtures and the existing native lifecycle tests, followed by live user validation for hardware/session behavior.
- Review each completed step for duplicated policy, unused helpers and incomplete experiments. Retain necessary protocol validation and platform backends; do not remove them merely because a future port will replace them. Port actual media/storage backends when a concrete target platform requires it.

### Android TV port

- Maximize common Rust source with Windows. Do not reimplement authentication, discovery, protocol, validation, session policy or diagnostics in Kotlin. Reuse the existing Rust models before adding a target adapter; move OS-free rules to shared modules where needed.
- Kotlin/Compose for TV is a thin TV presentation and Android API adapter only. Keep UI/request boundaries outside the video hot path. MediaCodec/Surface, platform audio/input, Android lifecycle and Keystore are legitimate target-specific operations.
- Follow Google Android TV / Google TV design and D-pad navigation, including usable 2FA/IME and settings focus restoration; no Xiaomi-specific skin. First live target: Xiaomi TV Box S (3rd Gen) EU on Android 14. Controller state and protocol are shared Rust; keep Android event/vibration and Windows XInput access in their respective adapters.
- Decoder Surface output must not be advertised as verified zero copy or direct scanout. Preserve the strict project definition and distinguish documented app behavior from unverified driver/compositor work.

- Android TV relay provisioning is strictly local-network-only, including future
  development. No external setup website/service or public listener is wanted.
  Keep setup opt-in, expiring, authenticated, encrypted in transit and at rest,
  with explicit TV approval. Do not treat a PIN as a substitute for authenticating
  the local TLS certificate; preserve the displayed out-of-band fingerprint.

## Agent roles and resource efficiency

The project uses three independent agent roles. Each agent must follow the shared project requirements above and operate within its assigned responsibilities.

### Developer

- Implement features, fix bugs, refactor code and write tests.
- Preserve existing architecture and functionality.
- Review only the code relevant to the requested task.
- Avoid unrelated changes and speculative improvements.
- Run relevant build, lint and test checks before completing implementation.

### Code Reviewer

- Operate independently from the Developer.
- Focus on confirmed bugs, regressions, security, concurrency, resource usage and performance issues.
- Start with the Git diff and expand the analysis only when necessary.
- Verify findings against actual code paths before reporting them.
- Report findings with severity, file, line, impact and suggested correction.
- Avoid cosmetic findings, speculative issues and duplication of existing automated checks.
- Do not modify source code unless explicitly requested.

### Git & Release Manager

- Handle commits, pushes, merges, branches, releases, tags, changelogs and GitHub issues.
- Verify repository status, target branches and relevant CI results before performing operations.
- Do not perform source code analysis, refactoring or feature development unless explicitly requested.
- Avoid repository-wide scans and unnecessary builds.
- Do not repeat successful CI checks locally without a specific reason.

### Context and resource efficiency

- Use the minimum context required to complete the assigned task reliably.
- Prefer targeted file reads, Git diffs and focused searches over full repository scans.
- Do not repeatedly inspect unchanged files or rediscover established project decisions.
- Treat the repository, Git history and current CI results as the authoritative source of project state.
- Keep responses concise and actionable.
- Do not sacrifice correctness, security or required verification to reduce token usage.
- Escalate tasks outside the assigned role rather than silently expanding their scope.
