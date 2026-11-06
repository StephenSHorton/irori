# Irori on iPhone (card #269)

There is no iOS download. Apple only lets an app onto an iPhone once it's signed with an Apple ID, so you build Irori on your own Mac and install it on your own iPhone with your own Apple ID. Like the Android build, it's a client for your spaces: pair it from Settings → Devices → Join with a code. It runs no agents.

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
