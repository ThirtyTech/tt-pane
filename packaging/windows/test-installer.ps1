# End-to-end test of the real installer script with an isolated AppId and
# shortcut names. Never installs over or uninstalls the user's tt-pane instance.
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $BinDir,
    [Parameter(Mandatory)] [string] $Tag,
    [string] $Iscc = $env:ISCC
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path "$PSScriptRoot/../..").Path
if ($Tag -notmatch '^v(\d+\.\d+\.\d+)(-[0-9A-Za-z.-]+)?$') { throw "Invalid tag: $Tag" }
$fileVersion = $Matches[1]
$version = $Tag.Substring(1)
if (-not $Iscc) {
    $Iscc = @(
        "$env:LOCALAPPDATA/Programs/Inno Setup 7/ISCC.exe",
        "$env:LOCALAPPDATA/Programs/Inno Setup 6/ISCC.exe",
        "${env:ProgramFiles(x86)}/Inno Setup 7/ISCC.exe",
        "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe"
    ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $Iscc) { throw 'Inno Setup is required' }
$id = [Guid]::NewGuid().ToString()
$name = "tt-pane installer test $id"
$testRoot = Join-Path $root "target/installer-smoke/$id"
$stage = Join-Path $testRoot 'stage'
$install = Join-Path $testRoot 'installed'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $BinDir 'tt-pane.exe'), "$root/README.md" -Destination $stage
& $Iscc /Qp "/DAppId=$id" "/DShortcutName=$name" "/DAppVersion=$version" "/DFileVersion=$fileVersion" `
    "/DStageDir=$stage" /DOutputBase=smoke-setup "/O$testRoot" "$PSScriptRoot/tt-pane.iss"
if ($LASTEXITCODE -ne 0) { throw 'Smoke installer compilation failed' }
$setup = Join-Path $testRoot 'smoke-setup.exe'
$uninstall = Join-Path $install 'unins000.exe'
$key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${id}_is1"
$desktop = Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) "$name.lnk"
$startup = Join-Path ([Environment]::GetFolderPath('Startup')) "$name.lnk"
$menu = Join-Path ([Environment]::GetFolderPath('Programs')) "$name/$name.lnk"
function Invoke-Setup([string] $Tasks, [string] $LogName) {
    $process = Start-Process -FilePath $setup -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-',
        "/DIR=`"$install`"", "/GROUP=`"$name`"", "/TASKS=`"$Tasks`"", "/LOG=`"$testRoot/$LogName.log`""
    ) -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Setup failed with $($process.ExitCode); see $testRoot" }
}
try {
    # A startup shortcut from an older installer must be removed: tt-pane now
    # registers its own sign-in entry.
    New-Item -ItemType Directory -Force -Path (Split-Path $startup) | Out-Null
    Set-Content -LiteralPath $startup -Value ''
    Invoke-Setup 'desktopicon' 'install'
    if (Test-Path -LiteralPath $startup) { throw 'Install kept the legacy startup shortcut' }
    foreach ($path in $uninstall, $desktop, $menu, $key) {
        if (-not (Test-Path -LiteralPath $path)) { throw "Missing installed item: $path" }
    }
    $shell = New-Object -ComObject WScript.Shell
    foreach ($link in $desktop, $menu) {
        if ($shell.CreateShortcut($link).TargetPath -ne (Join-Path $install 'tt-pane.exe')) { throw "Wrong shortcut target: $link" }
    }
    $actual = & "$install/tt-pane.exe" --version | Out-String
    if ($LASTEXITCODE -ne 0 -or "$actual".Trim() -ne "tt-pane $version") { throw "Wrong installed version: $actual" }
    $actions = & "$install/tt-pane.exe" actions | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $actions.schema_version -ne 1 -or 'left' -notin $actions.actions.id -or 'right' -notin $actions.actions.id) { throw 'Installed CLI smoke test failed' }
    # User-owned files must survive both upgrade and uninstall.
    $sentinel = Join-Path $install 'user-owned.txt'
    Set-Content -LiteralPath $sentinel -Value 'keep me' -NoNewline
    Invoke-Setup '' 'upgrade'
    if (Test-Path -LiteralPath $desktop) { throw 'Upgrade retained deselected shortcuts' }
    if ((Get-ItemProperty -LiteralPath $key).DisplayVersion -ne $version) { throw 'Wrong registered version' }
    $process = Start-Process -FilePath $uninstall -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Uninstall failed: $($process.ExitCode)" }
    foreach ($path in "$install/tt-pane.exe", $menu, $desktop, $startup, $key) {
        if (Test-Path -LiteralPath $path) { throw "Uninstall left behind: $path" }
    }
    if ((Get-Content -LiteralPath $sentinel -Raw) -ne 'keep me') { throw 'User-owned file was changed or removed' }
    Write-Host "PASS: install, CLI, registration, shortcuts, upgrade, deselection, uninstall and user-file preservation. Logs: $testRoot"
} finally {
    if (Test-Path -LiteralPath $uninstall) {
        Start-Process -FilePath $uninstall -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -Wait
    }
}
