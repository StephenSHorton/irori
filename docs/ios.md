# Irori on iPhone (card #269)

There is no iOS download. Apple only lets an app onto an iPhone once it's signed with an Apple ID, so you build Irori on your own Mac and install it on your own iPhone with your own Apple ID. Like the Android build, it's a board client: pair it from Settings → Devices → Join with a code. It runs no agents.

A free Apple ID works, but iOS stops opening the app after 7 days, so you rebuild and reinstall it from the Mac. A paid developer account ($99 a year) makes that a year.

## Toolchain (Mac)

- Xcode from the App Store. Open it once so it installs its parts.
- The iOS simulator runtime that matches your Xcode: `xcodebuild -downloadPlatform iOS` (several GB).
- `brew install xcodegen cocoapods`
- In the repo, so the pinned toolchain gets them: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
- `cargo install tauri-cli --version "^2" --locked`, plus bun for the frontend, as for the desktop build.

app/gen/apple is committed. It holds the Xcode project, the mic and local-network permission text, the iOS 16 minimum, and the SystemConfiguration link. Run `cargo tauri ios init --ci` only if that folder is missing. After you edit `app/gen/apple/project.yml`, run `xcodegen generate` in that folder.

## Try it in the simulator (no Apple ID)

```sh
(cd frontend && bun install --frozen-lockfile && bun run build)
cd app
rm -rf gen/apple/build   # a stale Irori.app there makes the CLI fail to replace it
CARGO_TARGET_DIR=$PWD/../target cargo tauri ios build --target aarch64-sim --debug --ci
xcrun simctl boot "iPhone 18 Pro Max"        # any iPhone from `xcrun simctl list devices available`
xcrun simctl install booted gen/apple/build/arm64-sim/Irori.app
xcrun simctl launch booted com.stephenshorton.irori
open -a Simulator
```

## Put it on your iPhone

1. Sign in to Xcode once: Xcode → Settings → Accounts → + → Apple ID. A free Apple ID shows as a "Personal Team".
2. On the iPhone, turn on Settings → Privacy & Security → Developer Mode. It restarts. Then plug it into the Mac and tap Trust.
3. Find your team ID: Xcode → Settings → Accounts → your Apple ID → the team's ID. Or run `security find-identity -v -p codesigning` after Xcode has made an "Apple Development" certificate.
4. Build for the phone with that team. If Xcode says the bundle identifier is taken (it belongs to whoever registered it first), give it your own, as in the second command:

   ```sh
   (cd frontend && bun run build)
   cd app && rm -rf gen/apple/build
   APPLE_DEVELOPMENT_TEAM=<team id> CARGO_TARGET_DIR=$PWD/../target \
     cargo tauri ios build --target aarch64 --debug --ci
   # or, with your own identifier:
   APPLE_DEVELOPMENT_TEAM=<team id> CARGO_TARGET_DIR=$PWD/../target \
     cargo tauri ios build --target aarch64 --debug --ci --config '{"identifier":"com.<you>.irori"}'
   ```

5. Install it: `xcrun devicectl list devices` gives the phone's ID. Then run `xcrun devicectl device install app --device <id> gen/apple/build/arm64/Irori.app`.
6. A free Apple ID only: the first time, iPhone Settings → General → VPN & Device Management → your Apple ID → Trust.

Alternatively, open `app/gen/apple/irori-app.xcodeproj` in Xcode, choose your team under Signing & Capabilities, pick the phone and press Run.

## Notes

- Data lives in the app's own Application Support folder (`app/src/lib.rs`, `mobile_main`). Deleting the app deletes it. Pair again to get your cards back.
- netdev and libc: the workspace patches netdev with a vendored 0.46.3 (vendor/netdev) and holds libc at 0.2.186, because newer versions don't build iroh's networking for iOS. Cargo.toml has the details.
- Not yet tried on a real iPhone: voice input (whisper.cpp built for iOS) and finding devices on the local network. iOS asks for the mic and local-network permissions on first use.

## In the background

Written for card #580 (no-server.md §4.3, "find out what phones really do in the background"). No iPhone was measured; the numbers for a real phone are on Android (docs/android.md, In the background). What follows is the code and Apple's documented behaviour.

**What the app does today.** Nothing asks iOS for time in the background. Info.plist has no `UIBackgroundModes`, the entitlements file is empty (no `aps-environment`, so no push), nothing registers a `BGTaskScheduler` task, and the only sources are `main.mm` and the bindings. The Rust shell doesn't handle Tauri's mobile `WindowEvent::Suspended` or `Resumed` (`app/src/shell/mod.rs`), so leaving the app tells the mesh nothing. After you come back, recovery waits on the ordinary timers (`crates/mesh/src/node.rs`, `Intervals`): connect and presence every 30 s, ping every 15 s, a vector every 30 s, and redial backoff from 1 s up to 60 s. Arriving with card #580: Irori now syncs as far as it can when the app goes to the background.

**What iOS does.** This is platform behaviour, from Apple's docs:

- "Preparing your UI to run in the background" and "Managing your app's life cycle": an app with no background mode is suspended within seconds of leaving the foreground. A suspended app runs no code, so its timers stop and its connections go quiet. iOS may end it later to free memory, which makes the next open a cold start.
- "Extending your app's background execution time": `beginBackgroundTask` buys about 30 s to finish work after leaving the foreground.
- "Using background tasks to update your app": a `BGAppRefreshTask` runs when iOS chooses, based on how often you use the app. It's short and isn't guaranteed.
- "Pushing background updates to your app": a silent push can wake an app, but it comes from a server through APNs.

**What it means for you.** As on Android, only more so: an iPhone catches up when you open it, and doesn't keep spaces fresh while locked.

**What could change it later.** Options, not decisions:

- `beginBackgroundTask` around the sync when the app leaves the foreground, so it gets its 30 s.
- A `BGAppRefreshTask` that opens the mesh, syncs and stops: an occasional catch-up while locked, when iOS allows it.
- Silent push needs a server, which §4.4 of docs/design/no-server.md chose not to run.
