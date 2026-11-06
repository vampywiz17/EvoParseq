BeforeAll {
    $script:packageRoot = Split-Path -Parent $PSScriptRoot
    $script:packageScript = Join-Path $packageRoot 'scripts/package-native.ps1'
}

Describe 'Native release packaging' {
    It 'packages the real GUI executable, preserves its bytes and excludes user settings' {
        & $packageScript
        $version = (Get-Content (Join-Path $packageRoot 'VERSION') -Raw).Trim()
        $expected = (Get-FileHash (Join-Path $packageRoot 'EvoParseq.exe') -Algorithm SHA256).Hash
        foreach ($product in @('EvoParseq', 'ParsecWebTurn')) {
            $file = Join-Path $packageRoot "dist/$product-v$version-win64.zip"
            Test-Path -LiteralPath $file | Should -BeTrue
            $zip = [IO.Compression.ZipFile]::OpenRead($file)
            try {
                $executables = @($zip.Entries | Where-Object Name -like '*.exe')
                $executables.Count | Should -Be 1
                $executables[0].Name | Should -Be "$product.exe"
                @($zip.Entries | Where-Object Name -eq 'settings.json').Count | Should -Be 0
                @($zip.Entries | Where-Object Name -eq 'MIGRATION.md').Count | Should -Be 1
                $stream = $executables[0].Open()
                try {
                    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)) | Should -Be $expected
                } finally { $stream.Dispose() }
            } finally { $zip.Dispose() }
        }
    }

    It 'rejects an invalid package version before touching executable data' {
        Mock Get-Content { 'not-a-version' } -ParameterFilter { $Path -eq 'VERSION' }
        { & $packageScript } | Should -Throw '*Invalid VERSION*'
    }

    It 'rejects a version that differs from the executable manifest' {
        Mock Get-Content { '0.0.0' } -ParameterFilter { $Path -eq 'VERSION' }
        { & $packageScript } | Should -Throw '*Native version mismatch*'
    }
}
