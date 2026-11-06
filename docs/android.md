# Irori on Android (card #225, #226)

The phone build is a board client: it pairs with your other devices (Settings → Devices → Join with a code), syncs your cards and boards over iroh, and lets you read, comment, answer blockers and ask. It runs no agents. The tray, single instance, launch at login and the updater are desktop only (`#[cfg(desktop)]` and `app/src/shell/tray_mobile.rs`).

## Toolchain (Mac)

```sh
brew install openjdk@17
brew install --cask android-commandlinetools
export JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home
export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools
yes | sdkmanager --sdk_root=$ANDROID_HOME --licenses
sdkmanager --sdk_root=$ANDROID_HOME "platform-tools" "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018"
export NDK_HOME=$ANDROID_HOME/ndk/27.2.12479018
rustup target add aarch64-linux-android   # in the repo, so the pinned toolchain gets it
```

## Voice input (whisper.cpp, #240)

The voice crate builds whisper.cpp with CMake for the phone. It needs, beside the env above:

```sh
P=$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64
export CXX_aarch64_linux_android=$P/bin/aarch64-linux-android26-clang++
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$P/sysroot --target=aarch64-linux-android26"
export CMAKE_TOOLCHAIN_FILE_aarch64_linux_android=$PWD/scripts/android/arm64.toolchain.cmake   # from the repo root
```

- The toolchain wrapper sets arm64-v8a and API 26; the NDK's own file alone builds 32-bit ARM, which fails to link.
- whisper-rs-sys comes from the StephenSHorton/whisper-rs fork (the `[patch]` in Cargo.toml) until upstream takes its cross-compile fix.
- The minimum Android is 8 (API 26): cpal records with AAudio. The mic permission (RECORD_AUDIO) is in app/gen/android's manifest, which is committed; Android asks at the first mic press.

## Build and install

```sh
(cd frontend && bun run build)
cd app
# app/gen/android is committed (its manifest carries the mic permission); run
# `cargo tauri android init --ci` only if it's missing
CARGO_PROFILE_DEV_OPT_LEVEL=3 CARGO_PROFILE_DEV_DEBUG=0 \
  cargo tauri android build --debug --target aarch64 --apk --ci
$ANDROID_HOME/platform-tools/adb install -r gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

On the phone: Settings → About phone → tap Build number 7 times, then Developer options → USB debugging on; plug in and allow the Mac. Use the optimised build above: an unoptimised one takes about 45 s to open a fully synced database (21 s optimised, 2026-11-01).

## Notes

- Data lives in the app's private files dir (`/data/data/com.stephenshorton.irori/files/irori`); `mobile_main` sets `IRORI_HOME`, `HOME` and `CLAUDE_CONFIG_DIR` there before the shell starts.
- The app library is linked with 16 KB pages (`.cargo/config.toml`); without it Android 15+ phones warn that the app isn't 16 KB compatible.
- Logs: `adb shell run-as com.stephenshorton.irori tail files/irori/irori.log`.
- Updates (card #270): the release's APK, checked against `SHA256SUMS`, goes to Android's install screen through `ApkInstallPlugin.kt` (app/gen/android) and the FileProvider's `irori/updates/` path; the manifest asks for `REQUEST_INSTALL_PACKAGES`. The updater checks TLS against bundled Mozilla roots on Android (the platform verifier would need JNI set up). See docs/releasing.md, Android.
- Notifications (card #395): only Talk's mentions of you and DMs to you (and agents' messages to you, which Talk shows as DMs); a plain channel post only bumps the unread count, and asks, held messages and cards waiting on you stay off the phone. `app/src/shell/notify_android.rs` is the notifier's toaster there, over `NotifyPlugin.kt` (app/gen/android), which posts to the "Mentions and direct messages" channel; Android 13+ asks for the permission at the first start. A tap opens that conversation: the plugin keeps the note's id and Rust takes it when the app resumes, or once the page listens for `app:open` (`app:open-ready`) after a cold start. An agent mentions you with `talk_post mention=["person"]`.

## In the background

Measured for card #580 (no-server.md §4.3, "find out what phones really do in the background").

**What the app does today.** Nothing keeps it running. The manifest asks only for INTERNET, the mic, installing updates and notifications: no service, no `FOREGROUND_SERVICE`, `WAKE_LOCK` or boot receiver, and the Gradle files pull in neither WorkManager nor Firebase. `MainActivity.kt` overrides only the insets, and `NotifyPlugin.kt` posts local notifications while the process is alive; there is no push. The Rust shell doesn't handle Tauri's mobile `WindowEvent::Suspended` or `Resumed` (`app/src/shell/mod.rs`), so going to the background tells the mesh nothing. The iroh endpoint and the timers keep going for as long as Android lets the process run, and stop when it freezes or kills it. After you open the app again, recovery waits on the ordinary timers (`crates/mesh/src/node.rs`, `Intervals`): connect and presence every 30 s, ping every 15 s, a vector every 30 s, and redial backoff from 1 s up to 60 s. A device counts as connected within 45 s of a ping. Arriving with card #580: Irori now syncs as far as it can when the app goes to the background.

**What was measured.** From the Mac only, read-only: its log (`~/.irori/irori.db`, opened with `mode=ro`) and `~/.irori/irori.log`. The phone wasn't touched. Every phone row is a row its Core wrote, and its HLC wall time (`hlc>>16`) says when, by the phone's clock. Nearly all are written by Irori itself (`actor irori`): `notification.posted` and `notification.read` (why `resolved`, card #482) when another device's row reaches it.

- The current phone, v0.12.1 then v0.12.2 (device f594f56f): 469 rows, 2026-11-05 18:36Z to 2026-11-06 00:47Z. Per UTC hour from 18h: 343, 44, 45, 20, 9, 6, 2. Four cold starts (`device.started`): 18:36:08, 20:00:00, 20:06:28 (the update) and 21:19:52.
- While it was open it kept up. For a notification key that the Mac or the PC also wrote, most of the phone's rows came 0–2 s after theirs (all 18 between 20:05Z and 20:32Z). Three came 39 s, 40 s and 51 s after, about one vector interval.
- After a break it caught up in one go. 2 min 41 s after the 21:19:52 cold start, it wrote 46 `notification.read` rows in the same second, some for things resolved hours before. At 23:20:02 it wrote a post 147 s after the Mac's and two reads 123 s and 328 s after the others', all in one second. That's a phone taking in what it missed at once, not reacting live.
- The Mac logged `pull from f594f56f…: connection lost` 75 times, all between 19:02Z and 01:00Z (about 12 an hour while the phone was in use). The last two were at 01:00:44Z, 14 min after its last row. Nothing logs a hello or a reconnect, so the quiet after that means no pull was running, not that nothing was tried.
- From 00:47Z to 05:40Z the phone wrote nothing, while the Mac and the PC wrote about 4,500 rows, 48 of them `notification.posted`.
- An earlier phone (45a587ce, a debug build, 2026-11-01 to 11-03) had gaps of 32, 62, 54, 103 and 479 min that ended without a cold start, so its process survived and carried on, and gaps of 84, 210, 632 and 1087 min that ended with one, so its process was gone.

What this proves: the phone syncs within a second or two while it's open; it can sit out minutes to hours and then take in everything at once; and Android sometimes keeps the process and sometimes ends it. What it only suggests: that a locked phone stops syncing. A device writes a notification row only if it hasn't already taken in another device's row for the same thing (`crates/core/src/service/notifications.rs`), so a missing phone row isn't proof it was asleep. But while open, the phone wrote one within 1–2 s dozens of times, and in the 5 h overnight it wrote none. The Mac also can't tell "in the background" from "open and idle", so the hourly counts follow use, not the screen.

**What it means for you.** A phone catches up when you open it, within a few minutes at most (a cold start opens the database first). It doesn't keep spaces fresh while locked, and what you do on it leaves the phone only while it's open. Another of your devices doesn't need the phone to stay current.

**Why: what Android does.** This is platform behaviour, from Android's docs:

- "Optimize for Doze and App Standby": once the screen is off and the phone is still for a while, Doze cuts apps off the network outside short maintenance windows, and defers their jobs and alarms.
- "App Standby Buckets": an app you haven't opened lately gets fewer chances to run.
- "Processes and app lifecycle" and the cached-apps freezer (Android 11 and later): a backgrounded app with no foreground service is a cached process. Android freezes it, so its timers and sockets stop, and kills it when it wants the memory. The earlier phone's gaps show both.

**What could change it later.** Options, not decisions:

- A foreground service ("Foreground services"; Android 14 needs a declared type) would keep the process and its connections running while locked. It needs `FOREGROUND_SERVICE` and a notification that stays up, and it costs battery.
- WorkManager ("Schedule tasks with WorkManager"): a periodic sync at most every 15 min, run when Android chooses. Doze still delays it.
- A push from a server could wake it, but §4.4 of docs/design/no-server.md chose not to run one.
