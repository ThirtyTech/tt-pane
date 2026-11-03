# Installs tt-pane from its GitHub releases on Windows (x64; arm64 through
# x64 emulation) by running the release's setup.exe silently:
#
#   powershell -ExecutionPolicy Bypass -c "irm https://github.com/ThirtyTech/tt-pane/releases/latest/download/install.ps1 | iex"
#
# The setup installs for your account, without administrator rights, into
# %LOCALAPPDATA%\Programs\tt-pane, and adds a Start menu shortcut. A running
# tt-pane is closed for the upgrade, and tt-pane is started afterwards. To
# remove it again:
#
#   $env:TTPANE_UNINSTALL = '1'; irm https://github.com/ThirtyTech/tt-pane/releases/latest/download/install.ps1 | iex
#
# Environment:
#   TTPANE_VERSION    tag to install, such as v0.2.0 (default: latest release)
#   TTPANE_TASKS      setup's optional tasks: desktopicon (desktop shortcut).
#                     Unset, a new install has none and an upgrade keeps the
#                     choice made before; '' turns it off. tt-pane starts at
#                     sign-in by default; turn it off in Settings.
#   TTPANE_NO_LAUNCH  set to 1 to not start tt-pane after installing
#   TTPANE_UNINSTALL  set to 1 to uninstall instead (or pass -Uninstall)

param([switch]$Uninstall)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is very slow with the progress bar on
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = 'ThirtyTech/tt-pane'
# packaging/windows/tt-pane.iss's AppId; Inno Setup registers it with _is1.
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{05CF136A-FC31-43EE-A704-44900F4B94F1}_is1'
$silent = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-')

function Fail($msg) {
    Write-Host "tt-pane install: $msg" -ForegroundColor Red
    throw $msg
}

function Get-Installed {
    Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
}

# Stops tt-pane processes running from $dir and returns whether any were.
# The tray app has no command to quit it, and setup can't replace a running
# exe.
function Stop-TtPane($dir) {
    $running = @(Get-Process tt-pane -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase)
    })
    if ($running.Count -eq 0) { return $false }
    Write-Host 'Closing the running tt-pane'
    $running | Stop-Process -Force
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    return $true
}

$defaultDir = Join-Path $env:LOCALAPPDATA 'Programs\tt-pane'

function Uninstall-TtPane {
    $installed = Get-Installed
    if (-not $installed) { Fail "tt-pane isn't installed for this account" }
    $dir = if ($installed.InstallLocation) { $installed.InstallLocation } else { $defaultDir }
    # UninstallString is the quoted path of unins000.exe.
    $uninstaller = $installed.UninstallString.Trim('"')
    if (-not (Test-Path -LiteralPath $uninstaller)) { Fail "the uninstaller is missing: $uninstaller" }
    [void](Stop-TtPane $dir)
    Write-Host 'Uninstalling tt-pane'
    $process = Start-Process -FilePath $uninstaller -ArgumentList $silent -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { Fail "uninstall failed with exit code $($process.ExitCode)" }
    Write-Host "Removed tt-pane. Your configuration in $env:APPDATA\tt-pane is kept."
}

function Install-TtPane {
    # The OS architecture, not the process's: x64 PowerShell on an ARM machine reports AMD64.
    $osArch = try {
        [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
        if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    }
    switch ($osArch) {
        { $_ -in 'X64', 'AMD64' } { }
        { $_ -in 'Arm64', 'ARM64' } { Write-Host 'Only an x64 build is published; it runs on ARM through Windows x64 emulation.' }
        default { Fail "unsupported architecture: $osArch" }
    }

    if ($env:TTPANE_VERSION) {
        $version = $env:TTPANE_VERSION
    } else {
        $latest = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'tt-pane-installer' }
        $version = $latest.tag_name
        if (-not $version) { Fail "couldn't find the latest release" }
    }

    $name = "tt-pane-$version-windows-x64-setup.exe"
    $base = "https://github.com/$repo/releases/download/$version"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
    New-Item -ItemType Directory $tmp | Out-Null
    # Outside $tmp, so it's still there to read when setup fails.
    $log = Join-Path ([IO.Path]::GetTempPath()) 'tt-pane-install.log'

    try {
        Write-Host "Downloading tt-pane $version (windows-x64)"
        $setup = Join-Path $tmp $name
        try {
            Invoke-WebRequest "$base/$name" -OutFile $setup -UseBasicParsing
            Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile (Join-Path $tmp 'SHA256SUMS.txt') -UseBasicParsing
        } catch {
            Fail "download failed: $($_.Exception.Message)"
        }

        $line = Get-Content (Join-Path $tmp 'SHA256SUMS.txt') | Where-Object { $_ -match " \*?$([regex]::Escape($name))$" }
        if (-not $line) { Fail "no checksum for $name" }
        $expected = ($line -split '\s+')[0]
        # .NET rather than Get-FileHash: its module may not load, such as when
        # PowerShell 7's PSModulePath is inherited.
        $sha = [Security.Cryptography.SHA256]::Create()
        $stream = [IO.File]::OpenRead($setup)
        try { $actual = [BitConverter]::ToString($sha.ComputeHash($stream)) -replace '-', '' } finally { $stream.Dispose(); $sha.Dispose() }
        if ($expected -ne $actual) { Fail "checksum mismatch for $name" }

        $installed = Get-Installed
        $dir = if ($installed -and $installed.InstallLocation) { $installed.InstallLocation } else { $defaultDir }
        [void](Stop-TtPane $dir)

        $arguments = $silent + "/LOG=`"$log`""
        # Without /TASKS, setup uses its defaults on a new install and the
        # previous choices on an upgrade.
        # "startup" was a setup task before tt-pane managed launch at login
        # itself; drop it so older scripts keep working.
        if ($null -ne $env:TTPANE_TASKS) {
            $tasks = ($env:TTPANE_TASKS -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ -and $_ -ne 'startup' }) -join ','
            $arguments += "/TASKS=`"$tasks`""
        }

        Write-Host "Installing tt-pane $version"
        $process = Start-Process -FilePath $setup -ArgumentList $arguments -PassThru -Wait
        if ($process.ExitCode -ne 0) { Fail "setup failed with exit code $($process.ExitCode); see $log" }
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    $installed = Get-Installed
    $dir = if ($installed -and $installed.InstallLocation) { $installed.InstallLocation.TrimEnd('\') } else { $defaultDir }
    $exe = Join-Path $dir 'tt-pane.exe'
    if (-not (Test-Path -LiteralPath $exe)) { Fail "setup finished but $exe is missing; see $log" }
    Write-Host "Installed tt-pane $version to $exe"

    if ($env:TTPANE_NO_LAUNCH -eq '1') {
        Write-Host 'Start tt-pane from the Start menu. It runs in the system tray.'
    } elseif (Get-Process tt-pane -ErrorAction SilentlyContinue) {
        # Another copy, such as the portable ZIP, is running. tt-pane doesn't
        # hand off to a running instance, so don't start a second one.
        Write-Host 'Another tt-pane is already running. Quit it, then start tt-pane from the Start menu.'
    } else {
        Start-Process -FilePath $exe
        Write-Host 'tt-pane is running in the system tray. Open Settings from its tray menu.'
    }
}

# Functions rather than a top-level return, which doesn't stop a script run
# through iex.
if ($Uninstall -or $env:TTPANE_UNINSTALL -eq '1') { Uninstall-TtPane } else { Install-TtPane }
