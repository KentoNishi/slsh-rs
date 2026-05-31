param(
    [string]$SlshExe = "",
    [string]$HostName = "wsl"
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$source = Join-Path $root "scripts\windows_terminal_smoke\WindowsTerminalSmoke.cs"
$outDir = Join-Path $root "target\windows-terminal-smoke"
$exe = Join-Path $outDir "WindowsTerminalSmoke.exe"
$csc = Join-Path $env:WINDIR "Microsoft.NET\Framework64\v4.0.30319\csc.exe"

if (!(Test-Path $csc)) {
    throw "missing .NET Framework C# compiler: $csc"
}

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
& $csc /nologo "/out:$exe" $source
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if (!$SlshExe) {
    $SlshExe = Join-Path $root "target\windows-conpty-smoke\slsh.exe"
}
if (!(Test-Path $SlshExe)) {
    throw "missing slsh.exe: $SlshExe"
}

$stamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$result = Join-Path $env:TEMP "slsh-wt-result-$stamp.txt"
$archiveResult = Join-Path $outDir "result-$stamp.txt"
$wt = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps\wt.exe"
if (!(Test-Path $wt)) {
    throw "missing Windows Terminal launcher: $wt"
}

if (Test-Path $result) {
    Remove-Item -Force $result
}

& $wt new-tab --title "SLSH-WT-SMOKE-$stamp" $exe $SlshExe $HostName $result

for ($i = 0; $i -lt 90; $i++) {
    if (Test-Path $result) {
        Copy-Item -Force $result $archiveResult
        $text = Get-Content -Raw $result
        Write-Output $text
        if ($text.StartsWith("PASS")) {
            exit 0
        }
        exit 1
    }
    Start-Sleep -Milliseconds 500
}

throw "timed out waiting for Windows Terminal smoke result: $result"
