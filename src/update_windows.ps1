# Embedded in tt-pane, staged locally, and run by Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($PSScriptRoot)
$job = $null
$exited = $false
$backup = $null
$replaced = $false
$success = $false

function Start-App {
    # Start-Process joins ArgumentList; Windows filenames cannot contain quotes.
    Start-Process -FilePath $job.exe -ArgumentList @('--config', ('"' + $job.config + '"')) -WindowStyle Hidden
}

try {
    $job = Get-Content -LiteralPath (Join-Path $root 'job.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $parentProcess = Get-Process -Id $job.pid -ErrorAction Stop
    $destination = Split-Path -Parent $job.exe
    $staged = Join-Path $root 'tt-pane.exe'
    if (-not $job.installed) {
        # Extract only the expected executable; never follow paths from ZIP entries.
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [IO.Compression.ZipFile]::OpenRead($job.asset)
        try {
            $entryName = $job.bundle + '/tt-pane.exe'
            $entries = @($zip.Entries | Where-Object { $_.FullName.Replace('\', '/') -eq $entryName })
            if ($entries.Count -ne 1) { throw 'The portable archive does not contain exactly one tt-pane.exe' }
            if ($entries[0].Length -gt 536870912) { throw 'The unpacked executable exceeds the size limit' }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entries[0], $staged)
        } finally { $zip.Dispose() }
    }
    [IO.File]::WriteAllText((Join-Path $root 'ready'), 'ready')
    if (-not $parentProcess.WaitForExit(60000)) { throw 'The running application did not exit; update canceled' }
    $exited = $true
    if ($job.installed) {
        $log = Join-Path $root 'installer.log'
        $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOCLOSEAPPLICATIONS', '/NORESTARTAPPLICATIONS', ('/DIR="' + $destination + '"'), ('/LOG="' + $log + '"'))
        $setup = Start-Process -FilePath $job.asset -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
        if ($setup.ExitCode -ne 0) { throw "Installer failed with exit code $($setup.ExitCode). See $log" }
    } else {
        # Copy to the destination volume first; keep a unique backup until launch.
        $replacement = Join-Path $destination ('.tt-pane-new-' + [Guid]::NewGuid().ToString('N') + '.exe')
        $backup = Join-Path $destination ('.tt-pane-backup-' + [Guid]::NewGuid().ToString('N') + '.exe')
        try {
            Copy-Item -LiteralPath $staged -Destination $replacement
            [IO.File]::Replace($replacement, $job.exe, $backup)
            $replaced = $true
        } finally {
            if (Test-Path -LiteralPath $replacement) { Remove-Item -LiteralPath $replacement -Force }
        }
    }
    Start-App
    $success = $true
    if ($backup -and (Test-Path -LiteralPath $backup)) { Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue }
} catch {
    $message = $_.Exception.Message
    if ($replaced -and $backup -and (Test-Path -LiteralPath $backup)) {
        try { [IO.File]::Replace($backup, $job.exe, $null) } catch { $message += "`nRestore failed: $($_.Exception.Message)" }
    }
    if ($exited) {
        try { Start-App } catch { $message += "`nRestart failed: $($_.Exception.Message)" }
    }
    [IO.File]::WriteAllText((Join-Path $root 'error.txt'), $message)
    # Before readiness the live application reports the error. After exit there
    # is no UI left, so surface it using a native dialog and retain diagnostics.
    if ($exited) {
        Add-Type -AssemblyName System.Windows.Forms
        [void][System.Windows.Forms.MessageBox]::Show("$message`n`nDiagnostics: $root", 'tt-pane update failed')
    }
    exit 1
} finally {
    if ($success) {
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        if ($root.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
            [IO.Path]::GetFileName($root).StartsWith('tt-pane-update-')) {
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
