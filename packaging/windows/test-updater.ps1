# Exercise the embedded helper with disposable executables, no network or real install.
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ('tt-pane-updater-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$helper = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../src/update_windows.ps1'))
$powershell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert($condition, $message) { if (-not $condition) { throw $message } }
function Wait-File($path, $process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not (Test-Path -LiteralPath $path)) {
        if ($process.HasExited) { throw "Helper exited before writing $path" }
        if ([DateTime]::UtcNow -gt $deadline) { throw "Timed out waiting for $path" }
        Start-Sleep -Milliseconds 50
    }
}

try {
    $fixture = Join-Path $root 'fixture.exe'
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Reflection;
public class UpdateFixture {
    public static int Main(string[] args) {
        string exe = Assembly.GetExecutingAssembly().Location;
        if (Path.GetFileName(exe) == "setup.exe") {
            File.WriteAllLines(exe + ".args", args);
            string dir = Array.Find(args, a => a.StartsWith("/DIR=" )).Substring(5);
            File.Copy(Path.Combine(Path.GetDirectoryName(exe), "payload.exe"), Path.Combine(dir, "tt-pane.exe"), true);
        } else {
            File.WriteAllLines(exe + ".launched", args);
        }
        return 0;
    }
}
'@ -OutputAssembly $fixture -OutputType ConsoleApplication

    foreach ($mode in @('portable', 'installed', 'invalid-archive')) {
        $caseRoot = Join-Path $root $mode
        $destination = Join-Path $caseRoot 'application with spaces'
        $stage = Join-Path $caseRoot 'stage'
        New-Item -ItemType Directory -Path $destination, $stage | Out-Null
        $exe = Join-Path $destination 'tt-pane.exe'
        [IO.File]::WriteAllText($exe, 'original executable sentinel')
        $config = Join-Path $caseRoot ("settings with spaces [test] ' " + [char]0x00e9 + '.toml')
        [IO.File]::WriteAllText($config, 'configuration sentinel')
        Copy-Item -LiteralPath $helper -Destination (Join-Path $stage 'update.ps1')
        $bundle = 'tt-pane-v99.0.0-windows-x64'
        if ($mode -eq 'installed') {
            $asset = Join-Path $stage 'setup.exe'
            Copy-Item -LiteralPath $fixture -Destination $asset
            Copy-Item -LiteralPath $fixture -Destination (Join-Path $stage 'payload.exe')
        } else {
            $archiveRoot = Join-Path $caseRoot 'archive'
            $bundleDir = Join-Path $archiveRoot $bundle
            New-Item -ItemType Directory -Path $bundleDir | Out-Null
            $filename = if ($mode -eq 'invalid-archive') { 'wrong.exe' } else { 'tt-pane.exe' }
            Copy-Item -LiteralPath $fixture -Destination (Join-Path $bundleDir $filename)
            $asset = Join-Path $stage 'release.zip'
            [IO.Compression.ZipFile]::CreateFromDirectory($archiveRoot, $asset)
        }
        $parent = Start-Process -FilePath $powershell -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 90') -WindowStyle Hidden -PassThru
        $worker = $null
        try {
            @{ pid = $parent.Id; exe = $exe; config = $config; asset = $asset; installed = ($mode -eq 'installed'); bundle = $bundle } |
                ConvertTo-Json | Set-Content -LiteralPath (Join-Path $stage 'job.json') -Encoding UTF8
            $worker = Start-Process -FilePath $powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', ('"' + (Join-Path $stage 'update.ps1') + '"')) -WindowStyle Hidden -PassThru
            if ($mode -eq 'invalid-archive') {
                Assert ($worker.WaitForExit(20000)) 'Invalid archive helper hung'
                Assert ($worker.ExitCode -ne 0) 'Invalid archive was accepted'
                Assert (-not (Test-Path -LiteralPath (Join-Path $stage 'ready'))) 'Invalid archive signaled readiness'
                Assert ([IO.File]::ReadAllText($exe) -eq 'original executable sentinel') 'Invalid archive modified the app'
            } else {
                Wait-File (Join-Path $stage 'ready') $worker
                Assert ([IO.File]::ReadAllText($exe) -eq 'original executable sentinel') 'App was modified before parent exit'
                Stop-Process -Id $parent.Id
                Assert ($worker.WaitForExit(20000)) 'Update helper hung'
                Assert ($worker.ExitCode -eq 0) ('Helper failed: ' + (Get-Content -LiteralPath (Join-Path $stage 'error.txt') -ErrorAction SilentlyContinue))
                Wait-File ($exe + '.launched') (Get-Process -Id $PID)
                $arguments = @(Get-Content -LiteralPath ($exe + '.launched') -Encoding UTF8)
                Assert ($arguments.Count -eq 2 -and $arguments[0] -eq '--config' -and $arguments[1] -eq $config) 'Restart lost config path or quoting'
                Assert ((Get-FileHash -LiteralPath $exe).Hash -eq (Get-FileHash -LiteralPath $fixture).Hash) 'Updated binary differs from payload'
                Assert (@(Get-ChildItem -LiteralPath $destination -Filter '.tt-pane-*').Count -eq 0) 'Portable staging/backup was not cleaned up'
                if ($mode -eq 'installed') {
                    $setupArgs = @(Get-Content -LiteralPath ($asset + '.args'))
                    Assert ($setupArgs -contains '/NORESTART') 'Installer may restart Windows'
                    Assert ($setupArgs -contains ('/DIR=' + $destination)) 'Installer destination changed'
                }
            }
            Assert ([IO.File]::ReadAllText($config) -eq 'configuration sentinel') 'Configuration was changed'
            Write-Host "PASS: $mode"
        } finally {
            foreach ($process in @($parent, $worker)) {
                if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -ErrorAction SilentlyContinue }
            }
        }
    }
} finally {
    $resolved = [IO.Path]::GetFullPath($root)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if ($resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved).StartsWith('tt-pane-updater-test-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
