# Automatic fix criteria

Follow AGENTS.md at all times. Apply the approval requirements below before the automatic-fix allowlist. A compilation, lint or CI failure does not override an approval requirement.

## Automatically fix

Apply small, localized fixes whose intended correction is unambiguous and preserves existing behavior:

- Mechanical compilation errors, such as a missing import or an obvious name typo.
- Formatting and behavior-preserving Clippy violations.
- Unused imports and variables whose removal preserves side effects and resource lifetimes.
- Straightforward static-analysis findings with a local, behavior-preserving correction.
- Simple code defects reported by CI that meet the same criteria.

## Require manual approval

Leave the finding open and propose a correction for review whenever a fix changes or affects:

- Protocol implementation, serialization, signaling or compatibility.
- WebRTC, ICE, STUN or TURN behavior, connection policy, routing, retries or transport timing.
- Unsafe Rust, memory ownership, allocation, resource lifetimes or memory management.
- Concurrency, synchronization, ordering, queues, threads or cancellation.
- Platform-specific APIs, media pipelines, decoded-frame handling or input capture.
- Dependencies, dependency features, manifests or lockfiles.
- Architecture, shared-core boundaries, public interfaces or application behavior.
- Authentication, authorization, credentials, secrets, security controls or privacy.
- CI workflows, permissions, release, signing or deployment configuration.
- Agent instructions, automation permissions or these automatic-fix criteria.

If the correction is ambiguous or crosses one of these boundaries, require manual approval even when the diff is small.

## Preserve verification

Never weaken, remove, skip or mark tests ignored to make CI pass. Never suppress warnings, lower the 80% new-code coverage threshold, exclude production code from analysis, bypass security checks or disable required checks to obtain a passing result.

These criteria select automatic code-review fixes. Gitar documents that CI fixes, CI recovery, rule actions and explicit fix requests operate independently of this selector. The AGENTS.md approval requirements apply to those actions too.
