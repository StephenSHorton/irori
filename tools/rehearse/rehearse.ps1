# The full rehearsal on copies (architecture.md 5.2, 8 M8 check 3), on the
# PC. Every leg works on its own fresh copy or scratch install; the live data
# dir is only ever copied, the running Go app, its install and the real Run
# key are never touched (checked before and after).
#
#   powershell -NoProfile -File tools/rehearse/rehearse.ps1 -Scratch <dir>
#       [-Data <data dir>] [-GoIrori <Go repo>] [-Skip verify,replay,probe,update]
#       [-Release]
#
# Legs:
#   verify   tools/goldens/verify-live.sh: every row signature and RowHash on
#            a copy, a Rust append, the Go Core opening the Rust copy.
#   replay   tools/goldens/replay-live.sh: the Rust and Go Cores' views of
#            identical copies are equal.
#   probe    irori-dev probe: the Rust build (`irori serve --headless`) on a
#            copy with IRORI_DEV_OFFLINE=1 (no socket ever binds: the copy
#            holds this device's real key), every Go API route answers, and
#            `irori mcp` lists Go's tools. With IRORI_VERIFY_DEVICES=1 its
#            /v1/devices is also compared with the running Go Core's.
#   update   Go v0.5.0 -> Rust v0.6.0 in scratch per-user installs. The client
#            is Go's own internal/update code (tools/rehearse/
#            update_rehearse_test.go via go test -overlay) against a fake
#            GitHub on 127.0.0.1 serving the Rust installer, built here from
#            app/nsis/irori.nsi with IRORI_SCRATCH and IRORI_SCRATCH_INSTDIR
#            (Go passes no /D=). The go test process is the running Go GUI
#            (/WAITPID); a stub "old irori.exe" holds the image as an `irori
#            mcp` would. Two runs:
#              restart  "Restart now": the installer replaces irori.exe
#                       (renaming the held one aside) and relaunches the
#                       Rust build, which runs under the shell probe (no
#                       window, no tray), offline, on a fresh scratch home,
#                       and does the launch-at-login switch-over against a
#                       scratch Run key holding the value Wails wrote for
#                       the Go install (IRORI_DEV_RUN_KEY).
#              exit     quit with the update ready: /NORELAUNCH, nothing
#                       starts.
#
# The Rust build is a dev build stamped IRORI_VERSION=0.6.0. With -Release
# the update leg installs the release build instead: scripts/release.ps1
# -Version 0.6.0 -SkipChecks runs first (custom-protocol, LTO, its dry-run
# installer, zip and SHA256SUMS under dist\release\v0.6.0, left there), and
# the leg wraps that same target\release\irori.exe (the zip's exe, checked)
# in the scratch installer; release.ps1's own installer is never run (its
# $INSTDIR is the real install). The restart run then also checks that the
# frontend loaded from the embedded assets (http://tauri.localhost). A
# release build honours the shell probe and the scratch Run key only with
# IRORI_DEV_OFFLINE=1, which the leg sets. Needs bun, cargo, go,
# Git's sh and NSIS. Everything made under <Scratch>\irori-rehearse is
# deleted at the end; output is statuses, never row contents, keys or tokens.
param(
    [Parameter(Mandatory = $true)][string]$Scratch,
    [string]$Data = (Join-Path $env:LOCALAPPDATA "Irori"),
    [string]$GoIrori = "",
    [string[]]$Skip = @(),
    [switch]$Release
)
$ErrorActionPreference = "Stop"
# -File passes "a,b" as one string.
$Skip = @($Skip | ForEach-Object { $_ -split "," } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $GoIrori) { $GoIrori = Join-Path (Split-Path -Parent $repo) "irori-go" }
$GoIrori = (Resolve-Path $GoIrori).Path
$Data = (Resolve-Path $Data).Path

$sh = (Get-Command sh -ErrorAction SilentlyContinue).Source
if (-not $sh) { $sh = Join-Path $env:ProgramFiles "Git\bin\sh.exe" }
$makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $makensis) { $makensis = "C:\Program Files (x86)\NSIS\makensis.exe" }
foreach ($t in @($sh, $makensis)) { if (-not (Test-Path $t)) { throw "missing tool: $t" } }

New-Item -ItemType Directory -Force $Scratch | Out-Null
$work = Join-Path (Resolve-Path $Scratch).Path "irori-rehearse"
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force $work | Out-Null
$runId = [guid]::NewGuid().ToString("N").Substring(0, 8)
$results = [ordered]@{}
function Result([string]$leg, [bool]$ok, [string]$note = "") {
    $script:results[$leg] = @{ ok = $ok; note = $note }
    if ($ok) { Write-Host "== $leg passed $note" } else { Write-Host "== $leg FAILED $note" }
}
function Fwd([string]$p) { return $p.Replace("\", "/") }

# --- What must not change ---------------------------------------------------
$realRunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$realExe = Join-Path $env:LOCALAPPDATA "Programs\Irori\irori.exe"
$realUninst = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Stephen HortonIrori"
function Snapshot {
    $run = (Get-ItemProperty $realRunKey -ErrorAction SilentlyContinue)
    $runVal = if ($run -and ($run.PSObject.Properties.Name -contains "Irori")) { $run.Irori } else { "<none>" }
    $exe = if (Test-Path $realExe) { (Get-FileHash $realExe).Hash } else { "<none>" }
    $un = if (Test-Path $realUninst) { (Get-ItemProperty $realUninst).DisplayVersion } else { "<none>" }
    $wv = Test-Path (Join-Path $env:LOCALAPPDATA "com.stephenshorton.irori")
    return "$runVal|$exe|$un|$wv"
}
$before = Snapshot
$goBefore = (& git -C $GoIrori --no-optional-locks status --porcelain) -join "`n"

# --- Build ------------------------------------------------------------------
Write-Host "== build: frontend, then irori and irori-dev (dev, IRORI_VERSION=0.6.0)"
Push-Location (Join-Path $repo "frontend")
try {
    & bun install --frozen-lockfile | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "bun install failed" }
    & bun run build | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "bun run build failed" }
} finally { Pop-Location }
Push-Location $repo
try {
    $env:IRORI_VERSION = "0.6.0"
    & cargo build -q -p irori-app -p irori-dev
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Remove-Item Env:IRORI_VERSION -ErrorAction SilentlyContinue; Pop-Location }
$bin = Join-Path $work "bin"
New-Item -ItemType Directory -Force $bin | Out-Null
$exe = Join-Path $bin "irori.exe"
Copy-Item (Join-Path $repo "target\debug\irori.exe") $exe
$dev = Join-Path $bin "irori-dev.exe"
Copy-Item (Join-Path $repo "target\debug\irori-dev.exe") $dev
$v = (& $exe version) -join ""
if ($v -ne "irori 0.6.0") { throw "the built exe says '$v', not 'irori 0.6.0'" }

# The exe the update leg installs: the dev build, or with -Release the one
# release.ps1 builds and zips.
$upExe = $exe
if ($Release) {
    Write-Host "== build: scripts/release.ps1 -Version 0.6.0 -SkipChecks (release, custom-protocol, LTO)"
    & powershell -NoProfile -File (Join-Path $repo "scripts\release.ps1") -Version 0.6.0 -SkipChecks
    if ($LASTEXITCODE -ne 0) { throw "release.ps1 failed" }
    $relTarget = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo "target" }
    $relExe = Join-Path $relTarget "release\irori.exe"
    $zipped = Join-Path $work "release-zip"
    Expand-Archive -Path (Join-Path $repo "dist\release\v0.6.0\irori-0.6.0-windows-amd64.zip") -DestinationPath $zipped
    if ((Get-FileHash $relExe).Hash -ne (Get-FileHash (Join-Path $zipped "irori.exe")).Hash) {
        throw "target\release\irori.exe is not the exe in release.ps1's zip"
    }
    Remove-Item -Recurse -Force $zipped
    $upExe = Join-Path $bin "release\irori.exe"
    New-Item -ItemType Directory -Force (Split-Path -Parent $upExe) | Out-Null
    Copy-Item $relExe $upExe
    $rv = (& $upExe version | Out-String).Trim()
    if ($rv -ne "irori 0.6.0") { throw "the release exe says '$rv', not 'irori 0.6.0'" }
}

# A stable copy of the data dir (as verify-live.sh: the database files are
# copied twice and compared, so a checkpoint mid-copy can't tear the set;
# no irori.json, so no copy ever claims the live Core's port or token).
function Copy-Data([string]$dst) {
    New-Item -ItemType Directory -Force $dst | Out-Null
    $db = @("irori.db", "irori.db-wal", "irori.db-shm")
    for ($try = 1; $try -le 5; $try++) {
        $a = Join-Path $dst ".db-a"; $b = Join-Path $dst ".db-b"
        foreach ($d in @($a, $b)) {
            if (Test-Path $d) { Remove-Item -Recurse -Force $d }
            New-Item -ItemType Directory -Force $d | Out-Null
            foreach ($f in $db) { if (Test-Path (Join-Path $Data $f)) { Copy-Item (Join-Path $Data $f) $d } }
        }
        $same = $true
        foreach ($f in $db) {
            $pa = Join-Path $a $f; $pb = Join-Path $b $f
            if ((Test-Path $pa) -ne (Test-Path $pb)) { $same = $false; continue }
            if ((Test-Path $pa) -and ((Get-FileHash $pa).Hash -ne (Get-FileHash $pb).Hash)) { $same = $false }
        }
        if ($same) {
            Get-ChildItem $a | ForEach-Object { Move-Item $_.FullName $dst }
            Remove-Item -Recurse -Force $a, $b
            break
        }
        Remove-Item -Recurse -Force $a, $b
        if ($try -eq 5) { throw "the database kept changing while it was copied; try again when Irori is idle" }
        Start-Sleep -Seconds 1
    }
    foreach ($f in @("device.json", "device.key", "person.json", "turn.key", "mesh-peers.json", "hub-addrs.json",
            "home-server.json", "relay.json", "names.json", "settings.json", "capture-cursors.json", "pair-rows.json")) {
        if (Test-Path (Join-Path $Data $f)) { Copy-Item (Join-Path $Data $f) $dst }
    }
    if (Test-Path (Join-Path $Data "boards")) { Copy-Item -Recurse (Join-Path $Data "boards") (Join-Path $dst "boards") }
}

# --- verify and replay (the existing live scripts) --------------------------
foreach ($leg in @("verify", "replay")) {
    if ($Skip -contains $leg) { Write-Host "== $leg skipped"; continue }
    Write-Host "== $leg"
    $legDir = Join-Path $work $leg
    New-Item -ItemType Directory -Force $legDir | Out-Null
    Push-Location $repo
    try {
        & $sh "tools/goldens/$leg-live.sh" (Fwd $Data) (Fwd $legDir) (Fwd $GoIrori)
        Result $leg ($LASTEXITCODE -eq 0)
    } finally { Pop-Location }
}

# --- probe: the API routes and the MCP tool list -----------------------------
if ($Skip -contains "probe") { Write-Host "== probe skipped" } else {
    Write-Host "== probe"
    $p = Join-Path $work "probe"
    Copy-Data (Join-Path $p "home")
    New-Item -ItemType Directory -Force (Join-Path $p "claude") | Out-Null
    $probeArgs = @("--irori", $exe, "--home", (Join-Path $p "home"), "--claude", (Join-Path $p "claude"),
        "--go-tools", (Join-Path $GoIrori "internal\mcpserver\fallback_tools.json"))
    if ($env:IRORI_VERIFY_DEVICES -eq "1") {
        # As verify-live.sh: the running Go Core's lockfile is copied beside
        # the copy, never into it, and irori-dev reads its port and token in
        # code to compare /v1/devices.
        $lock = Join-Path $p "lock"
        New-Item -ItemType Directory -Force $lock | Out-Null
        Copy-Item (Join-Path $Data "irori.json") (Join-Path $lock "irori.json")
        $probeArgs += @("--go-lockfile", (Join-Path $lock "irori.json"))
    }
    & $dev probe @probeArgs
    Result "probe" ($LASTEXITCODE -eq 0)
    Start-Sleep -Milliseconds 500
    Remove-Item -Recurse -Force $p -ErrorAction SilentlyContinue
}

# --- update: Go v0.5.0 -> Rust v0.6.0 ----------------------------------------
function Wait-Until([int]$secs, [scriptblock]$ok) {
    $deadline = (Get-Date).AddSeconds($secs)
    while ((Get-Date) -lt $deadline) {
        if (& $ok) { return $true }
        Start-Sleep -Milliseconds 200
    }
    return (& $ok)
}
function Procs-Under([string]$dir) {
    return @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
            $_.Path -and $_.Path.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase)
        })
}

function Update-Run([string]$mode) {
    $u = Join-Path $work "update-$mode"
    $inst = Join-Path $u "Programs\Irori"
    $home_ = Join-Path $u "rehearse-home"
    $claude = Join-Path $u "claude"
    $shellLog = Join-Path $u "shell.log"
    New-Item -ItemType Directory -Force $inst, $home_, $claude | Out-Null
    if ($inst.Contains(" ")) { throw "the scratch install path has a space: Wails would quote the Run value" }
    function Check([bool]$ok, [string]$what) {
        if ($ok) { Write-Host "  ok   $what" } else { Write-Host "  FAIL $what"; $script:upFails += "${mode}: $what" }
    }

    # The Go install: a stub irori.exe (never the real Go build: a gave-up
    # installer relaunches it), held open as an `irori mcp` would.
    $src = Join-Path $u "stub.rs"
    Set-Content -Encoding ascii $src @'
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() >= 3 && a[1] == "hold" {
        std::thread::sleep(std::time::Duration::from_secs(a[2].parse().unwrap()));
    }
}
'@
    & rustc --edition 2021 -O -o (Join-Path $inst "irori.exe") $src
    if ($LASTEXITCODE -ne 0) { throw "rustc stub failed" }
    Remove-Item (Join-Path $inst "irori.pdb") -ErrorAction SilentlyContinue
    $oldHash = (Get-FileHash (Join-Path $inst "irori.exe")).Hash
    $holder = Start-Process -FilePath (Join-Path $inst "irori.exe") -ArgumentList "hold", "300" -PassThru -WindowStyle Hidden

    # The Rust installer, its default $INSTDIR the scratch install.
    $installer = Join-Path $u "irori-0.6.0-windows-amd64-installer.exe"
    # Quoted: PowerShell 5.1 splits a bare -DX=0.6.0 at the dot.
    & $makensis "-V2" "-DIRORI_SCRATCH" "-DIRORI_SCRATCH_INSTDIR=$inst" "-DINFO_PRODUCTVERSION=0.6.0" `
        "-DIRORI_BINARY=$upExe" "-DIRORI_OUTFILE=$installer" (Join-Path $repo "app\nsis\irori.nsi")
    if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
    $newHash = (Get-FileHash $upExe).Hash

    # What Wails wrote for the Go install (autostart_windows.go enable:
    # quoteWindowsArg(exe) + " --startup"; no space in the path, so bare),
    # under a scratch key, beside another app's value.
    $subkey = "Software\irori-rust-rehearse\$runId-$mode\Run"
    $key = "HKCU:\$subkey"
    New-Item -Force $key | Out-Null
    $goValue = "$inst\irori.exe --startup"
    New-ItemProperty -Path $key -Name "Irori" -Value $goValue -PropertyType String | Out-Null
    New-ItemProperty -Path $key -Name "Other" -Value "C:\other\app.exe" -PropertyType String | Out-Null

    $overlay = Join-Path $u "overlay.json"
    $target = Fwd (Join-Path $GoIrori "internal\update\update_rehearse_test.go")
    $ours = Fwd (Join-Path $PSScriptRoot "update_rehearse_test.go")
    Set-Content -Encoding ascii $overlay "{`"Replace`":{`"$target`":`"$ours`"}}"
    $pidFile = Join-Path $u "go-gui.pid"
    $envs = @{
        IRORI_HOME = $home_; CLAUDE_CONFIG_DIR = $claude; IRORI_DEV_OFFLINE = "1"
        IRORI_DEV_SHELL_LOG = $shellLog; IRORI_DEV_RUN_KEY = $subkey
        IRORI_REHEARSE_INSTALLER = $installer; IRORI_REHEARSE_MODE = $mode; IRORI_REHEARSE_PIDFILE = $pidFile
    }
    foreach ($k in $envs.Keys) { Set-Item "Env:$k" $envs[$k] }
    Push-Location $GoIrori
    # PowerShell 5.1 turns a native command's redirected stderr into errors.
    $eap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $out = & go test -count=1 -v -overlay $overlay -vet=off -run '^TestRehearseGoUpdatesToRust$' ./internal/update 2>&1
        $goOk = ($LASTEXITCODE -eq 0)
    } finally {
        $ErrorActionPreference = $eap
        Pop-Location
        foreach ($k in $envs.Keys) { Remove-Item "Env:$k" -ErrorAction SilentlyContinue }
    }
    $out | Where-Object { "$_" -match '^(---|ok|FAIL|panic|\s+update_rehearse_test)' } | ForEach-Object { Write-Host "  $_" }
    Check $goOk "the Go updater found, downloaded and verified v0.6.0 and launched the installer"

    if ($goOk) {
        $downloaded = Join-Path $home_ "updates\irori-0.6.0-windows-amd64-installer.exe"
        $done = Wait-Until 90 { -not (Procs-Under $downloaded) -and (Test-Path (Join-Path $inst "irori.exe")) -and ((Get-FileHash (Join-Path $inst "irori.exe")).Hash -eq $newHash) }
        Check $done "the installer waited for the Go GUI and replaced irori.exe with the Rust build"
        $old = Join-Path $inst "irori.exe.old"
        Check ((Test-Path $old) -and ((Get-FileHash $old).Hash -eq $oldHash)) "the held Go exe was renamed to irori.exe.old"
        Check (-not $holder.HasExited) "the holder (an old irori mcp) kept running"
        Check (Test-Path (Join-Path $inst "uninstall.exe")) "uninstall.exe written"
        $vv = (& (Join-Path $inst "irori.exe") version | Out-String).Trim()
        Check ($vv -eq "irori 0.6.0") "the installed exe says 'irori 0.6.0' (got '$vv')"
        if ($mode -eq "restart") {
            $lines = { if (Test-Path $shellLog) { @(Get-Content $shellLog) } else { @() } }
            $ready = Wait-Until 180 { (& $lines) -contains "ready" }
            Check $ready "the relaunched Rust build came up (shell probe: no window, offline, scratch home)"
            Check ((& $lines) -contains "version 0.6.0") "it runs v0.6.0"
            # The hidden window loaded the frontend; a release build from
            # its embedded assets.
            $pageLoad = { @(& $lines) | Where-Object { $_ -like "page-load *" } | Select-Object -First 1 }
            $loaded = Wait-Until 60 { [bool](& $pageLoad) }
            if ($Release) {
                Check ($loaded -and ((& $pageLoad) -eq "page-load http://tauri.localhost")) "the hidden window loaded the embedded frontend (got '$(& $pageLoad)')"
            } else {
                Check $loaded "the hidden window loaded the frontend (got '$(& $pageLoad)')"
            }
            # It serves: the loopback API's health route (no token).
            $portLine = @(& $lines) | Where-Object { $_ -match '^api-port \d+$' } | Select-Object -First 1
            $health = $null
            if ($portLine) {
                $port = $portLine.Substring(9)
                try {
                    $health = Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/health" -TimeoutSec 10
                } catch { $health = $null }
            }
            Check ($health -and $health.ok -eq $true -and $health.version -eq "0.6.0") "it serves: GET /v1/health on its api-port says ok, version 0.6.0"
            Check ((& $lines) -contains "login-cutover keep") "the switch-over kept the Go Run value (already the Rust form)"
            Check ((& $lines) -contains "login-enabled true") "the Rust build reads launch at login as on"
            $vals = Get-ItemProperty $key
            Check ($vals.Irori -eq $goValue) "Run value Irori is still '<install>\irori.exe --startup'"
            Check ($vals.Other -eq "C:\other\app.exe") "another app's Run value is untouched"
            $running = Procs-Under $inst | Where-Object { $_.Id -ne $holder.Id }
            Check ($running.Count -eq 1) "one Rust irori.exe running from the scratch install"
            Set-Content -Encoding ascii "$shellLog.quit" ""
            $quit = Wait-Until 60 { -not (Procs-Under $inst | Where-Object { $_.Id -ne $holder.Id }) }
            Check ($quit -and ((& $lines) -contains "exiting")) "it quit cleanly through the probe"
        } else {
            Start-Sleep -Seconds 5
            $running = Procs-Under $inst | Where-Object { $_.Id -ne $holder.Id }
            Check ($running.Count -eq 0) "/NORELAUNCH: nothing started"
            Check (-not (Test-Path $shellLog)) "no Rust GUI ran"
        }
    }

    Stop-Process -Id $holder.Id -Force -ErrorAction SilentlyContinue
    Procs-Under $u | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force "HKCU:\Software\irori-rust-rehearse\$runId-$mode" -ErrorAction SilentlyContinue
    if (-not (Get-ChildItem "HKCU:\Software\irori-rust-rehearse" -ErrorAction SilentlyContinue)) {
        Remove-Item -Force "HKCU:\Software\irori-rust-rehearse" -ErrorAction SilentlyContinue
    }
}

if ($Skip -contains "update") { Write-Host "== update skipped" } else {
    $script:upFails = @()
    foreach ($mode in @("restart", "exit")) {
        Write-Host "== update ($mode)"
        try {
            Update-Run $mode
        } catch {
            Write-Host "  FAIL $_"
            $script:upFails += "${mode}: $_"
        } finally {
            # Whatever happened: nothing left running, no scratch key left.
            Procs-Under (Join-Path $work "update-$mode") | Stop-Process -Force -ErrorAction SilentlyContinue
            Remove-Item -Recurse -Force "HKCU:\Software\irori-rust-rehearse\$runId-$mode" -ErrorAction SilentlyContinue
            if (-not (Get-ChildItem "HKCU:\Software\irori-rust-rehearse" -ErrorAction SilentlyContinue)) {
                Remove-Item -Force "HKCU:\Software\irori-rust-rehearse" -ErrorAction SilentlyContinue
            }
        }
    }
    Result "update" ($script:upFails.Count -eq 0) ($script:upFails -join "; ")
}

# --- Nothing real changed ----------------------------------------------------
$after = Snapshot
$goAfter = (& git -C $GoIrori --no-optional-locks status --porcelain) -join "`n"
$untouched = ($before -eq $after) -and ($goBefore -eq $goAfter) -and -not (Procs-Under $work)
Result "untouched" $untouched "(the real Run value, installed exe, uninstall entry, Tauri WebView2 folder, Go repo status)"

Start-Sleep -Milliseconds 500
for ($i = 0; $i -lt 20 -and (Test-Path $work); $i++) {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
    if (Test-Path $work) { Start-Sleep -Milliseconds 500 }
}

$failed = @($results.Keys | Where-Object { -not $results[$_].ok })
Write-Host ""
foreach ($k in $results.Keys) { Write-Host ("{0,-10} {1}" -f $k, $(if ($results[$k].ok) { "passed" } else { "FAILED" })) }
if ($failed.Count -gt 0) { Write-Host "rehearse: $($failed.Count) failed"; exit 1 }
Write-Host "rehearse: all passed"
