param(
    [string]$SlshExe = "",
    [string]$HostName = "wsl",
    [switch]$SelfTest,
    [switch]$NanoRow,
    [string]$NestedLinuxSlsh = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$source = Join-Path $root "scripts\windows_conpty_smoke\ConptySmoke.cs"
$outDir = Join-Path $root "target\windows-conpty-smoke"
$exe = Join-Path $outDir "ConptySmoke.exe"
$csc = Join-Path $env:WINDIR "Microsoft.NET\Framework64\v4.0.30319\csc.exe"

if (!(Test-Path $csc)) {
    throw "missing .NET Framework C# compiler: $csc"
}

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
& $csc /nologo "/out:$exe" $source
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if ($SelfTest) {
    & $exe --self-test
    exit $LASTEXITCODE
}

if ($NestedLinuxSlsh) {
    & $exe --nested-linux-nano-row $NestedLinuxSlsh $HostName
    exit $LASTEXITCODE
}

if (!$SlshExe) {
    $SlshExe = Join-Path $root "target\release\slsh.exe"
}

if (!(Test-Path $SlshExe)) {
    throw "missing slsh.exe: $SlshExe"
}

if ($NanoRow) {
    & $exe --nano-row $SlshExe $HostName
    exit $LASTEXITCODE
}

& $exe $SlshExe $HostName
exit $LASTEXITCODE
