# Updates and migration to EvoParseq

EvoParseq is an independent native Rust client for compatible, unmodified Parsec
hosts. It replaces the ParsecWebTurn client runtime; no Parsec WebAssembly core
is bundled or checked for updates.

## Existing ParsecWebTurn users

1. Close ParsecWebTurn. Keep your existing `settings.json` in its folder. A backup
   of the folder is recommended before replacing an application.
2. Extract the **transition ZIP** `ParsecWebTurn-v<VERSION>-win64.zip`, and replace
   the old `ParsecWebTurn.exe` with the file from that ZIP. This is the same
   EvoParseq executable, named for existing shortcuts. Alternatively, extract the
   normal EvoParseq ZIP beside your settings and launch `EvoParseq.exe`.
3. Run only one client at a time. Existing network, video, audio and input
   preferences and Windows-user-protected TURN credentials remain usable.
4. Sign in again, including 2FA if enabled. The old web-core login is not imported.
   Remembered native sessions use Windows CurrentUser protection.

Neither package contains `settings.json`; neither deletes old files or changes
host configuration. Old web-core files are no longer needed by EvoParseq.
Keep backups until you have verified the new client.

## Update window

**Help > Check for updates** checks public stable release metadata. It opens a
validated EvoParseq Windows ZIP in your browser; installation is manual, as in
ParsecWebTurn. Close the application before replacing its executable, keeping
`settings.json` beside it. No core-version check or background core download runs.
Only newer stable EvoParseq packages are offered. Development builds do not
receive older stable releases as updates.

## Publishing a transition (maintainers)

The native packaging script creates both the normal EvoParseq ZIP and the
legacy-named transition ZIP, with identical executable hashes and license notices.
Publish both on the repository's same **strictly newer stable release** for the
existing ParsecWebTurn updater to recognize the transition. For example, clients
on stable 0.9.4 require a stable version greater than 0.9.4. A development CI
artifact or prerelease is not offered by that stable updater.

## Stable v0.9.5 and repository rename

The repository is now **vampywiz17/EvoParseq**. Main contains stable EvoParseq
v0.9.5; dev continues Windows and Android TV development.

Older ParsecWebTurn and EvoParseq previews validate download URLs against the old
repository name. GitHub's repository redirect does not update that compiled-in
allowlist, so their update check can reject the renamed repository's assets.
For this transition, download the stable ZIP directly from
[the release page](https://github.com/vampywiz17/EvoParseq/releases/tag/v0.9.5).
The transition ZIP preserves existing shortcuts; it cannot change an old updater
before that executable is replaced. EvoParseq v0.9.5 checks the new repository.

## Stable v0.9.6: Windows and Android TV

Windows v0.9.5 users can use Help > Check for updates or extract the new Windows
ZIP while retaining settings.json under the same Windows account. The legacy
ParsecWebTurn ZIP carries the identical executable under the old shortcut name.
Passwords and 2FA codes are not saved; protected sessions are account-bound.

Android distribution-signed dev.2-dev.5 builds can update in place to the stable
EvoParseq-TV.apk (versionCode 30). The stable update channel offers v0.9.6;
Include prereleases is not required. Android still confirms installation.
Old debug-signed previews, including v0.9.6-dev.1, cannot use the distribution
signature in place: uninstall once and install the signed APK. Uninstalling
erases private settings and remembered sign-in. Windows packages and Android
APKs are separate; Linux alpha and macOS work are not part of this release.
