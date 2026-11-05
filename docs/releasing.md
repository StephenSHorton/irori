# Releasing Irori (Rust)

There is no CI. A release is built on the PC and the Mac with local scripts
and published as one GitHub release of `StephenSHorton/irori`. Installed
copies, the Go v0.5.x ones included, find it at their next start through
`/releases/latest`, download their asset, check it against `SHA256SUMS`,
and ask before they restart (crates/update; Go `internal/update`, whose
rules it keeps). The first Rust release is `v0.6.0` (architecture.md D11):
the Go updater takes it because `0.6.0` is newer than `0.5.x`.

## Build tools

Voice input (#240) builds whisper.cpp from source (`whisper-rs-sys`), so
every machine that builds Irori needs **cmake** and **libclang** (bindgen
generates the bindings; the bundled ones are Linux-only) besides the C/C++
compiler. When they are not on PATH, point the build at them:

- PC: `python -m pip install --user cmake libclang`, then
  `export CMAKE="$HOME/AppData/Roaming/Python/Python312/Scripts/cmake.exe"` and
  `export LIBCLANG_PATH="$(python -c "import clang,os;print(os.path.join(os.path.dirname(clang.__file__),'native'))")"`
  before `scripts/check.sh` or a release build. `.cargo/config.toml` sets
  the portable AVX2 CPU flags and `CL=/O2` (cmake-rs drops /O2 under MSVC;
  without it Whisper is ~40x slower).
- Mac: `brew install cmake` (Xcode's clang provides libclang).
- Android: docs/android.md.

## Cut a release

1. Run `sh scripts/check.sh` on the PC (and on the Mac when macOS code
   changed), then `sh scripts/agent-check.sh` on the Mac. The second is
   what a real Claude agent does with Irori's tools: on a throwaway
   `IRORI_HOME`, it takes an ask and closes it, raises a blocker when only
   the person can choose, and renames a card whose purpose a comment
   changed. It prints PASS or FAIL per scenario and exits non-zero on any
   failure. It runs real Claude sessions (API spend, a few minutes), which
   is why check.sh leaves it out. A FAIL means the tool descriptions or
   wake text no longer lead an agent to do what they say: fix the
   wording, not the check. An agent is not deterministic, so rerun a
   FAIL (`--only <n>`); one that comes back on a rerun is a real gap,
   and the transcript path it prints shows what the agent did instead.
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
5. Mac, Android (#269):

   ```sh
   scripts/release-android.sh 0.6.0 --publish
   ```

   It builds the frontend and the app for `aarch64-linux-android` with
   `cargo tauri android build --apk` (release, `versionName` X.Y.Z and
   `versionCode` major·1000000 + minor·1000 + patch, Tauri's formula, so
   every release is newer to Android), signs it with the release key
   (zipalign with 16 KB pages, then `apksigner`), checks the signature,
   version and alignment, then uploads `irori-<v>-android-arm64.apk` to the
   draft and adds its line to `SHA256SUMS`. Toolchain: docs/android.md. See
   "Android" below for the key.
6. Publish the draft: `scripts/release.ps1 -Version 0.6.0 -Finalize` (or
   `scripts/release-mac.sh 0.6.0 --finalize`). It refuses unless the draft
   has all six assets and a `SHA256SUMS` line for each of the five.

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
| `irori-<v>-android-arm64.apk` | the Android app (card #270: `irori-…-android-<arch>.apk`) |
| `SHA256SUMS` | all three; an asset without a line is never offered |

### Android (card #270)

A phone never downloads on its own (it may be on a metered network). At
start it reads `/releases/latest`. When a newer release has the APK and a
`SHA256SUMS` line for it, the bar says "Irori v<v> is available". Update
downloads it to `<data>/updates`, checks the hash (a mismatch deletes it and
says nothing was installed), and opens Android's install screen through the
app's FileProvider. The first time, Android asks to allow installing unknown
apps from Irori: that settings page opens, and Install goes on from there.
Android replaces the app in place, data kept, and its "App installed" screen
offers Open. Two things only the release can get right: each release's
`versionCode` must be higher than the last (Android refuses an equal or lower
one), and every release must be signed with the same key (a debug build on a
phone needs one uninstall before the first release goes on).

Testing it on an emulator: build two APKs with `IRORI_VERSION` set (for
example 0.6.1 and 0.6.2), a higher `versionCode` for the second (`--config
'{"version":"0.6.2","bundle":{"android":{"versionCode":2}}}'`), and
`IRORI_UPDATE_API=http://127.0.0.1:9250` so the release lookup goes to a local
fake of the GitHub API (serve `repos/StephenSHorton/irori/releases/latest`
plus the APK and `SHA256SUMS`, then `adb reverse tcp:9250 tcp:9250`). Release
builds never set `IRORI_UPDATE_API`.

`crates/update/tests/release_names.rs` reads the names out of the three
scripts and fails if they drift from the matchers or from each other.

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

Signing: a Mac release is always Developer ID signed. `release-mac.sh` uses
the `Developer ID Application` identity in the login keychain (with several,
name one in `MACOS_CODESIGN_IDENTITY`) and stops if there is none or the name
is anything else. It never falls back to ad hoc, for two reasons: the updater
refuses an update that isn't Developer ID signed over one that is, and macOS
keeps a permission (Screen Recording, the mic, folders) against the app's
designated requirement. A Developer ID build's names the Team ID, so a grant
lasts across updates; an ad hoc build's is its own cdhash, so every update
loses them. Executable and bundle are signed with the hardened runtime, a
timestamp and `app/macos/entitlements.plist`, whose one key,
`com.apple.security.device.audio-input`, voice needs: without it the mic
gives silence and no error. `app/macos/check-signed.sh` then checks all of
that on the zip that ships. `package.sh` called directly still signs ad hoc
with no identity, for scratch builds and `tools/rehearse`.

Notarization follows when credentials are set (`notarize.sh`, `xcrun
notarytool submit --wait`, then stapled): `APPLE_API_KEY_ID`,
`APPLE_API_ISSUER_ID` and `APPLE_API_KEY_P8` (a path), or `APPLE_ID`,
`APPLE_TEAM_ID` and `APPLE_APP_SPECIFIC_PASSWORD`. Without them the build is
signed but not notarized: updates and permissions work, and a fresh DMG
download gets Gatekeeper's warning.

## Android

The release key is `~/.config/irori/android/irori-release.jks` (alias
`irori`, override with `IRORI_ANDROID_KEYSTORE`), and its password is in the
login Keychain (service `irori-android-release`, account `irori`). It was
made once with `scripts/release-android.sh --new-key`, which refuses to
make another. Android installs an update only if it is signed with the same
key as the installed app, so a lost key means every phone has to uninstall
(losing its local data) and reinstall. Keep a backup of the file and the
password, and on a new Mac restore both instead of making a key.

A phone that has a debug build (docs/android.md) is signed with the debug
key, so the first release APK won't install over it: uninstall the debug
build once, then install the APK and pair the phone again.

After that, the app's update bar offers each release (see "Android (card
#270)" above). By hand: open the APK from the release page on the phone, or
`adb install -r irori-<v>-android-arm64.apk`.

## iOS

Releases ship no iOS build: Apple only installs an app that is signed with an Apple ID, so
each person builds Irori on their own Mac and installs it on their own iPhone.
docs/ios.md has the steps.

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
