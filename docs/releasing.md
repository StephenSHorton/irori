# Releasing Irori (Rust)

There is no CI. A release is built on the PC and the Mac with local scripts
and published as one GitHub release of `StephenSHorton/irori`. Installed
copies, the Go v0.5.x ones included, find it at their next start through
`/releases/latest`, download their asset, check it against `SHA256SUMS`,
and ask before they restart (crates/update; Go `internal/update`, whose
rules it keeps). The first Rust release is `v0.6.0` (architecture.md D11):
the Go updater takes it because `0.6.0` is newer than `0.5.x`.

## Cut a release

1. Run `sh scripts/check.sh` on the PC (and on the Mac when macOS code
   changed).
2. Tag and push: `git tag v0.6.0 && git push origin v0.6.0`.
3. PC, PowerShell:

   ```powershell
   powershell -NoProfile -File scripts/release.ps1 -Version 0.6.0 -Publish
   ```

   It builds the frontend, `irori.exe` with `IRORI_VERSION=0.6.0` (what
   `irori version`, the API, the lockfile and the updater report) and the
   version resource from `TAURI_CONFIG`, checks `irori version` prints
   `irori 0.6.0`, builds the installer from `app/nsis/irori.nsi` with
   makensis (with Microsoft's WebView2 bootstrapper inside, as Go's had),
   zips the bare exe, writes `SHA256SUMS`, and creates a **draft** release
   with the three files. `-SkipChecks` skips step 1's checks;
   without `-Publish` it is a dry run that stops at `dist/release/v0.6.0`.
4. Mac:

   ```sh
   scripts/release-mac.sh 0.6.0 --publish
   ```

   It builds `irori` for `aarch64-apple-darwin` (deployment target 12.0),
   assembles and signs `Irori.app` (`app/macos/package.sh`), runs the Go
   updater's bundle checks on the zip (`app/macos/check-bundle.sh`), the
   smoke test (`irori version`; with `IRORI_SMOKE_GUI=1` also the app still
   running 20 s after a start on a throwaway `IRORI_HOME`, which leaves a
   Keychain item and goes to the real network, so it is off by default),
   then uploads the `.app.zip` and `.dmg` to the draft and adds their lines
   to its `SHA256SUMS`.
5. Publish the draft: `scripts/release.ps1 -Version 0.6.0 -Finalize` (or
   `scripts/release-mac.sh 0.6.0 --finalize`). It refuses unless the draft
   has all five assets and a `SHA256SUMS` line for each of the four.

With `-Publish` / `--publish` both scripts refuse to build unless HEAD is
the tag's commit and the tree has no uncommitted changes (a dry run only
warns), so both platforms' assets come from the tag. Both build with
`--features tauri/custom-protocol`, as `tauri build` would, so the shipped
exe is not a `cfg(dev)` build. The WebView2 bootstrapper is packed only
when its Authenticode signature is valid and Microsoft's.

A version with a `-` (`0.6.0-rc.1`) becomes a prerelease, which
`/releases/latest` never returns, so it is for hand testing only. Drafts are
never returned either, which is why nothing installed sees a release before
step 5.

## The contract with installed copies

| Asset | Picked by |
|---|---|
| `irori-<v>-windows-amd64-installer.exe` | the Windows updater (ends `-installer.exe`, contains `windows`) |
| `irori-<v>-windows-amd64.zip` | nobody (portable exe) |
| `irori-<v>-macos-arm64.app.zip` | the macOS updater (`irori-…-macos-<arch>.app.zip` or `-macos-universal.app.zip`) |
| `irori-<v>-macos-arm64.dmg` | nobody (first install) |
| `SHA256SUMS` | both; an asset without a line is never offered |

`crates/update/tests/release_names.rs` reads the names out of the two
scripts and fails if they drift from the matchers.

## What the installer does

`app/nsis/irori.nsi` is the Go installer's `project.nsi` ported, per-user
and without a UAC prompt:

- installs to `%LOCALAPPDATA%\Programs\Irori\irori.exe`, the path the Go
  build used, so the update replaces Go's exe in place;
- registers the uninstaller under the same key, `Stephen HortonIrori`, so
  Settings > Apps keeps one entry;
- always creates the Start menu shortcut, and the Desktop one only on a
  fresh install;
- silent installs (`/S`) wait for `/WAITPID=<pid>` (60 s), probe the exe for
  writing, rename a still-held exe (an `irori mcp` child) to
  `irori.exe.old[1-9]`, and relaunch the new Irori unless `/NORELAUNCH`; if
  the exe can be neither written nor renamed they change nothing, relaunch
  the old Irori and exit 3;
- the uninstaller removes the install folder, both shortcuts, the WebView2
  data of both builds (`%APPDATA%\irori.exe` from Wails,
  `%LOCALAPPDATA%\com.stephenshorton.irori` from Tauri) and the
  launch-at-login value `Run\Irori`.

Data (`%LOCALAPPDATA%\irori`) is not part of the install and survives
updates and uninstalling.

`scripts/installer-check.ps1` builds the installer with `-DIRORI_SCRATCH`
(nothing outside the install folder: no shortcuts, registry or WebView2
step) and runs every silent path against dummy exes in a scratch install
folder, then compiles (only) the release-mode installer with and without a
bootstrapper. Never run a normally built installer next to an Irori you use: it
installs for real even with `/D=`, which moves only the files.

## macOS

`app/macos/Info.plist` keeps the Go bundle's keys: `CFBundleIdentifier
com.stephenshorton.irori`, `CFBundleExecutable irori`,
`LSMinimumSystemVersion 12.0.0`, `NSLocalNetworkUsageDescription`.
`package.sh` stamps `CFBundleShortVersionString` and `CFBundleVersion` to
the bare `X.Y.Z` and zips with `ditto -c -k --keepParent`, so the zip holds
exactly one `Irori.app` at its top, which is what the updater requires.

Signing: without `MACOS_CODESIGN_IDENTITY` the bundle is signed ad hoc, as
the Go releases are. With a Developer ID identity, executable and bundle are
signed with the hardened runtime and a timestamp, then notarized and stapled
(`notarize.sh`, `xcrun notarytool submit --wait`) when notarization
credentials are set: `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID` and
`APPLE_API_KEY_P8` (a path), or `APPLE_ID`, `APPLE_TEAM_ID` and
`APPLE_APP_SPECIFIC_PASSWORD`. Once a release is Developer ID signed, keep
signing: the updater refuses an unsigned update over a signed app.

## Launch at login across the cutover

The installer and the `.app` swap keep the Go paths, so a registration the
Go build made (`HKCU\…\Run\Irori` = `<exe> --startup`, SMAppService, or
`~/Library/LaunchAgents/Irori.plist`) keeps working for the Rust build.
`update::login::plan` is the start-up check that brings a registration for
the running exe to the Rust form (value `Irori`, `--startup`); the GUI start
runs it through `shell::login::LoginItem::cutover` (Windows; tested on a
scratch HKCU key, not yet on a real Go install). A value pointing at
another exe (a dev build) is left until it is toggled from the installed
copy. On macOS 13+ SMAppService starts the app without `--startup`, so a
login launch opens the window, as it did with the Go build.
