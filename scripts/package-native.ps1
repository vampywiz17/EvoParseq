param()
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location $taskRoot
try {
    $taskVersion = (Get-Content VERSION -Raw).Trim()
    if ($taskVersion -notmatch '^\d+\.\d+\.\d+(?:-dev\.[1-9]\d*)?$') { throw 'Invalid VERSION' }
    $taskManifestVersion = [regex]::Match((Get-Content src-native/Cargo.toml -Raw), '(?m)^version = "([^"]+)"\r?$').Groups[1].Value
    if ($taskManifestVersion -ne $taskVersion) { throw 'Native version mismatch' }
    $taskBinary = [IO.File]::ReadAllBytes((Join-Path $taskRoot 'src-native/target/x86_64-pc-windows-msvc/release/evoparseq.exe'))
    $taskPeOffset = [BitConverter]::ToInt32($taskBinary, 0x3c)
    if ([BitConverter]::ToUInt16($taskBinary, $taskPeOffset + 24 + 68) -ne 2) { throw 'Only the normal Windows GUI executable may be packaged' }
    # Read the documented PE32+ import table, rather than searching incidental strings.
    $taskSectionTable = $taskPeOffset + 24 + [BitConverter]::ToUInt16($taskBinary, $taskPeOffset + 20)
    $taskSectionCount = [BitConverter]::ToUInt16($taskBinary, $taskPeOffset + 6)
    function Convert-TaskRva([uint32]$TaskRva) {
        for ($taskIndex = 0; $taskIndex -lt $taskSectionCount; $taskIndex++) {
            $taskSection = $taskSectionTable + 40 * $taskIndex
            $taskStart = [BitConverter]::ToUInt32($taskBinary, $taskSection + 12)
            $taskSize = [Math]::Max([BitConverter]::ToUInt32($taskBinary, $taskSection + 8), [BitConverter]::ToUInt32($taskBinary, $taskSection + 16))
            if ($TaskRva -ge $taskStart -and $TaskRva -lt $taskStart + $taskSize) {
                return [int]([BitConverter]::ToUInt32($taskBinary, $taskSection + 20) + $TaskRva - $taskStart)
            }
        }
        throw 'Invalid executable import address'
    }
    if ([BitConverter]::ToUInt16($taskBinary, $taskPeOffset + 24) -ne 0x20b) { throw 'Expected a Windows x64 executable' }
    $taskImport = Convert-TaskRva ([BitConverter]::ToUInt32($taskBinary, $taskPeOffset + 24 + 112 + 8))
    while (($taskNameRva = [BitConverter]::ToUInt32($taskBinary, $taskImport + 12)) -ne 0) {
        $taskNameOffset = Convert-TaskRva $taskNameRva
        $taskNameEnd = $taskNameOffset
        while ($taskBinary[$taskNameEnd] -ne 0) { $taskNameEnd++ }
        $taskDll = [Text.Encoding]::ASCII.GetString($taskBinary, $taskNameOffset, $taskNameEnd - $taskNameOffset)
        if ($taskDll -match '^(vcruntime|msvcp|libopus|WebView2Loader).*\.dll$') { throw "External application runtime dependency: $taskDll" }
        $taskImport += 20
    }
    $taskBinaryText = [Text.Encoding]::Latin1.GetString($taskBinary)
    foreach ($taskMarker in @('video-hardware-probe', 'login-audit', 'synthetic-session-value', 'private-password-token-url', 'PARSEC_BUD_DIAGNOSTICS', 'cursor-visibility-trace.json', 'cursor-post-turn-output-flush', 'hid_button_report')) {
        if ($taskBinaryText.Contains($taskMarker)) { throw "Diagnostic marker in normal executable: $taskMarker" }
    }
    Copy-Item src-native/target/x86_64-pc-windows-msvc/release/evoparseq.exe EvoParseq.exe -Force
    $taskStaging = Join-Path $taskRoot ('dist/evoparseq-package-' + [guid]::NewGuid().ToString('N'))
    $taskPackageName = "EvoParseq-v$taskVersion-win64"
    $taskPackage = Join-Path $taskStaging $taskPackageName
    New-Item -ItemType Directory -Path $taskPackage -Force | Out-Null
    foreach ($taskFile in @('EvoParseq.exe','README.md','LICENSE','CHANGELOG.md','MIGRATION.md')) { Copy-Item -LiteralPath $taskFile -Destination $taskPackage }
    if ($env:EVOPARSEQ_RTX_VIDEO_SDK) {
        Copy-Item (Join-Path $env:EVOPARSEQ_RTX_VIDEO_SDK 'NVIDIA_RTX_Video_SDK_License.pdf') (Join-Path $taskPackage 'NVIDIA-RTX-VIDEO-LICENSE.pdf')
        'Optional NVIDIA RTX Video SDK 1.1.0: NVIDIA RTX technology. Its optional runtime and linked SDK library are proprietary NVIDIA components, excluded from the EvoParseq MIT license. See NVIDIA-RTX-VIDEO-LICENSE.pdf. VSR is off by default and may increase latency, GPU load and memory use. The runtime is downloaded only with consent, checked against pinned archive and DLL SHA-256 values, and stored under %LOCALAPPDATA%/EvoParseq/vsr/1.1.0 without administrator rights. No runtime DLL is embedded in the EXE.' | Set-Content (Join-Path $taskPackage 'NVIDIA-RTX-VIDEO-NOTICE.txt') -Encoding utf8
    }
    Copy-Item src-native/vendor/webrtc-util-0.12.0/LICENSE-MIT (Join-Path $taskPackage 'WEBRTC-UTIL-LICENSE-MIT.txt')
    Copy-Item src-native/vendor/webrtc-util-0.12.0/LICENSE-APACHE (Join-Path $taskPackage 'WEBRTC-UTIL-LICENSE-APACHE.txt')
    Copy-Item src-native/vendor/webrtc-util-0.12.0/LOCAL-CHANGES.md (Join-Path $taskPackage 'WEBRTC-UTIL-LOCAL-CHANGES.md')
    Copy-Item src-native/vendor/webrtc-0.14.0/LICENSE-MIT (Join-Path $taskPackage 'WEBRTC-LICENSE-MIT.txt')
    Copy-Item src-native/vendor/webrtc-0.14.0/LICENSE-APACHE (Join-Path $taskPackage 'WEBRTC-LICENSE-APACHE.txt')
    Copy-Item src-native/vendor/dtls-0.13.0/LICENSE-MIT (Join-Path $taskPackage 'DTLS-LICENSE-MIT.txt')
    Copy-Item src-native/vendor/dtls-0.13.0/LICENSE-APACHE (Join-Path $taskPackage 'DTLS-LICENSE-APACHE.txt')
    Copy-Item src-native/vendor/dtls-0.13.0/EVOPARSEQ.md (Join-Path $taskPackage 'DTLS-LOCAL-CHANGES.md')
    Copy-Item src-native/vendor/webrtc-0.14.0/LOCAL-CHANGES.md (Join-Path $taskPackage 'WEBRTC-LOCAL-CHANGES.md')
    Copy-Item src-native/vendor/webrtc-ice-0.14.0/LOCAL-CHANGES.md (Join-Path $taskPackage 'ICE-LOCAL-CHANGES.md')
    Copy-Item src-native/vendor/turn-0.11.0/LOCAL-CHANGES.md (Join-Path $taskPackage 'TURN-LOCAL-CHANGES.md')
    Copy-Item src-native/crates/bud/vendor/lowlat-client/LICENSE (Join-Path $taskPackage 'LOWLAT-LICENSE.txt')
    Copy-Item src-native/crates/bud/vendor/lowlat-client/EVOPARSEQ.md (Join-Path $taskPackage 'BUD-LOCAL-CHANGES.md')
    Copy-Item src-native/crates/bud/vendor/lowlat-net/EVOPARSEQ.md (Join-Path $taskPackage 'BUD-NET-LOCAL-CHANGES.md')
    Copy-Item src-native/crates/video/LICENSE-lowlat (Join-Path $taskPackage 'GPU-LOWLAT-LICENSE.txt')
    $taskCargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $taskCell = Get-ChildItem -Path (Join-Path $taskCargoHome 'registry/src/*/self_cell-1.3.0') -Directory | Select-Object -First 1
    if (!$taskCell) { throw 'GPU lifetime helper license directory missing' }
    Copy-Item (Join-Path $taskCell.FullName 'LICENSE-APACHE') (Join-Path $taskPackage 'SELF-CELL-LICENSE-APACHE.txt')
    $taskOpus = Get-ChildItem -Path (Join-Path $taskCargoHome 'registry/src/*/libopus_sys-0.3.3') -Directory | Select-Object -First 1
    if (!$taskOpus) { throw 'Bundled Opus license directory missing' }
    Copy-Item (Join-Path $taskOpus.FullName 'LICENSE') (Join-Path $taskPackage 'LIBOPUS-SYS-LICENSE.txt')
    Copy-Item (Join-Path $taskOpus.FullName 'opus/COPYING') (Join-Path $taskPackage 'OPUS-LICENSE.txt')
    if ((Get-ChildItem -LiteralPath $taskPackage -Filter '*.exe').Count -ne 1) { throw 'Single executable package violated' }
    $taskZip = "dist/$taskPackageName.zip"
    Compress-Archive -LiteralPath $taskPackage -DestinationPath $taskZip -Force
    # Legacy ParsecWebTurn updaters recognize this exact asset name. The ZIP
    # carries the identical native executable under the existing shortcut name.
    $taskTransitionName = "ParsecWebTurn-v$taskVersion-win64"
    $taskTransition = Join-Path $taskStaging $taskTransitionName
    New-Item -ItemType Directory -Path $taskTransition | Out-Null
    Get-ChildItem -LiteralPath $taskPackage -File | Where-Object Extension -ne '.exe' | Copy-Item -Destination $taskTransition
    Copy-Item -LiteralPath (Join-Path $taskPackage 'EvoParseq.exe') -Destination (Join-Path $taskTransition 'ParsecWebTurn.exe')
    if ((Get-ChildItem -LiteralPath $taskTransition -Filter '*.exe').Count -ne 1) { throw 'Single executable transition package violated' }
    if ((Get-FileHash (Join-Path $taskTransition 'ParsecWebTurn.exe')).Hash -ne (Get-FileHash EvoParseq.exe).Hash) { throw 'Transition executable differs from native build' }
    $taskTransitionZip = "dist/$taskTransitionName.zip"
    Compress-Archive -LiteralPath $taskTransition -DestinationPath $taskTransitionZip -Force
    @(
        "$((Get-FileHash EvoParseq.exe -Algorithm SHA256).Hash.ToLowerInvariant())  EvoParseq.exe"
        "$((Get-FileHash $taskZip -Algorithm SHA256).Hash.ToLowerInvariant())  $taskPackageName.zip"
        "$((Get-FileHash $taskTransitionZip -Algorithm SHA256).Hash.ToLowerInvariant())  $taskTransitionName.zip"
    ) | Set-Content dist/SHA256SUMS.txt -Encoding ascii
} finally { Pop-Location }
