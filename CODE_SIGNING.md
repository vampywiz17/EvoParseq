# Code signing policy

## SignPath status

EvoParseq is preparing to apply for Windows code signing through SignPath
Foundation. SignPath signing is pending. The currently published Windows
releases do not carry a SignPath Foundation signature.

Once the integration is approved and in use, releases signed through it will
carry this attribution:

> Free code signing provided by SignPath.io, certificate by SignPath Foundation.

## Scope and approvals

The planned scope is the EvoParseq Windows executable built from the public
repository by GitHub Actions. The signing request will be tied to its source
revision and build. The release approver will verify the source, build and
artifact identity before manually approving each signing request. This policy
covers Windows code signing. The Android TV APK uses its own Android signing
identity.

| Role | Responsible |
| --- | --- |
| Committer and reviewer | [Norbert Sütő (@vampywiz17)](https://github.com/vampywiz17), project maintainer |
| Signing approver | [Norbert Sütő (@vampywiz17)](https://github.com/vampywiz17), repository owner |

The maintainer reviews proposed changes through pull requests, including
changes to build and packaging scripts. Access to the signing service and
repository requires multi-factor authentication for signing team members.

See the [official downloads](https://github.com/vampywiz17/EvoParseq/releases)
and the [EvoParseq privacy policy](PRIVACY.md).
