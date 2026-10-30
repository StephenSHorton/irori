# The live two-person shared-board run (card #48 s3, v0.5-shared-boards
# §11 "Live"), one side per machine; the Mac's is boards-live.sh.
#
#   powershell -NoProfile -File tools/rehearse/boards-live.ps1 -Role host     (the PC, person A)
#   powershell -NoProfile -File tools/rehearse/boards-live.ps1 -Role guest -Link '<irori-invite:...>'
#
# Each side is `irori-dev boards-live` (crates/dev/src/boards_live.rs): a
# real Core in-process on a fresh scratch IRORI_HOME, as a made-up person
# with one made-up Claude session, stepping through the run and printing
# PASS or FAIL per step. Person A (the PC) turns its home server on, so B
# reaches A through A's relay; it creates a board and an invite and prints
# the link: send it to B. B joins and prints five words; read them out over
# the call, and A types them here. Then: cards both ways, A's comment
# reaching B's agent through the bridge, share then withdraw, a board ask,
# a review request from A's agent to B's agent, and B leaving. Each side
# waits on the other by what it sees on the board, up to -StepWait seconds
# per step (default 300).
#
# No DPAPI or Keychain secret: the scratch person's keys are sealed with
# the file keystore inside the scratch home (debug build). The running
# Irori app, %APPDATA%\Irori and the real Run key are never touched.
# IRORI_DEV_PORT_OFFSET (default 200) moves the LAN lookup and the home
# server's ports off the running app's; Windows Firewall may ask once to
# let the debug irori-dev.exe listen. The scratch dirs are deleted at the
# end; the exit code is the number of failed steps.
param(
  [Parameter(Mandatory = $true)][ValidateSet('host', 'guest')][string]$Role,
  [string]$Link = '',
  [int]$StepWait = 300
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
if ($Role -eq 'guest' -and -not $Link) { throw 'boards-live: the guest needs -Link' }
if (-not $env:IRORI_DEV_PORT_OFFSET) { $env:IRORI_DEV_PORT_OFFSET = '200' }
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("irori-boards-live-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$env:IRORI_HOME = Join-Path $work 'home'
$code = 100
try {
  Push-Location $root
  try { cargo build -q -p irori-dev; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } } finally { Pop-Location }
  $dev = Join-Path $root 'target\debug\irori-dev.exe'
  $claude = Join-Path $work 'claude'
  if ($Role -eq 'host') {
    & $dev boards-live host --home $env:IRORI_HOME --claude $claude --link-out (Join-Path $work 'link.txt') --step-wait $StepWait
  } else {
    & $dev boards-live guest --home $env:IRORI_HOME --claude $claude --link $Link --step-wait $StepWait
  }
  $code = $LASTEXITCODE
} finally {
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
exit $code
