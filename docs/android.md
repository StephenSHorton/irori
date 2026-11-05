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
- Notifications (card #395): only Talk's mentions of you and DMs to you (and agents' messages to you, which Talk shows as DMs); a plain channel post only bumps the unread count, and asks, held messages and cards waiting on you stay off the phone. The one exception (card #541): word that no agent can start for something you posted, because no computer can take it. `app/src/shell/notify_android.rs` is the notifier's toaster there, over `NotifyPlugin.kt` (app/gen/android), which posts to the "Mentions and direct messages" channel; Android 13+ asks for the permission at the first start. A tap opens that conversation: the plugin keeps the note's id and Rust takes it when the app resumes, or once the page listens for `app:open` (`app:open-ready`) after a cold start. An agent mentions you with `talk_post mention=["person"]`.
