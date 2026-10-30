# Builds the Windows release assets locally and, with -Publish, puts them on
# a draft GitHub release (architecture.md 7.2; there is no CI). The local
# twin of the Go build's release.yml windows + release jobs.
#
#   powershell -NoProfile -File scripts/release.ps1 -Version 0.6.0            # build only (dry run)
#   powershell -NoProfile -File scripts/release.ps1 -Version 0.6.0 -Publish   # + draft release
#   powershell -NoProfile -File scripts/release.ps1 -Version 0.6.0 -Finalize  # publish the draft
#
# Order for a release: tag vX.Y.Z and push it; run this with -Publish on the
# PC (draft release with the Windows assets and SHA256SUMS); run
# scripts/release-mac.sh X.Y.Z --publish on the Mac (adds the .app.zip and
# .dmg and their SHA256SUMS lines); then -Finalize here (or --finalize there).
# The updater reads /releases/latest, which never returns drafts or
# prereleases, so nothing installed sees the release before -Finalize.
#
# The asset names and SHA256SUMS are a contract with crates/update (and the
# Go v0.5.x updater, which installs the first Rust release):
# crates/update/tests/release_names.rs reads the names below.
#
# Needs bun, cargo, NSIS (makensis), and gh for -Publish/-Finalize.
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [switch]$Publish,
    [switch]$Finalize,
    [switch]$SkipChecks,
    [switch]$NoWebView2,
    [string]$Repo = "StephenSHorton/irori"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$v = $Version -replace '^v', ''
if ($v -notmatch '^\d+\.\d+\.\d+') { throw "version $Version is not X.Y.Z" }
$tag = "v$v"
# The version resource and NSIS want a bare X.Y.Z (no prerelease part).
$numeric = $v -replace '[-+].*$', ''
$prerelease = $v.Contains('-')

$installer = "irori-$v-windows-amd64-installer.exe"
$zip = "irori-$v-windows-amd64.zip"
# The Mac's two, as scripts/release-mac.sh names them (checked at -Finalize).
$macAssets = @("irori-$v-macos-arm64.app.zip", "irori-$v-macos-arm64.dmg")

if ($Finalize) {
    # All five assets, and a SHA256SUMS line for each of the four, before
    # anything installed can see the release: a Go v0.5.x Mac finding no
    # .app.zip line fails with "release vX has no macOS arm64 app asset".
    $names = @(& gh release view $tag --repo $Repo --json assets --jq ".assets[].name")
    if ($LASTEXITCODE -ne 0) { throw "gh release view failed" }
    $want = @($installer, $zip) + $macAssets
    foreach ($n in $want + @("SHA256SUMS")) {
        if ($names -notcontains $n) { throw "draft $tag has no $n; not publishing" }
    }
    $sums = (& gh release download $tag --repo $Repo --pattern SHA256SUMS --output - | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "gh release download SHA256SUMS failed" }
    foreach ($n in $want) {
        if ($sums -notmatch ("(?m)^[0-9a-f]{64}  " + [regex]::Escape($n) + "\r?$")) {
            throw "SHA256SUMS on $tag has no line for $n; not publishing"
        }
    }
    & gh release edit $tag --repo $Repo --draft=false
    if ($LASTEXITCODE -ne 0) { throw "gh release edit failed" }
    Write-Host "release $tag is published"
    exit 0
}

# The assets go up against the tag (gh --verify-tag), so they must be
# built from it: a clean tree at the tag's commit (Go's CI built the tag
# itself). A dry run only warns.
$dirty = (& git status --porcelain --untracked-files=no | Out-String).Trim()
$head = (& git rev-parse HEAD).Trim()
$tagCommit = (& git rev-parse --verify --quiet "$tag^{commit}")
$problem = ""
if ($dirty) { $problem = "the tree has uncommitted changes" }
elseif (-not $tagCommit) { $problem = "there is no tag $tag here" }
elseif ($tagCommit.Trim() -ne $head) { $problem = "HEAD $head is not $tag ($($tagCommit.Trim()))" }
if ($problem) {
    if ($Publish) { throw "$problem; check out $tag in a clean tree to publish" }
    Write-Warning "$problem (fine for a dry run)"
}

$dist = Join-Path $root "dist\release\$tag"
New-Item -ItemType Directory -Force $dist | Out-Null

if (-not $SkipChecks) {
    # Git for Windows' sh, when PowerShell has no sh on PATH.
    $sh = (Get-Command sh -ErrorAction SilentlyContinue).Source
    if (-not $sh) { $sh = "C:\Program Files\Git\bin\sh.exe" }
    & $sh scripts/check.sh
    if ($LASTEXITCODE -ne 0) { throw "scripts/check.sh failed" }
}

Push-Location frontend
& bun install --frozen-lockfile
if ($LASTEXITCODE -ne 0) { throw "bun install failed" }
& bun run build
if ($LASTEXITCODE -ne 0) { throw "frontend build failed" }
Pop-Location

# IRORI_VERSION is what `irori version`, the API, the lockfile and the
# updater report; TAURI_CONFIG stamps the exe's version resource.
$env:IRORI_VERSION = $v
$env:TAURI_CONFIG = "{`"version`":`"$numeric`"}"
# tauri/custom-protocol is what `tauri build` adds: without it the exe is
# compiled with cfg(dev) (tauri::is_dev() true, the build machine's config
# path embedded).
& cargo build --release --locked -p irori-app --features tauri/custom-protocol
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
Remove-Item Env:IRORI_VERSION, Env:TAURI_CONFIG
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "target" }
$exe = Join-Path $targetDir "release\irori.exe"

# Smoke: the exact text (Go release.yml). A throwaway IRORI_HOME, so
# nothing reads the real data dir even if the role ever opened it.
$smokeHome = Join-Path ([System.IO.Path]::GetTempPath()) ("irori-release-smoke-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
$env:IRORI_HOME = $smokeHome
$out = (& $exe version | Out-String).Trim()
Remove-Item Env:IRORI_HOME
Remove-Item -Recurse -Force $smokeHome -ErrorAction SilentlyContinue
if ($out -ne "irori $v") { throw "irori version printed '$out', want 'irori $v'" }
Write-Host "smoke: $out"

$makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $makensis) { $makensis = "C:\Program Files (x86)\NSIS\makensis.exe" }
$nsisArgs = @("-DINFO_PRODUCTVERSION=$numeric", "-DIRORI_BINARY=$exe", "-DIRORI_OUTFILE=$dist\$installer")
if (-not $NoWebView2) {
    # Microsoft's evergreen bootstrapper, as wails3 generate webview2bootstrapper
    # fetched it for the Go installer; installed only where WebView2 is missing.
    $wv2 = Join-Path $root "dist\MicrosoftEdgeWebview2Setup.exe"
    if (-not (Test-Path $wv2)) {
        Invoke-WebRequest -UseBasicParsing -UserAgent "irori-rust-dev" -Uri "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile $wv2
    }
    # It goes inside the signed installer, so it must be Microsoft's: a
    # valid Authenticode signature by Microsoft Corporation (the evergreen
    # bootstrapper changes, so there is no hash to pin). A cached copy is
    # checked again every run; a bad one is deleted.
    $sig = Get-AuthenticodeSignature -FilePath $wv2
    if ($sig.Status -ne "Valid" -or $sig.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") {
        Remove-Item -Force $wv2
        throw "WebView2 bootstrapper signature: $($sig.Status) ($($sig.SignerCertificate.Subject)); deleted, run again"
    }
    $nsisArgs += "-DIRORI_WEBVIEW2_BOOTSTRAPPER=$wv2"
}
& $makensis @nsisArgs (Join-Path $root "app\nsis\irori.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed" }

Compress-Archive -Path $exe -DestinationPath (Join-Path $dist $zip) -Force

# sha256sum format: "<hex>  <name>", LF line ends.
$sums = ""
foreach ($n in @($installer, $zip)) {
    $h = (Get-FileHash -Algorithm SHA256 (Join-Path $dist $n)).Hash.ToLower()
    $sums += "$h  $n`n"
}
[System.IO.File]::WriteAllText((Join-Path $dist "SHA256SUMS"), $sums)
Get-ChildItem $dist | Format-Table Name, Length
Get-Content (Join-Path $dist "SHA256SUMS")

if ($Publish) {
    $ghArgs = @("release", "create", $tag, "--repo", $Repo, "--verify-tag", "--draft", "--title", "Irori $tag", "--generate-notes")
    if ($prerelease) { $ghArgs += "--prerelease" }
    $ghArgs += @((Join-Path $dist $installer), (Join-Path $dist $zip), (Join-Path $dist "SHA256SUMS"))
    & gh @ghArgs
    if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
    Write-Host "draft release $tag created; add the Mac assets (scripts/release-mac.sh $v --publish), then -Finalize"
} else {
    Write-Host "dry run: assets in $dist"
}
