# Releasing Irori

Irori ships the way Toru does: a version tag builds a per-user Windows
installer and a macOS app on GitHub Actions and publishes a GitHub Release,
and every installed Irori finds that release the next time it starts,
downloads it, checks it and asks before it restarts. The Mac has its own
section below ([macOS](#macos)).

## Cut a release

1. Make sure the local checks in the README ([Checks](../README.md#checks-run-locally---no-ci)) pass on `main` and it has everything the release needs.
2. Tag it and push the tag:

   ```sh
   git tag v0.3.0
   git push origin v0.3.0
   ```

   You can also start **Actions > Release > Run workflow** with a `tag`
   input. The tag has to exist already, because the workflow checks it out.
3. Watch the **Release** workflow. When it finishes, the release is live and
   installed copies pick it up the next time they start.

The tag is the version. It has to look like `vX.Y.Z`; the workflow refuses
anything else. A tag with a `-` in it (`v0.4.0-rc.1`) becomes a GitHub
**prerelease**. The updater reads `/releases/latest`, which never returns
prereleases or drafts, so a prerelease is for hand testing and no installed
copy sees it.

The versions in `build/config.yml`, `build/windows/info.json`,
`build/windows/wails.exe.manifest`, `build/windows/nsis/wails_tools.nsh` and
`build/darwin/Info*.plist` are only what a local build falls back to. CI
stamps the tag over them. Keep them on the current minor so local installers
don't say 0.1.0, but a release does not need them bumped. Do not run
`wails3 task common:update:build-assets` to sync them: it regenerates
`project.nsi` and wipes out Irori's update logic.

## What CI produces

`.github/workflows/release.yml` has three jobs.

**windows** (windows-latest):

- runs `go test ./internal/...`;
- writes the numeric version (`X.Y.Z`, prerelease part dropped) into
  `build/windows/info.json`, which becomes the exe's version resource;
- builds the frontend (`bun install --frozen-lockfile`, `bun run build`);
- builds `bin/irori.exe` with `-tags production` and
  `-ldflags "-w -s -H windowsgui -X main.version=<version>"`, so
  `irori version`, the tray, the app header and the updater all report the
  tag's version;
- builds the NSIS installer with `makensis -DWAILS_INSTALL_SCOPE=user
  -DREQUEST_EXECUTION_LEVEL=user -DINFO_PRODUCTVERSION=<X.Y.Z>`;
- uploads two assets:
  - `irori-<version>-windows-amd64-installer.exe`, the per-user installer
    (this is the one the updater uses);
  - `irori-<version>-windows-amd64.zip`, the bare `irori.exe` for portable
    use.

**macos** (macos-latest, Apple silicon) builds `Irori.app` for arm64 and
uploads `irori-<version>-macos-arm64.dmg` and
`irori-<version>-macos-arm64.app.zip`; see [macOS](#macos).

**release** (ubuntu-latest) writes `SHA256SUMS` over all four assets and
creates the GitHub Release `Irori vX.Y.Z` with generated release notes.

The asset names and the `SHA256SUMS` file are a contract with
`internal/update/release.go`: the Windows updater takes the asset whose name
ends in `-installer.exe` and contains `windows`, the macOS updater the one
ending in `-macos-<arch>.app.zip` (or `-macos-universal.app.zip`), and both
refuse to offer a release that has no `SHA256SUMS` line for their asset.
`TestReleaseWorkflowAssetNames` reads the workflow and fails if they drift
apart.

There is no Windows code signing yet, so SmartScreen warns on the first
manual install. Silent updates are not affected.

### Dry run

**Actions > Release > Run workflow** with **dry_run** ticked builds and tests
both platforms from the branch you pick, runs the macOS smoke test, and keeps
the assets as workflow artifacts (`irori-windows-amd64`,
`irori-macos-arm64`) instead of making a release. The tag does not have to
exist; it only names the version stamped in. From a terminal:

```sh
gh workflow run release.yml --ref feat/macos -f tag=v0.0.0-macos-test -f dry_run=true
gh run watch
```

Use it to try a branch's Mac build on a Mac before tagging: download the
`irori-macos-arm64` artifact from the run page.

## What the installer does

`build/windows/nsis/project.nsi`, per-user and without a UAC prompt:

- installs to `%LOCALAPPDATA%\Programs\Irori\irori.exe`;
- always creates the Start menu shortcut, and creates the Desktop shortcut
  only on a fresh install, so an update never brings back one you deleted;
- registers its uninstaller under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Stephen HortonIrori`
  (Settings > Apps lists it);
- the uninstaller removes the install folder, both shortcuts, the WebView2
  data in `%APPDATA%\irori.exe`, and the launch-at-login value
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Irori`.

The installer never turns launch at login on. Irori writes that Run value
itself when you tick **Launch at login** (Settings or the tray), pointing at
the exe that is running with `--startup`. Updates replace that exe in place,
so the value stays valid. A value written by a dev build keeps pointing at
the dev build until you toggle it from the installed copy.

Irori's data (`%LOCALAPPDATA%\irori`: the ledger, settings.json,
downloaded updates) is not part of the install and survives both updates
and uninstalling.

## How the updater picks it up

`internal/update` runs in the desktop app only; `irori serve --headless` and
`irori mcp` never update.

1. When the app starts (including a hidden launch at login), it asks
   `https://api.github.com/repos/StephenSHorton/irori/releases/latest`.
   A `dev` build (no `-X main.version`) never asks.
2. If the tag is newer than the running version, it downloads the installer
   to `%LOCALAPPDATA%\irori\updates\` and checks it against `SHA256SUMS`. A
   mismatch deletes it and reports an error.
3. Once the installer checks out the updater is **ready**, and Irori says so
   in three places:
   - a sticky notification card, id `update:<version>`, with **Install** and
     **Later** (only if notifications are on in Settings);
   - the banner at the top of the window, with **Restart now** and **Later**;
   - **Restart to update to vX** at the top of the tray menu. Settings
     also has a version line with **Check for updates**, which shows
     **Restart now** once an update is ready.
4. **Install** / **Restart now** / the tray item runs the installer silently
   (`/S /WAITPID=<pid>`) and quits. The installer waits for the GUI process
   to exit, installs, and starts the new Irori. The card comes down as soon
   as the install starts. If the installer can't be started, the card comes
   back with the reason.
5. **Later** just closes the card or banner. When you quit Irori with an
   update ready, the installer runs silently with `/NORELAUNCH` and Irori
   stays closed.

Agent sessions keep `irori mcp` processes running from the same exe, so the
exe can still be locked after the GUI quits. The installer then renames it
to `irori.exe.old` (Windows allows renaming a running exe) and writes the new
one under the original name. Running sessions stay on the old version until
they restart. The next install and the next app start delete the `.old`
copies once nothing holds them. If the exe can be neither written nor
renamed, the installer changes nothing, starts the old Irori again (unless
`/NORELAUNCH`), and the update is offered again at the next start.

## macOS

### What the macos job does

- builds the frontend and runs the Go tests (the FSEvents watcher, kqueue
  process waits and the updater's bundle swap are tested on the runner);
- builds `bin/irori` with `CGO_ENABLED=1` (Wails' AppKit and WebKit glue and
  FSEvents need cgo), `GOARCH=arm64`, `MACOSX_DEPLOYMENT_TARGET=12.0` and
  `-ldflags "-w -s -X main.version=<version>"`;
- runs `build/darwin/package.sh`, which assembles `Irori.app` by hand
  (`Contents/MacOS/irori`, `build/darwin/Info.plist` with
  `CFBundleShortVersionString` and `CFBundleVersion` set to `X.Y.Z`,
  `icons.icns`, `Assets.car`), signs it, and writes the two assets:
  - `irori-<version>-macos-arm64.dmg`, the first install: open it and drag
    Irori to Applications;
  - `irori-<version>-macos-arm64.app.zip`, `Irori.app` zipped with
    `ditto`: what the in-app updater downloads (it never takes the dmg);
- smoke-tests the bundle: `Irori.app/Contents/MacOS/irori version` must print
  the tag's version, and the desktop app started from the bundle (with a
  throwaway `IRORI_HOME`) must still be running 20 s later.

It is arm64 only. Intel Macs get no asset, and their updater says the
release has no macOS amd64 app. A universal build (a second cgo build for
x86_64 plus `lipo`) is owed.

### Unsigned builds (now)

Without signing secrets the app is signed **ad hoc** (`codesign --sign -`
with the bundle id `com.stephenshorton.irori`). Apple silicon refuses to run
an arm64 binary with no signature at all, but Gatekeeper still treats an
ad hoc app from the internet as unsigned. The first time:

1. Open the dmg and drag **Irori** to **Applications**. (Running it from the
   dmg or from Downloads makes macOS run a temporary read-only copy, which
   cannot update itself.)
2. In Applications, **right-click Irori > Open**, then **Open** in the
   dialog. If the dialog only offers **Done** (macOS 15 and later), open
   **System Settings > Privacy & Security**, scroll to the message about
   Irori, and click **Open Anyway**.

Or clear the quarantine flag from a terminal, after which it opens normally:

```sh
xattr -dr com.apple.quarantine /Applications/Irori.app
```

This is only for the first install. Updates downloaded by Irori itself carry
no quarantine flag, so they open without asking.

### Signing and notarization (later)

The job signs with Developer ID and notarizes as soon as the secrets exist,
with the same names suzuri uses. Add them under **Settings > Secrets and
variables > Actions**:

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | the Developer ID Application certificate and key exported as .p12, base64 (`base64 -i cert.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | the .p12's password |
| `MACOS_CODESIGN_IDENTITY` | the identity's full name, `Developer ID Application: Stephen Horton (TEAMID)` |
| `APPLE_TEAM_ID` | the 10-character team id |

and one set of notarization credentials, either an Apple ID:

| Secret | Value |
| --- | --- |
| `APPLE_ID` | the Apple ID's email |
| `APPLE_APP_SPECIFIC_PASSWORD` | an app-specific password from appleid.apple.com |

or (preferred for CI) an App Store Connect API key:

| Secret | Value |
| --- | --- |
| `APPLE_API_KEY_ID` | the key id |
| `APPLE_API_ISSUER_ID` | the issuer id |
| `APPLE_API_KEY_P8_BASE64` | the .p8 file, base64 |

With them, `package.sh` signs the executable and then the bundle with
hardened runtime and a secure timestamp, notarizes and staples the app
(`build/darwin/notarize.sh`, `xcrun notarytool submit --wait`), and signs,
notarizes and staples the dmg. A Developer ID identity without notarization
credentials signs and skips notarization. Hardened runtime is on with no
entitlements file; if notarization or the WebView needs one, add
`build/darwin/entitlements.plist` and `package.sh` passes it to codesign.

Once a release is Developer ID signed, the updater refuses an update that is
not (Gatekeeper would call the swapped-in app damaged), so keep the secrets
once they are in.

### How the updater works on macOS

The same check, card, banner and tray item as on Windows; the install step
differs.

1. The startup check stops early, with the reason in Settings, when Irori is
   not running from an `Irori.app` bundle (a bare `bin/irori`), runs from
   App Translocation's read-only copy (opened from the dmg or Downloads), or
   the folder holding `Irori.app` is not writable for this user.
2. It downloads `irori-<version>-macos-arm64.app.zip` into
   `~/.irori/updates/` and checks it against `SHA256SUMS`.
3. It unpacks it with `ditto` and checks the bundle id
   (`com.stephenshorton.irori`), that `CFBundleShortVersionString` is the
   release's `X.Y.Z`, that the executable is there, and that
   `codesign --verify --deep --strict` passes. Then it copies the bundle
   next to the installed app as `.Irori.app.new` (hidden, same folder, same
   volume). Only then is the update **ready**.
4. **Install** / **Restart now** starts a detached helper (`/bin/sh`, its
   output in `~/.irori/updates/update.log`) and quits. The helper waits for
   the GUI process to exit, renames `Irori.app` to `.Irori.app.old`, renames
   `.Irori.app.new` to `Irori.app` (putting the old one back if that fails),
   deletes the old one, and reopens Irori with `open`. Quitting with an
   update ready runs the same swap without reopening.

`irori mcp` processes started by agent sessions keep running from the old
bundle's binary after the swap (macOS keeps a deleted file's image alive),
and new sessions get the new one, so nothing waits for them. The next start
deletes a `.Irori.app.new` or `.Irori.app.old` that an interrupted update
left behind. The whole bundle is replaced, signature included, so nothing is
re-signed on the user's Mac (suzuri swaps only the binary, which is why it
needs its heal and re-sign steps).

### Other platform notes

- **Launch at login** uses Wails' autostart: on macOS 13+ from a bundle it
  registers the app with `SMAppService` (System Settings > General > Login
  Items lists it); otherwise it writes a LaunchAgent plist in
  `~/Library/LaunchAgents`. `SMAppService` starts the app without
  arguments, so on macOS 13+ a login launch does not get `--startup` and
  opens the window instead of staying in the menu bar (owed).
- **Menu bar**: a monochrome 囲 template image (`build/trayicon-template.png`,
  rendered from `assets/brand/irori-mark-mono.svg`), so macOS tints it for
  light and dark menu bars. A click opens the menu; **Open Irori** is in
  it.
- **Notifications**: Irori pins `github.com/StephenSHorton/go-toast` v0.1.0,
  whose cards are Windows only; on macOS they are silent no-ops until a
  go-toast release with a macOS backend is pinned.
- **PATH**: an app opened from Finder gets launchd's short PATH, so at start
  the desktop app asks the login shell (`$SHELL -ilc`) for its PATH and puts
  it in front, and `claude`, git and the agents it starts see the user's
  tools.
- **Data**: `~/.irori` (the ledger, settings.json, updates) and Claude
  Code's `~/.claude` (or `CLAUDE_CONFIG_DIR`).

## Test an update end to end

The updater only reads real GitHub releases, so a full test uses two of
them.

1. Release **vA** (for example `v0.3.0`) and install it from the release
   page by running `irori-0.3.0-windows-amd64-installer.exe`. Check
   `%LOCALAPPDATA%\Programs\Irori\irori.exe version` and the badge in the
   app header: both say 0.3.0. (`irori.exe` is a GUI-subsystem exe, so the
   shell does not wait for it: the command-line roles attach to the parent
   console, and the `irori 0.3.0` line can print just after the next prompt.
   `irori version | more` waits for it.)
2. Release **vB** (`v0.3.1`) from a later commit.
3. Quit Irori vA and start it again (or use Settings > **Check for updates**).
   Within a few seconds:
   - the notification card **Irori v0.3.1 is ready** appears;
   - the banner and the tray item appear.
4. Try each path:
   - **Install** on the card: Irori closes and comes back as v0.3.1 (header
     badge, `irori version`).
   - Or **Later**, then quit from the tray: Irori closes and stays closed;
     the next start is v0.3.1.
   - With an agent session running (an `irori mcp` child), install and check
     that `irori.exe.old` appears and goes away once that session ends and
     Irori restarts.
5. Turn notifications off in Settings and repeat with a vC: the banner and
   tray item still appear and the card does not.

To test without publishing anything, build the installer locally (below) and
run it by hand. That checks the install, shortcuts and uninstaller, but not
the update check, which always goes to GitHub.

## Build the installer locally

The same steps as CI, from the repo root in PowerShell. Needs Go, bun,
`wails3` (`go install github.com/wailsapp/wails/v3/cmd/wails3@v3.0.0-alpha.98`)
and NSIS (`choco install nsis` or `winget install NSIS.NSIS`; add
`C:\Program Files (x86)\NSIS` to `PATH`).

```powershell
$version = "0.3.1"           # what main.version reports
$numeric = $version -replace '[-+].*$', ''

# Version resource (CI does this to build/windows/info.json; revert it after).
$j = Get-Content build/windows/info.json -Raw | ConvertFrom-Json
$j.fixed.file_version = $numeric
$j.info.'0000'.ProductVersion = $numeric
$j | ConvertTo-Json -Depth 10 | Set-Content build/windows/info.json

cd frontend; bun install --frozen-lockfile; bun run build; cd ..

Push-Location build
wails3 generate syso -arch amd64 -icon windows/icon.ico -manifest windows/wails.exe.manifest -info windows/info.json -out ../wails_windows_amd64.syso
Pop-Location
$env:CGO_ENABLED = "0"
go build -tags production -trimpath -buildvcs=false -ldflags="-w -s -H windowsgui -X main.version=$version" -o bin/irori.exe .
Remove-Item *.syso

wails3 generate webview2bootstrapper -dir build/windows/nsis
makensis "-DINFO_PRODUCTVERSION=$numeric" "-DWAILS_INSTALL_SCOPE=user" "-DREQUEST_EXECUTION_LEVEL=user" "-DARG_WAILS_AMD64_BINARY=$PWD\bin\irori.exe" build/windows/nsis/project.nsi

git checkout -- build/windows/info.json
```

That writes `bin/irori.exe` and `bin/irori-amd64-installer.exe`.
`wails3 task windows:package VERSION=0.3.1` does the same through the
Taskfile, but it runs `bun install` and `go mod tidy` side by side, which can
trip Windows file locks; CI runs the steps one by one for that reason.

`bin\irori.exe version` prints the stamped version. The exe's version
resource shows in Explorer's Properties > Details. PowerShell's
`(Get-Item bin\irori.exe).VersionInfo` shows the strings blank because the
resource is language-neutral; that is a quirk of the .NET reader, not a
missing resource.

Running the installer installs for real (`%LOCALAPPDATA%\Programs\Irori`,
shortcuts, the Apps entry) even with `/D=<folder>`, which moves only the
files. The uninstaller also deletes `%APPDATA%\irori.exe`, the WebView2 data
a dev build of Irori shares, and the launch-at-login value. Don't run either
next to an Irori you are using unless that is what you want.
