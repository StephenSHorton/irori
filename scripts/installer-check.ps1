# Checks the NSIS installer's silent-update path in a scratch per-user
# install dir (architecture.md section 8, M8 check 1), with dummy exes in
# place of irori.exe, so nothing real starts and no data dir is opened.
#
#   powershell -NoProfile -File scripts/installer-check.ps1 [-Scratch <dir>]
#
# The installer is built from app/nsis/irori.nsi with -DIRORI_SCRATCH, which
# leaves out the shortcuts, the uninstall key, the WebView2 step and every
# registry write, and installed with /D=<scratch>\Programs\Irori. Cases, as
# the Go updater drives them (Go internal/update, project.nsi):
#   fresh       /S into an empty dir: installs and starts the new exe
#   update      /S /WAITPID=<gui>: waits for the running "GUI", replaces
#               irori.exe and relaunches the new one
#   locked      as update, while a second process (an `irori mcp`) keeps
#               running from irori.exe: renamed aside to irori.exe.old
#   norelaunch  /S /NORELAUNCH /WAITPID=<gui>: replaces, starts nothing
#   gaveup      the GUI never exits (wait cut to 2 s): exit code 3, the old
#               exe stays and is relaunched
#   release     the release-mode installer compiles (not run: it writes the
#               real shortcuts and HKCU keys)
# Needs NSIS (makensis on PATH or in C:\Program Files (x86)\NSIS) and rustc.
param(
    [string]$Scratch = ""
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot

$makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $makensis) { $makensis = "C:\Program Files (x86)\NSIS\makensis.exe" }
if (-not (Test-Path $makensis)) { throw "makensis not found (winget install NSIS.NSIS)" }

if (-not $Scratch) { $Scratch = Join-Path ([System.IO.Path]::GetTempPath()) "irori-installer-check" }
$root = Join-Path $Scratch ("run-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Force $root | Out-Null
$src = Join-Path $root "src"
$bin = Join-Path $root "bin"
New-Item -ItemType Directory -Force $src, $bin | Out-Null

# Two stubs. Started with "hold <s>" they sleep (the running GUI, or an
# `irori mcp` child holding the image); started bare, as the installer's
# relaunch does, they append "<tag> <pid>" to launched.txt beside themselves.
$stub = @'
use std::io::Write;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() >= 3 && a[1] == "hold" {
        std::thread::sleep(std::time::Duration::from_secs(a[2].parse().unwrap()));
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let mut f = std::fs::OpenOptions::new().create(true).append(true)
        .open(exe.parent().unwrap().join("launched.txt")).unwrap();
    writeln!(f, "{} {}", TAG, std::process::id()).unwrap();
}
'@
foreach ($tag in "old", "new") {
    Set-Content -Encoding ascii (Join-Path $src "$tag.rs") ("const TAG: &str = `"$tag`";`n" + $stub)
    & rustc --edition 2021 -O -o (Join-Path $bin "$tag.exe") (Join-Path $src "$tag.rs")
    if ($LASTEXITCODE -ne 0) { throw "rustc $tag failed" }
}
$oldExe = Join-Path $bin "old.exe"
$newExe = Join-Path $bin "new.exe"
$newHash = (Get-FileHash $newExe).Hash
$oldHash = (Get-FileHash $oldExe).Hash

$nsi = Join-Path $repo "app\nsis\irori.nsi"
function Build-Installer([string]$out, [string[]]$extra) {
    $mk = @("-V2", "-DIRORI_SCRATCH", "-DINFO_PRODUCTVERSION=0.6.0", "-DIRORI_BINARY=$newExe", "-DIRORI_OUTFILE=$out") + $extra + @($nsi)
    & $makensis @mk
    if ($LASTEXITCODE -ne 0) { throw "makensis failed ($LASTEXITCODE)" }
}
$installer = Join-Path $bin "irori-0.6.0-windows-amd64-installer.exe"
$installerShort = Join-Path $bin "irori-short-wait-installer.exe"
Build-Installer $installer @()
Build-Installer $installerShort @("-DIRORI_WAITPID_MS=2000")

$failures = @()
function Check([string]$case, [bool]$ok, [string]$what) {
    if ($ok) { Write-Host "  ok   $what" } else { Write-Host "  FAIL $what"; $script:failures += "${case}: $what" }
}
function Launched([string]$dir) {
    $p = Join-Path $dir "launched.txt"
    if (Test-Path $p) { return @(Get-Content $p) } else { return @() }
}
function Wait-Launched([string]$dir, [string]$tag) {
    for ($i = 0; $i -lt 50; $i++) {
        if ((Launched $dir) -match "^$tag ") { return $true }
        Start-Sleep -Milliseconds 100
    }
    return $false
}
function New-Install([string]$case) {
    $dir = Join-Path $root "$case\Programs\Irori"
    New-Item -ItemType Directory -Force $dir | Out-Null
    return $dir
}
function Install-Old([string]$dir) { Copy-Item $oldExe (Join-Path $dir "irori.exe") }
function Hold([string]$dir, [int]$secs) {
    return Start-Process -FilePath (Join-Path $dir "irori.exe") -ArgumentList "hold", "$secs" -PassThru -WindowStyle Hidden
}
function Run-Installer([string]$exe, [string]$dir, [string[]]$opts) {
    # /D= must come last and unquoted.
    $argv = @("/S") + $opts + @("/D=$dir")
    $p = Start-Process -FilePath $exe -ArgumentList $argv -PassThru -Wait
    return $p.ExitCode
}
function Exe-Hash([string]$dir) { return (Get-FileHash (Join-Path $dir "irori.exe")).Hash }
# What the Desktop-shortcut step saw (-DIRORI_SCRATCH writes it): "1" fresh.
function Fresh([string]$dir) { return (Get-Content -Raw (Join-Path $dir "fresh-install.txt")).Trim() }

Write-Host "fresh"
$d = New-Install "fresh"
$code = Run-Installer $installer $d @()
Check "fresh" ($code -eq 0) "exit code 0 (got $code)"
Check "fresh" ((Exe-Hash $d) -eq $newHash) "irori.exe is the new build"
Check "fresh" (Wait-Launched $d "new") "the new exe was started"
Check "fresh" (Test-Path (Join-Path $d "uninstall.exe")) "uninstall.exe written"
Check "fresh" ((Fresh $d) -eq "1") "seen as a fresh install (Desktop shortcut made)"

Write-Host "update"
$d = New-Install "update"
Install-Old $d
$gui = Hold $d 3
$t0 = Get-Date
$code = Run-Installer $installer $d @("/WAITPID=$($gui.Id)")
$waited = ((Get-Date) - $t0).TotalSeconds
Check "update" ($code -eq 0) "exit code 0 (got $code)"
Check "update" ($gui.HasExited) "the GUI had exited before the installer finished"
Check "update" ($waited -ge 2) "the installer waited for the GUI ($([math]::Round($waited, 1)) s)"
Check "update" ((Exe-Hash $d) -eq $newHash) "irori.exe is the new build"
Check "update" (Wait-Launched $d "new") "the new exe was relaunched"
Check "update" (-not (Test-Path (Join-Path $d "irori.exe.old"))) "no irori.exe.old (nothing held the image)"

Write-Host "locked"
$d = New-Install "locked"
Install-Old $d
$gui = Hold $d 2
$mcp = Hold $d 60
$code = Run-Installer $installer $d @("/WAITPID=$($gui.Id)")
Check "locked" ($code -eq 0) "exit code 0 (got $code)"
Check "locked" ((Exe-Hash $d) -eq $newHash) "irori.exe is the new build"
Check "locked" ((Test-Path (Join-Path $d "irori.exe.old")) -and ((Get-FileHash (Join-Path $d "irori.exe.old")).Hash -eq $oldHash)) "the held old exe was renamed to irori.exe.old"
Check "locked" (-not $mcp.HasExited) "the mcp holder kept running"
Check "locked" (Wait-Launched $d "new") "the new exe was relaunched"
Check "locked" ((Fresh $d) -eq "0") "seen as an update though irori.exe was renamed aside (no Desktop shortcut back)"
Stop-Process -Id $mcp.Id -Force
$mcp.WaitForExit()
# The next install clears the .old copy once nothing holds it.
$gui = Hold $d 1
$code = Run-Installer $installer $d @("/WAITPID=$($gui.Id)")
Check "locked" (($code -eq 0) -and -not (Test-Path (Join-Path $d "irori.exe.old"))) "the next install deleted irori.exe.old"

Write-Host "norelaunch"
$d = New-Install "norelaunch"
Install-Old $d
$gui = Hold $d 2
$code = Run-Installer $installer $d @("/NORELAUNCH", "/WAITPID=$($gui.Id)")
Check "norelaunch" ($code -eq 0) "exit code 0 (got $code)"
Check "norelaunch" ((Exe-Hash $d) -eq $newHash) "irori.exe is the new build"
Start-Sleep -Seconds 2
Check "norelaunch" ((Launched $d).Count -eq 0) "nothing was started"

Write-Host "gaveup"
$d = New-Install "gaveup"
Install-Old $d
$gui = Hold $d 30
$code = Run-Installer $installerShort $d @("/WAITPID=$($gui.Id)")
Check "gaveup" ($code -eq 3) "exit code 3 (got $code)"
Check "gaveup" ((Exe-Hash $d) -eq $oldHash) "irori.exe is still the old build"
Check "gaveup" (Wait-Launched $d "old") "the old exe was relaunched"
Stop-Process -Id $gui.Id -Force

# The release-mode installer (no -DIRORI_SCRATCH: WebView2, shortcuts, the
# uninstall key, the Run-key and WebView2-dir removal) is compiled only, with
# and without a bootstrapper; running it would touch the real Start menu,
# Desktop and HKCU. A dummy file stands in for the bootstrapper.
Write-Host "release (compile only)"
$wv2 = Join-Path $bin "MicrosoftEdgeWebview2Setup.exe"
Copy-Item $oldExe $wv2
foreach ($extra in @(@(), @("-DIRORI_WEBVIEW2_BOOTSTRAPPER=$wv2"))) {
    $out = Join-Path $bin "irori-release-compile.exe"
    & $makensis @(@("-V2", "-DINFO_PRODUCTVERSION=0.6.0", "-DIRORI_BINARY=$newExe", "-DIRORI_OUTFILE=$out") + $extra + @($nsi))
    Check "release" (($LASTEXITCODE -eq 0) -and (Test-Path $out)) "makensis compiles the release installer $($extra -join ' ')"
    Remove-Item -Force $out -ErrorAction SilentlyContinue
}

Start-Sleep -Milliseconds 500
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
if ($failures.Count -gt 0) {
    Write-Host "installer-check: $($failures.Count) failed"
    $failures | ForEach-Object { Write-Host "  $_" }
    exit 1
}
Write-Host "installer-check: all passed"
