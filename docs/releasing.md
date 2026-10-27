# Releasing Irori

Irori ships the way Toru does: a version tag builds a per-user Windows
installer on GitHub Actions and publishes a GitHub Release, and every
installed Irori finds that release the next time it starts, downloads it,
checks it and asks before it restarts.

## Cut a release

1. Make sure `main` is green in CI and has everything the release needs.
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

`.github/workflows/release.yml` has two jobs.

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

**release** (ubuntu-latest) writes `SHA256SUMS` over both assets and creates
the GitHub Release `Irori vX.Y.Z` with generated release notes.

The asset names and the `SHA256SUMS` file are a contract with
`internal/update/release.go`: the updater takes the asset whose name ends in
`-installer.exe` and contains `windows`, and refuses to offer a release that
has no `SHA256SUMS` line for it. `TestReleaseWorkflowAssetNames` reads the
workflow and fails if the two drift apart.

There is no code signing yet, so Windows SmartScreen warns on the first
manual install. Silent updates are not affected. macOS is not built yet (see
the TODO in the workflow); the Mac app reports "updates not supported".

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

## Test an update end to end

The updater only reads real GitHub releases, so a full test uses two of
them.

1. Release **vA** (for example `v0.3.0`) and install it from the release
   page by running `irori-0.3.0-windows-amd64-installer.exe`. Check
   `%LOCALAPPDATA%\Programs\Irori\irori.exe version` and the badge in the
   app header: both say 0.3.0.
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
