# Optional build-time SDK. Nothing is installed and proprietary SDK files are
# not committed. The app separately installs the runtime only with startup consent.
param([Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference = 'Stop'
$archive = Join-Path $Destination 'RTX_Video_SDK_v1.1.0.zip'
$sdk = Join-Path $Destination 'sdk'
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$expected = 'ABF4F34E2B5A618E355B0D5A0365D8ECC3DB4396E756E4C850A867E1AE2ED69E'
if (!(Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) {
    # Invoke-WebRequest follows HTTPS redirects automatically; the final 200
    # response generally does not contain a Location header.
    Invoke-WebRequest -Uri 'https://api.ngc.nvidia.com/v2/models/nvidia/multimedia/dlpp/versions/1.5/files/RTX_Video_SDK_v1.1.0.zip' -OutFile $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw 'RTX Video SDK checksum mismatch' }
Expand-Archive -LiteralPath $archive -DestinationPath $sdk -Force
$env:EVOPARSEQ_RTX_VIDEO_SDK = (Resolve-Path -LiteralPath $sdk).Path
if ($env:GITHUB_ENV) { "EVOPARSEQ_RTX_VIDEO_SDK=$env:EVOPARSEQ_RTX_VIDEO_SDK" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append }
