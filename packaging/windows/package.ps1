# Package an existing GUI release build. Requires Inno Setup 6 or newer.
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $BinDir,
    [Parameter(Mandatory)] [string] $Tag,
    [string] $OutputDir = 'dist',
    [string] $Iscc = $env:ISCC
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path "$PSScriptRoot/../..").Path
if ($Tag -notmatch '^v(\d+\.\d+\.\d+)(-[0-9A-Za-z.-]+)?$') { throw "Invalid release tag: $Tag" }
$fileVersion = $Matches[1]
$version = $Tag.Substring(1)
$binary = (Resolve-Path (Join-Path $BinDir 'tt-pane.exe')).Path
# Reject accidentally packaging an ARM64/x86 binary as x64.
$bytes = [IO.File]::ReadAllBytes($binary)
$pe = [BitConverter]::ToInt32($bytes, 0x3c)
if ([BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) { throw 'Expected an x64 tt-pane.exe' }
$reported = & $binary --version | Out-String
if ($LASTEXITCODE -ne 0 -or "$reported".Trim() -ne "tt-pane $version") {
    throw "Binary version '$reported' does not match $Tag"
}
if (-not $Iscc) {
    $candidates = @(
        "$env:LOCALAPPDATA/Programs/Inno Setup 7/ISCC.exe",
        "$env:LOCALAPPDATA/Programs/Inno Setup 6/ISCC.exe",
        "${env:ProgramFiles(x86)}/Inno Setup 7/ISCC.exe",
        "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe"
    )
    $Iscc = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $Iscc -or -not (Test-Path -LiteralPath $Iscc)) {
    throw 'Inno Setup is required. Install it or set ISCC to the compiler path.'
}
$out = [IO.Path]::GetFullPath($OutputDir)
New-Item -ItemType Directory -Force -Path $out | Out-Null
$name = "tt-pane-$Tag-windows-x64"
# Unique staging path, kept under the output directory for predictable cleanup.
$stage = Join-Path $out ('.stage-' + [Guid]::NewGuid().ToString('N'))
$bundle = Join-Path $stage $name
New-Item -ItemType Directory -Path $bundle | Out-Null
try {
    Copy-Item -LiteralPath $binary -Destination $bundle
    Copy-Item -LiteralPath "$root/README.md" -Destination $bundle
    & $Iscc /Qp "/DAppVersion=$version" "/DFileVersion=$fileVersion" "/DStageDir=$bundle" `
        "/DOutputBase=$name-setup" "/O$out" "$PSScriptRoot/tt-pane.iss"
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
    if (-not (Test-Path -LiteralPath "$out/$name-setup.exe")) { throw 'Installer is missing' }
    Compress-Archive -LiteralPath $bundle -DestinationPath "$out/$name.zip" -Force
    $checksums = foreach ($file in "$out/$name.zip", "$out/$name-setup.exe") {
        $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $([IO.Path]::GetFileName($file))"
    }
    $checksums | Out-File -FilePath "$out/SHA256SUMS.txt" -Encoding ascii
} finally {
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if (-not $resolvedStage.StartsWith($out + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Staging directory escaped OutputDir'
    }
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
}
Write-Host "Packages written to $out"
