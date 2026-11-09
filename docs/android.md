# Irori on Android (card #225, #226)

The phone build is a client for your spaces: it pairs with your other devices (Settings → Devices → Join with a code), syncs your spaces, boards and cards over iroh, and lets you read, comment, answer blockers and ask. It runs no agents. The tray, single instance, launch at login and the updater are desktop only (`#[cfg(desktop)]` and `app/src/shell/tray_mobile.rs`).

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
- Proving the phone holds a row, without touching it: a release build's data can't be read (`run-as` needs a debug build), but the phone's own rows reach every peer. To prove it holds row N of device D, look in another device's log for a `notification.read` row the phone wrote naming a blocker that was resolved in D's row N or a later one. The phone could only mark it read if it held that row, and ingest is gapless per device, so it holds every lower seq of D as well, N included.
- Updates (card #270): the release's APK, checked against `SHA256SUMS`, goes to Android's install screen through `ApkInstallPlugin.kt` (app/gen/android) and the FileProvider's `irori/updates/` path; the manifest asks for `REQUEST_INSTALL_PACKAGES`. The updater checks TLS against bundled Mozilla roots on Android (the platform verifier would need JNI set up). See docs/releasing.md, Android.
- Notifications (card #395): only Talk's mentions of you and DMs to you (and agents' messages to you, which Talk shows as DMs); a plain channel post only bumps the unread count, and asks, held messages and cards waiting on you stay off the phone. The one exception (card #541): word that no agent can start for something you posted, because no computer can take it. `app/src/shell/notify_android.rs` is the notifier's toaster there, over `NotifyPlugin.kt` (app/gen/android), which posts to the "Mentions and direct messages" channel; Android 13+ asks for the permission at the first start. A tap opens that conversation: the plugin keeps the note's id and Rust takes it when the app resumes, or once the page listens for `app:open` (`app:open-ready`) after a cold start. An agent mentions you with `talk_post mention=["person"]`.

## Checking a release on the phone

For a "check it on the real phone after the release" step:

- First run `git merge-base --is-ancestor <merge-commit> v<latest>`. A merge can land minutes after a release is cut and miss it.
- The phone's app is release-signed, so there is no `run-as`. Read the installed version with `adb shell dumpsys package com.stephenshorton.irori | grep versionName`.
- A foldable has two displays, and `adb exec-out screencap -p` shoots only one. Pass `-d <id>`, with the ids from `adb shell dumpsys SurfaceFlinger --display-id`.
- A release build has no WebView devtools, so there is no DOM to read: check by screenshots.
- The phone may be in the person's hand. Before sending input, read `dumpsys power` for `lastUserActivityTime`; if they used it a moment ago, wait or ask. Put the screen back as you found it (off, locked) when you are done.

## Calls

A call or huddle (card #654) keeps running with the screen off or Irori in the background, through a foreground service.

- `CallService.kt` (app/gen/android) is a foreground service of type microphone. While a call runs it shows an ongoing, silent notification in the "Calls" channel, with a Hang up button, and holds a partial wake lock. Hang up, or swiping Irori away from recents, ends the call.
- `CallPlugin.kt` starts and stops it. On start it also puts the phone in its voice-call audio mode (`MODE_IN_COMMUNICATION`), takes audio focus, and makes the volume keys set the call's volume.
- In Rust, `app/src/shell/call_android.rs` is the call layer's `call::Platform` on the phone (`Phone`). The call layer calls it from its own threads, never Android's main thread, where the Kotlin side answers.
- Permissions in the manifest: `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_MICROPHONE` and `WAKE_LOCK`, beside the mic (`RECORD_AUDIO`) and `MODIFY_AUDIO_SETTINGS`.
- Android only lets a microphone service start while Irori is on screen and the mic is allowed. Without the mic the call is refused ("mic-denied"); started from the background, Android 12+ refuses it ("background"). Android 14+ can still refuse it as it starts; the service catches that and tells Rust, so the call ends instead of the app crashing.
- Without the notification permission (Android 13+) the service still runs; only its notification is hidden.
- Routing: speakerphone is the v1 route. cpal opens its AAudio output with media usage, and the earpiece route (`setCommunicationDevice`) moves only voice-call streams, so asking for the earpiece may leave the sound on the speaker. A voice-call output needs AAudio's usage setters, which need API 28 (see docs/research/08-voice-calls).

### Whisper during calls

A huddle's transcript (card #656, docs/design/call-transcripts.md) runs Whisper base.en on the phone, fed by the call's own cleaned mic.

- Measured on the Pixel 10 Pro Fold (Tensor G5): base.en's final pass takes 5.7–6.2 s on 2 threads and 3.7–4.3 s on 4.
- So calls use 4 threads on Android (`voice::Options::call()`; 2 elsewhere), and a line lands about 4 s after it's said.

To measure again, cross-build the bench with the voice env above (plus a linker for a plain cargo build), push it with the C++ runtime, the model and a 16 kHz mono 16-bit WAV of speech, and run it there:

```sh
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$P/bin/aarch64-linux-android26-clang
cargo build --release -p irori-voice --example bench --target aarch64-linux-android
adb push target/aarch64-linux-android/release/examples/bench /data/local/tmp/
adb push $P/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so /data/local/tmp/
adb push ggml-base.en.bin speech.wav /data/local/tmp/
adb shell 'cd /data/local/tmp && LD_LIBRARY_PATH=. ./bench ggml-base.en.bin speech.wav'
```

It times the first 1, 4 and 8 s of the WAV on 2 and 4 threads.

## In the background

Measured for card #580 (no-server.md §4.3, "find out what phones really do in the background").

**What the app does today.** Nothing keeps it running, except during a call. The manifest asks for INTERNET, the mic, installing updates and notifications, and, for calls only (card #654, see Calls above), `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_MICROPHONE` and `WAKE_LOCK`. `CallService` is the one service, and it runs only while a call does: there is no boot receiver, and the Gradle files pull in neither WorkManager nor Firebase. `MainActivity.kt` overrides only the insets, and `NotifyPlugin.kt` posts local notifications while the process is alive; there is no push. When the app goes to the background, the Rust shell handles Tauri's mobile `WindowEvent::Suspended` (`app/src/shell/sync_out.rs`, card #580): it sends this device's newest rows and waits up to 2 s for another device to say it holds them, so the last thing you did has left the phone. `Resumed` nudges the mesh to reconnect. Nothing keeps it syncing after that. The iroh endpoint and the timers keep going for as long as Android lets the process run, and stop when it freezes or kills it. After you open the app again, the reconnect starts at once, and the rest follows the ordinary timers (`crates/mesh/src/node.rs`, `Intervals`): connect and presence every 30 s, ping every 15 s, a vector every 30 s, and redial backoff from 1 s up to 60 s. A device counts as connected within 45 s of a ping.

**What was measured.** From the Mac only, read-only: its log (`~/.irori/irori.db`, opened with `mode=ro`), the space logs in `~/.irori/spaces/` and `~/.irori/irori.log`. The phone wasn't touched. Every phone row is a row its Core wrote, and its HLC wall time (`hlc>>16`) says when, by the phone's clock. Most are written by Irori itself (`actor irori`, 453 of the 469 in `irori.db`): 252 are `notification.posted` and `notification.read` (why `resolved`, card #482), written when another device's row reaches it, and most of the rest are `message.sent` and `delivery.state` from the first sync. Counts below are rows unless they say keys; one `notification.read` row can carry many keys.

- The current phone, v0.12.1 then v0.12.2 (device f594f56f): 469 rows in `irori.db`, 2026-11-05 18:36Z to 2026-11-06 00:47Z. Per UTC hour from 18h: 343, 44, 45, 20, 9, 6, 2. Hour 18 is mostly the first sync, not use: `message.sent` and `delivery.state` in step around 18:47, then 115 `notification.posted` rows around 18:55 for things from hours before. Four cold starts (`device.started`): 18:36:08, 20:00:00, 20:06:28 (the update) and 21:19:52.
- While it was open it often kept up, but not always. Pairing each notification key the phone wrote with the first row another device wrote for it gives 203 pairs: 55 within 3 s, 47 at 3–60 s, 28 at 1–10 min and 73 over 10 min. Most of the slow ones are the catch-ups below. The best window was 20:05Z to 20:32Z, where all 18 came 0–2 s after the other device's. Elsewhere it was slower while plainly in use: at 19:00:28 reads came 41 s, 90 s and 752 s after, and from 21:30Z to 00:05Z the lags were 2–8 s, 39 s, 173 s and 648 s.
- After a break it caught up in one go. At 19:02:45 one `notification.read` row carried 62 keys, most 15 s or 136 s behind the other devices. At 21:22:33, 2 min 41 s after the 21:19:52 cold start, it wrote 8 `notification.read` rows covering 52 keys in the same second; 40 or so of those had been read on another device about 20 h (71,868 s) before. At 23:20:02 it wrote a post 147 s after the Mac's and two reads 123 s and 328 s after the others', all in one second. That's a phone taking in what it missed at once, not reacting live.
- The Mac logged `pull from f594f56f…: connection lost` 75 times, all between 19:02Z and 01:00Z (about 12 an hour while the phone was in use). The last two were at 01:00:44Z, 14 min after its last row. Nothing logs a hello or a reconnect, so the quiet after that means no pull was running, not that nothing was tried.
- From 00:47Z to 05:40Z the phone wrote nothing, while the Mac and the PC wrote about 4,500 rows, 48 of them `notification.posted`.
- An earlier phone (45a587ce, a debug build, 2026-11-01 19:46Z to 11-03 21:57Z), its rows merged from `irori.db` and every space log. Of its gaps over 25 min, 54, 103 and 152 min ended without a cold start (no `device.started` within hours), so its process survived and carried on. Gaps of 210, 461 and 1087 min ended with one (the next row is `device.started`), so its process was gone. A 477-min gap is unclear: its next row is ordinary, then `device.started` 2 min later.

What this shows: the phone often syncs within a second or two while it's open, and sometimes takes up to minutes; it can sit out minutes to hours and then take in everything at once; and Android sometimes keeps the process and sometimes ends it. What it only suggests: that a locked phone stops syncing. A device writes a notification row only if it hasn't already taken in another device's row for the same thing (`crates/core/src/service/notifications.rs`), so a missing phone row isn't proof it was asleep. But while open, the phone wrote one within 3 s dozens of times, and in the 5 h overnight it wrote none. The Mac also can't tell "in the background" from "open and idle", so the hourly counts after the first sync follow use, not the screen.

**What it means for you.** A phone catches up when you open it, within a few minutes at most (a cold start opens the database first). It doesn't keep spaces fresh while locked, and what you do on it leaves the phone only while it's open. Another of your devices doesn't need the phone to stay current.

**Why: what Android does.** This is platform behaviour, from Android's docs:

- "Optimize for Doze and App Standby": once the screen is off and the phone is still for a while, Doze cuts apps off the network outside short maintenance windows, and defers their jobs and alarms.
- "App Standby Buckets": an app you haven't opened lately gets fewer chances to run.
- "Processes and app lifecycle" and the cached-apps freezer (Android 11 and later): a backgrounded app with no foreground service is a cached process. Android freezes it, so its timers and sockets stop, and kills it when it wants the memory. The earlier phone's gaps show both.

**What could change it later.** Options, not decisions:

- A foreground service ("Foreground services"; Android 14 needs a declared type) would keep the process and its connections running while locked. It needs `FOREGROUND_SERVICE` and a notification that stays up, and it costs battery. Calls have one now (card #654), but only for the length of a call; a sync service would need a type of its own (dataSync, which Android 15 limits to 6 hours a day).
- WorkManager ("Schedule tasks with WorkManager"): a periodic sync at most every 15 min, run when Android chooses. Doze still delays it.
- A push from a server could wake it, but §4.4 of docs/design/no-server.md chose not to run one.
