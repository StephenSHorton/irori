# #571 Screen sharing in Rust: survey (2026-11-06), fact-checked

**Legend:** **[V]** means verified this session (crates.io API, GitHub API, cloned source or a fetched page). **[I]** means inferred, or based on background knowledge that was not re-checked. **[FC✓]** marks a claim the fact-checker re-verified against a primary source on 2026-11-06. All crate versions, dates and download counts come from the crates.io API today. "Recent" is crates.io's 90-day download count.

## TL;DR
- **Use H.264 everywhere.** Each platform encodes it with its own hardware encoder: VideoToolbox on the Mac, Media Foundation on Windows, MediaCodec on Android. Every Irori webview can decode it with **WebCodecs `VideoDecoder`**: WKWebView on Safari/WebKit 16.4+, WebView2 on Chromium 94+, Android System WebView on Chrome 94+. [V: MDN BCD, FC✓ `safari 16.4`, `chrome 94`, `webview_android`/`webview_ios` mirror]
- **Send video on iroh QUIC *streams*, not datagrams.** Use one uni stream per GOP and request keyframes on a control stream. Keep datagrams for voice. [I]
- **The best codec layer available is `moq-video`** (moq-dev, MIT OR Apache-2.0, 0.1.21 released 2026-11-05). [FC✓]
  - Its encoder API stands on its own: `Encoder::new(&Config)`, `encode(&Frame) -> Result<Vec<Encoded>>` where `Encoded` is `{timestamp, payload: Bytes, keyframe}`, plus `cut()` for keyframe-on-demand and `set_bitrate()`. [FC✓ `src/encode/encoder.rs`, `encoded.rs`]
  - It supports H.264 **and H.265**. [FC✓]
  - It already has ScreenCaptureKit, WGC/DXGI, VideoToolbox, Media Foundation, MediaCodec and openh264 backends. [V: source; FC✓ backend files exist]
  - It is young. It pulls in `moq-net`/`hang` types, and `set_bitrate` takes `moq_net::bandwidth::Rate`, so moq-net leaks into the public API. [FC✓]
- **MoQ over iroh is not something to adopt today.** Upstream moq removed its iroh transport on **2026-11-03** (PR #7345, "feat(tokio)!: drop the iroh backend", merged 2026-11-03T09:09Z). [FC✓] The only remaining MoQ-on-iroh path is n0's `iroh-live`/`iroh-moq`. It is not on crates.io, it calls itself an "Early tech preview", and its README says Windows "Builds in CI … Never run here". [FC✓]
- **macOS TCC:** with ad-hoc signing, the Screen Recording grant is lost on every rebuild (Apple DTS). [V in the original survey; not re-fetched, because Apple's forum returned a bot-verification page]
  - `SCContentSharingPicker` (macOS 14+) is *designed* as per-session consent and is *reported* to avoid the persistent grant.
  - The sources disagree. The `screencapturekit` crate's own README says capture "**always requires user permission**" and is "purely TCC-gated". [FC✓]
  - This is **not verified on macOS 15/26** and needs a 1-hour spike.
- **Relay:** the 124 ms relay RTT is fine for screen sharing, which can tolerate hundreds of ms of playout buffer. **Relay throughput is the unknown.**
  - A 1080p share at 5–15 fps needs about 0.5–2.5 Mbps sustained, with keyframe bursts on top.
  - Irori uses its own relays (`RelayMode::Custom`, "Never `presets::N0`"), not n0's rate-limited public ones. [FC✓ `crates/net/src/endpoint.rs:77,113`]
  - A sustained-throughput relay-only spike is needed.

---

## 1. Capture

### Crate table

| Crate | Version | Last release | Licence | Platforms | Fit | Risks |
|---|---|---|---|---|---|---|
| **`screencapturekit`** (doom-fish/screencapturekit-rs) | 11.1.2 | 2026-11-02 | MIT OR Apache-2.0 | macOS (picker 14.0+; recording/mic 15.0+) | **Best macOS capture crate.** Safe bindings, async API, IOSurface/CVPixelBuffer frames, and the **`SCContentSharingPicker` wrapper** (`src/content_sharing_picker.rs`, example `examples/11_content_picker.rs`). [FC✓] | macOS only. 244★, 3.27 M downloads total, 2.16 M recent, 0 open issues, last push 2026-11-02. [FC✓] Major versions move fast: 11.x, and the README notes contract changes in 10.0. [FC✓] **It is not pure Rust.** `build.rs` compiles a **Swift bridge** (`swift-bridge/`, SwiftPM, via `xcrun`), so it needs Xcode CLT and the macOS 13+ SDK at build time. [FC✓] Irori's code stays Rust, but this matters for the "Rust only" constraint and for CI. |
| `screencapturekit-rs` | — | — | — | — | No such crate on crates.io ("does not exist"). The crate is `screencapturekit`. [FC✓] | — |
| `objc2-screen-capture-kit` | 0.3.2 | 2025-10-04 | Zlib OR Apache-2.0 OR MIT | macOS | Raw auto-generated bindings. `moq-video` uses objc2 crates. [V] | Unsafe, low-level API. The objc2 repo has 114 open issues. [V] |
| **`windows-capture`** | 2.0.1 | 2026-08-08 | MIT | Windows 10+ | **Best Windows capture crate.** WGC + DXGI Desktop Duplication, cursor/border/dirty-region settings, and a `GraphicsCapturePicker`. [V] | Its built-in encoder goes through WinRT `MediaTranscoder` into a container, `ContainerSettingsSubType::MPEG4` by default (`src/encoder.rs:20,424-426`). [FC✓] Use it for capture only. 525★, 21 open issues, last push 2026-10-05. [FC✓] 2.57 M downloads. [FC✓] Needs `windows` ^0.62, which matches Irori's 0.62.2. [V; FC✓ Irori lock] |
| `xcap` | 0.9.8 | 2026-08-01 | Apache-2.0 | macOS, Windows, Linux (+OHOS) | Good for screenshots. Its own description calls video recording "(WIP)". [FC✓] | On macOS it uses **`CGWindowListCreateImage`** (`src/macos/capture.rs`) and **`AVCaptureScreenInput`** (`impl_video_recorder.rs`). [FC✓] These legacy APIs trigger extra consent alerts per Apple's release notes. [V] **Not suitable for streaming on the Mac.** |
| `scap` | 0.1.0-beta.1 | 2025-08-04 | MIT | macOS, Windows, Linux | It looked like the cross-platform answer in 2024. | **Stale.** CapSoftware/scap: last push 2025-08-05, 48 open issues, still beta. [FC✓] It pins `windows` ^0.58. [V] |
| `crabgrab` | 0.4.0 | 2024-06-14 | MIT OR Apache-2.0 | macOS, Windows | — | **Repo archived.** [FC✓] |
| `captrs` | 0.3.1 | 2020-12-04 | **AGPL-3.0** | Windows (DXGI), Linux (X11) | — | Dead, and AGPL is incompatible with Irori's MIT licence. [FC✓ licence; Irori `LICENSE` is MIT] |
| `scrap` | 0.5.0 | 2018-04-08 | MIT | macOS, Windows, X11 | — | Dead. Uses legacy CGDisplayStream. [V] RustDesk keeps an AGPL fork. [I] |
| `moq-video` (capture module) | 0.1.21 | 2026-11-05 | MIT OR Apache-2.0 | macOS SCK, Windows WGC/MF, Linux PipeWire/X11 | Capture, encode, decode and render are all in one crate. [V] | macOS capture calls **`CGPreflightScreenCaptureAccess` then `CGRequestScreenCaptureAccess`**, which is the persistent TCC grant, not the picker (`capture/screencapture.rs:127-131`). [FC✓] First release 2026-05-25, 4,154 downloads total. [FC✓] |
| `ndk` | 0.9.0 | 2024-04-26 | MIT OR Apache-2.0 | Android | Covers `AMediaCodec`, `ANativeWindow`, `AHardwareBuffer`. | No release in 2.5 years. [FC✓] Repo pushed 2026-10-30. [V] |
| `jni` | 0.22.4 | 2026-03-16 | MIT OR Apache-2.0 | Android | Calls into MediaProjection from Rust. [FC✓ version/date] | — |

### macOS: TCC, signing, and the picker
- **Ad-hoc vs Developer ID.** Apple DTS (Quinn, Aug 2025) says: *"Ad hoc signed code does not include a stable DR…"*. [V in original; the fact-checker could not re-fetch it (bot wall), but it is consistent with documented TCC/designated-requirement behaviour]
  - The fix is Apple Development or Developer ID signing.
  - The practical effect for Irori: until #537 lands, every rebuilt or updated build loses the Screen Recording grant.
  - Third-party docs add that you may need `tccutil reset ScreenCapture <bundle-id>`. [I]
- **Sequoia nag.** macOS 15 re-prompts apps that hold a persistent grant, and moved from weekly to monthly in a 15.x beta. [V, secondary sources] Apple tells developers to migrate to ScreenCaptureKit + `SCContentSharingPicker`. [V]
- **Does `SCContentSharingPicker` avoid the persistent grant? Unverified, and the evidence conflicts.**
  - Nonstrict (2023) says both yes and "even when using SCContentSharingPicker … the user needs to give authorization". [V]
  - Apple's ScreenCaptureKit overview says "Request screen recording permission … before capturing content". [V]
  - There are forum reports of notices on macOS 15 with the picker. [V via search summary]
  - The `screencapturekit` README states capture "always requires user permission" and is "purely TCC-gated". It is generic guidance, not picker-specific. [FC✓]
  - The original survey's working belief [I] was that the picker grants per-session access without a TCC entry. **The weight of the evidence is now against assuming this.**
  - **Spike:** build `11_content_picker`, ad-hoc sign it, run it with no Screen Recording entry, rebuild, and run again. About an hour.
- **Minimum OS.** The picker needs macOS 14+. Irori's `tauri.conf.json` sets only iOS `minimumSystemVersion` 16.0 and Android `minSdkVersion` 26, with no macOS minimum. [FC✓] So screen sharing would be gated at runtime on 14+. (The crate's Swift bridge itself targets macOS 12.3. [FC✓])

### Windows
WGC needs no permission prompt. It shows a yellow capture border, and `GraphicsCapturePicker` gives a system picker. [V for the API; I for the no-prompt behaviour] The PC was heavily loaded in the latency test, so it matters that WGC plus hardware MF encoding stays on the GPU. [I]

### Android (MediaProjection from Rust)
This part cannot be pure Rust, because MediaProjection has no NDK API. [I] The Android docs require:
- Consent through `createScreenCaptureIntent()` before every session. On Android 14+, reusing the intent or calling `createVirtualDisplay()` twice throws `SecurityException`. [V]
- A `mediaProjection` foreground service. [V]
- An `onStop` callback. Android 15 QPR1+ shows a status-bar chip and auto-stops projection on lock. [V]

**Plan:**
1. Add about 100 lines of Kotlin in the Tauri Android shell for consent and the foreground service.
2. Pass the `MediaProjection` to Rust through `jni`.
3. In Rust, create an `AMediaCodec` encoder input surface and turn it into a `Surface` with `ANativeWindow_toSurface`.
4. Call `createVirtualDisplay(surface)`, so frames go straight into the hardware encoder with no copies.

`moq-video` has a MediaCodec backend (`encode/backend/mediacodec.rs`). [FC✓] `iroh-live` ships a Kotlin + Rust Android demo ("Tested on device, two-way audio and video"). [FC✓] Its Android path uses Camera2, so there is no evidence it does screen capture. [FC✓ README `moq-media-android`: "Camera2 push bridge"]

**Recommendation:** treat phone-as-viewer as the real target and phone-as-sharer as optional, later work.

---

## 2. Encode

| Crate | Version | Last release | Licence | Platforms | HW/SW | Fit | Risks |
|---|---|---|---|---|---|---|---|
| **`moq-video`** | 0.1.21 | 2026-11-05 | MIT OR Apache-2.0 | macOS/iOS, Windows, Linux, Android | Both: videotoolbox, mediafoundation, mediacodec, openh264, nvenc, vaapi, v4l2 [FC✓ files] | **Strong.** `Encoder::new(&Config)`, `encode()`, `cut()`, `set_bitrate()`, `flush()`/`finish()`, `Gop::keyframe_every`. [FC✓] The MF backend sets low-latency mode and forced keyframes. [V] | Output is Annex-B with in-band SPS/PPS. [V] Depends on `moq-net`, `hang`, `moq-mux`, `kio`. [FC✓] Default features `["mediacodec", "nvidia"]`, so use `default-features=false`. [FC✓] Cargo.toml on main reads `version = "0.1.10"` while crates.io has 0.1.21. [FC✓, still unexplained] |
| `openh264` / `openh264-sys2` | 0.9.9 | 2026-10-15 | BSD-2-Clause | Win/mac/Linux; Android "usual shenanigans" | SW | Reliable fallback. About 2.06 M downloads. [FC✓] | **Licence:** compiling from source makes "the team … responsible for paying all applicable license fees". Cisco's binary has Cisco cover MPEG LA. [FC✓ issue #43 quoting Cisco] A commenter on #43 says the binary must be *"downloaded during/after installation … directly from Cisco"*, not bundled. [V: issue comment, not Cisco's licence text] 1080p CPU cost acceptable at 5–15 fps. [I] |
| `objc2-video-toolbox` | 0.3.2 | 2025-10-04 | Zlib OR Apache-2.0 OR MIT | macOS/iOS | HW | Raw VTCompressionSession bindings, used by `moq-video`. 255 k downloads. [FC✓] | Unsafe. VideoToolbox emits AVCC. [V] |
| `video-toolbox` (rust-media/apple-media-rs) | 0.3.1 | 2026-06-09 | MIT OR Apache-2.0 | macOS/iOS | HW | Higher-level wrapper. [I] | 38★, little adoption. [V] |
| `shiguredo_video_toolbox` | 2026.2.0-canary.4 | 2026-10-06 | Apache-2.0 | macOS | HW | Built for WebRTC/Sora. [I] | Canary only, 5★. [V] |
| `windows` (Media Foundation H.264 MFT) | 0.62.2 | 2025-10-06 | MIT OR Apache-2.0 | Windows | HW, with fallback to the MS software MFT | Irori already depends on it (0.62.2 in Cargo.lock). [FC✓] `moq-video`'s `mediafoundation.rs` is a worked example. [FC✓ exists] | Complex API. "N" editions need the Media Feature Pack. [I] |
| `ndk` (AMediaCodec) | 0.9.0 | 2024-04-26 | MIT OR Apache-2.0 | Android | HW | See §1. | Stale release. [FC✓] |
| `rav1e` | 0.8.1 | 2025-06-16 | BSD-2-Clause | all | SW AV1 | Royalty-free. | Too slow live on a loaded PC. Limited WebKit AV1 hardware decode. [I] **No.** |
| `svt-av1-enc` | 0.1.2 | 2025-02-18 | MIT OR Apache-2.0 | needs C library | SW AV1 | — | 33 recent downloads. [V] **No.** |
| `env-libvpx-sys` / `vpx-encode` | 5.1.3 / 0.6.2 | 2023-09-10 / 2022-08-31 | MPL-2.0 / MIT | system libvpx | SW VP8/VP9 | — | Stale. [V] `moq-video` uses `libvpx-native-sys` 5.0.17 (optional `vpx` feature, decode). [FC✓] **No.** |
| `ffmpeg-next` | 9.0.0 | 2026-08-05 | WTFPL wrapper; FFmpeg LGPL/GPL | all | Both | Can do everything. | Heavy native dependency, painful on Android, LGPL compliance work. [I] **No.** |

**Licence note [I, not legal advice].** OS-vendor H.264 encoders are generally considered covered by the vendor's licence. OpenH264 compiled from source is not covered by Cisco's licence.

**Codec choice:** H.264 Constrained Baseline or Main. It's the only codec with hardware encode on all three devices **and** WebCodecs decode in all three webviews.

---

## 3. Decode and display in the webview
Irori is Tauri 2.12.5. [FC✓ Cargo.lock]

| Option | Verdict |
|---|---|
| **WebCodecs `VideoDecoder` → canvas** | **Recommended.** MDN BCD: Safari 16.4 and Chrome 94; `webview_ios`, `webview_android` and Edge mirror. [FC✓] WKWebView therefore needs macOS 13.3+. [I] caniuse "partial" is about `AudioDecoder`. [V] Rust sends encoded frames over a Tauri `ipc::Channel` or raw `ipc::Response`. [I] |
| Decode in Rust, push raw frames | **No.** About 80–120 MB/s through IPC. iroh-live renders natively with egui/wgpu. [V; FC✓ `moq-media-egui` crate in workspace] |
| MJPEG | **LAN only**, as a debug fallback. [I] |

**Bitstream gotcha.** `moq-video` emits Annex-B. [V] WebCodecs reads a config with no `description` as Annex-B, and an `avcC` `description` as length-prefixed. [I] Normalise to AVCC plus a `description` in Rust. Probe `isConfigSupported` with `optimizeForLatency: true` and the codec string built from the encoder's own SPS (profile_idc, constraint flags, level_idc): both moq-video hardware encoders emit High, e.g. `avc1.640028` (see Results).

---

## 4. Transport over iroh
- **Datagrams are wrong for video. [I]** QUIC datagrams must fit in one packet, about 1.2 KB, while keyframes are 50–300 KB.
- **Use streams, MoQ-style, without MoQ. [I: design]**
  - Open one uni stream per GOP.
  - When the receiver falls behind, it `STOP_SENDING`s the old GOP, and the sender resets stale GOPs.
  - A control bi-stream carries `RequestKeyframe` (the sender calls `cut()`) and bitrate reports (`set_bitrate`).
  - Set stream priority below voice.
- **Relay leg [I].** While relayed, traffic travels over HTTPS/WebSocket (TCP) to the relay. That's fine for streams.
- **MoQ on iroh**
  - `moq-native` 0.20.0: "Deprecated: use moq-tokio instead". [FC✓]
  - PR #7345 (merged 2026-11-03) "Removes the iroh transport everywhere", because "iroh and noq hard-wire aws-lc-rs or ring". [FC✓]
  - Published `moq-tokio` 0.19.34 (2026-11-05) still declares optional `web-transport-iroh ^0.7`, so the removal isn't in a release yet. [FC✓]
  - `web-transport-iroh` 0.8.4 (2026-10-20) still exists. [FC✓] The same rationale may threaten it. [I]
  - **n0's `iroh-live`**: licence Apache-2.0 OR MIT per its README (GitHub's API detects only Apache-2.0). 140★, pushed 2026-11-05. [FC✓]
    - It provides `iroh-moq` plus `iroh-rooms` on **iroh 1.2.0** and **iroh-gossip 0.101.0**, the same majors Irori uses (Irori: iroh 1.4.1, gossip 0.101.0). [FC✓]
    - **It itself depends on `moq-video` 0.1.2, `moq-net`, `moq-mux`, `moq-tokio`, and `web-transport-iroh` 0.7.** [FC✓]
    - It isn't on crates.io (`iroh-live` and `iroh-moq` both "does not exist"). [FC✓]
    - Its README says "Early tech preview… Windows builds in CI but has never been run… the relay has no authentication". [FC✓]
    - Its README tells downstream users to copy a `[patch.crates-io]` block (to `Frando/moq@iroh-live`, "eleven open pull requests"), but Cargo.toml on main has no such block. The README is stale, or the patches have landed. [FC✓]
    - It enables iroh feature `tls-aws-lc-rs`. [FC✓] Irori's lockfile contains `ring` 0.17.14 and **no `aws-lc-rs`**, and its net crate pins rustls with `ring`. [FC✓] Adopting iroh-live would add aws-lc-rs (C/asm build) and a second crypto provider. [I on friction severity]
    - **Verdict:** watch, don't adopt.
  - `moq-transport` (cloudflare/moq-rs, 0.17.0, 2026-11-02): no iroh support. [FC✓ version/date; V for the rest]

---

## 5. Bandwidth and the relay path
- **Bitrates.**
  - BytePlus guidance: 1080p at 5 fps is about 2 Mbps; motion at 30 fps is about 4 Mbps. [V]
  - Multi.app saw text-heavy shares drop frames under a 2.61 Mbps cap. [V]
  - **Budget 1–2.5 Mbps at 5–15 fps, and 4 Mbps if video is playing.** Use a long GOP and keyframe-on-demand. [I]
- **Can the relay carry it?**
  - **Latency: yes**, with a 200–500 ms playout buffer. [I]
  - **Throughput: unknown.** Irori uses `RelayMode::Custom`, never `presets::N0`, and embeds `iroh-relay` with the `server` feature. [FC✓] A self-hosted `iroh-relay` has rate limiting off by default. [V: docs.iroh.computer]
  - The ceiling is the relay host's uplink times the number of viewers. The bandwidth-delay product is about 31 KB. [I]
  - With no SFU, the sharer uploads N×: about 5 Mbps worst case with 3 devices. [I]
- **Spike:** add a relay-only mode to `spike-call` that pushes 2 Mbps sustained, with a 200 KB burst every few seconds, on a uni stream, Mac↔PC.

---

## Recommended stack (A)
1. **Capture**
   - Mac: `screencapturekit` 11.x with `SCContentSharingPicker` (macOS 14+). Note that it compiles a Swift bridge, which needs Xcode CLT. Fall back to `SCShareableContent` plus the TCC grant once #537 lands.
   - Windows: `windows-capture` 2.0 (WGC, optionally `GraphicsCapturePicker`).
   - Android: a Kotlin shim, then MediaProjection → MediaCodec surface via `jni` + `ndk`. Phase 2.
2. **Encode:** `moq-video`, `default-features=false` (plus `mediacodec` on Android). Use `Encoder`/`Frame` only, with openh264 (`libloading`) as fallback. **Pin an exact version.**
3. **Transport:** iroh uni stream per GOP, a control bi-stream, priority below voice. No MoQ library.
4. **View:** WebCodecs `VideoDecoder` (H.264, AVCC + `description`) drawing to a canvas, fed through a Tauri `ipc::Channel`.

**Why:** `moq-video` already solved low-latency H.264 on VideoToolbox, Media Foundation and MediaCodec, with forced keyframes and bitrate control. The Rust crates are MIT/Apache. The two capture crates are the most actively maintained.

**Risks:**
- `moq-video` is 5 months old, has about 4 k downloads, and churns. Its public API exposes `moq_net` types.
- Its upstream just dropped iroh, which doesn't matter for the codec layer alone.
- Picker/TCC behaviour is unverified, and the evidence leans against it.
- Relay throughput is unmeasured.
- `screencapturekit` needs a Swift toolchain at build time.

## Runner-up (B)
Use the same capture crates and write the encoders yourself:
- `objc2-video-toolbox`
- `windows` 0.62 Media Foundation
- `ndk` AMediaCodec
- `openh264` 0.9.9 (`libloading`) as fallback

Adapt the patterns from `moq-video/src/encode/backend/{videotoolbox,mediafoundation,mediacodec}.rs` (MIT/Apache).
- **Pros:** no dependency on a fast-moving workspace.
- **Cons:** about 2–4× the work, and you own the unsafe FFI. [I]

Choose B if `moq-video` pulls in too much of `moq-net`/`hang`/`kio`. For a fully-Rust Mac build, B can also replace `screencapturekit` with `objc2-screen-capture-kit`.

**Watch:** `iroh-live` / `iroh-moq`.

## Results (#657, 2026-11-06)

What was built: `crates/share` (`irori-share`) on `moq-video =0.1.21` with `default-features = false, features = ["capture"]`, patched to `vendor/moq-video` for one fix (below). It is capture and encode only: a `Source` (one window or a display) becomes `Chunk`s, one Annex-B access unit each, with SPS/PPS in band on every keyframe. Capture is moq-video's (ScreenCaptureKit on the Mac, WGC on the PC), not the `screencapturekit` / `windows-capture` crates; encode is moq-video's VideoToolbox and Media Foundation backends. The picture is the source fitted into the 1920x1080 box with its aspect ratio kept (`share::fit`): a tall window stays tall, and the encoder is built at that size. Tools: `examples/share-dev.rs` lists windows and displays and times capture + encode; `tests/it/encode.rs` checks keyframes on request on the real hardware encoder; `examples/share-picker.rs` and `grant-test.sh` / `grant-walk.sh` test the Mac grant.

**Findings about moq-video 0.1.21** (from its source, unlike the survey's 0.1.10-era notes above):
- `Encoded` is `{timestamp, payload}` only: the `keyframe` flag is gone. `irori-share` reads it from the bitstream (an IDR NAL, type 5).
- Both hardware backends encode **H.264 High** (`kVTProfileLevel_H264_High_AutoLevel`, `eAVEncH264VProfile_High`), not Baseline/Main. WebCodecs decodes High (`avc1.64…`), so the viewer's codec string must say High.
- Rate control: VideoToolbox gets `AverageBitRate` only, no `DataRateLimits`, so on the Mac nothing caps a keyframe's size. Media Foundation is asked for CBR, low-latency mode, a mean bitrate and a GOP size through `ICodecAPI`; moq-video logs a refused value at debug level and carries on, so these are requests. On the PC's NVENC the keyframes came out a fifth of the Mac's size, which suggests CBR held.
- **Two GPUs (fixed, #666):** Media Foundation's hardware list on the PC (RTX 5080 plus the Ryzen's Radeon iGPU) put `AMDh264Encoder` first; moq-video took it, while its D3D11 device is on the default adapter (the NVIDIA), and `SetOutputType` failed with `0xC00D6D76`. `vendor/moq-video` enumerates with `MFTEnum2` filtered to adapter 0's LUID and falls back to the old list. Worth sending upstream.
- **Hardware only** (`Settings::hardware_only`) is enforced on the PC (Media Foundation is asked for hardware encoders only). On the Mac moq-video opens VideoToolbox with no encoder specification, so VideoToolbox prefers hardware but isn't made to use it; enforcing it means passing `RequireHardwareAcceleratedVideoEncoder` upstream.
- **A resize reopens:** WGC ends a capture when the window changes size, and moq-video's `read()` then returns `None`, meaning "open again", not "ended". `irori-share` turns that into `Error::Reopen` and `Capture::reopen`; on the PC the encoder must be rebuilt too, because Media Foundation binds the first capture's D3D11 device. The share really ends with `Error::Video(SourceUnavailable)` (the window closed, Stop in the menu bar, the grant taken away).
- **WGC ignores the size asked for** and hands over the window's own size; `Capture` scales it on the GPU to the fitted size. WGC delivers a frame only when the window changes (about 9.4 fps on a busy window at 10 fps).
- `capture::open` checks `CGPreflightScreenCaptureAccess` before every Mac stream and calls `CGRequestScreenCaptureAccess` when there is no grant (`capture/screencapture.rs`), so it always goes through the persistent grant. A picker-based share can't use moq-video's capture; it needs its own `SCStream` on the picker's filter, as `examples/share-picker.rs` does.
- **Audio:** nothing in moq-video sets `capturesAudio`, and ScreenCaptureKit's default is off, so a share carries no sound at all, Irori's included. WGC captures no audio. Sharing audio later means turning it on with `excludesCurrentProcessAudio` set, so Irori's own call voices and chimes stay out.
- **Frames stay on the GPU**, with no CPU readback. On the Mac the IOSurface `CVPixelBuffer` goes to VideoToolbox as is. On the PC moq-video converts each WGC BGRA texture into a new NV12 D3D11 texture, which Media Foundation takes as is, and `Capture` scales it on the GPU first when the size differs. A CPU I420 frame (the synthetic bench) is uploaded first.

**Numbers.** `share-dev`, release build. CPU is a percent of one core. On the Mac VideoToolbox encodes in a separate `VTEncoderXPCService` process launched for the client, counted separately ("enc svc"). Media Foundation encodes in-process. Capture compositing in WindowServer is shared with everything on screen and is not counted. "Ask → keyframe" runs from when the ask falls due, through the wait for the next picture, to the keyframe coming out of the encoder: what a viewer waits. At 10 fps that includes up to 100 ms of waiting for the next frame. The synthetic picture is 30 frames of dense "text" and a moving block, played in a loop: harder than most real shares.

Mac: MacBook Pro, M5 Max, 18 cores, macOS 26.6.2. PC: Ryzen 7 9850X3D (8 cores), RTX 5080 (NVENC through Media Foundation), Windows 11.

| Run | Backend | fps | Bitrate (target) | Keyframes | Other frames | Ask → keyframe | P-frame encode ms p50 / p95 | CPU (+ enc svc) |
|---|---|---|---|---|---|---|---|---|
| Mac synth 1080p, 20 s, no asks | videotoolbox | 10.04 | 837 kbit/s (1451) | 1 × 594 KB | 7.5 KB | — | 4.87 / 5.39 | 0.8 % (+0.6 %) |
| Mac synth 1080p, 30 s, ask every 5 s, 800 kbit/s from 20 s | videotoolbox | 10.03 | 1936 kbit/s (1451 → 800) | 6 × 855 KB avg | 7.2 KB | 5/5, 121 ms avg | 5.15 / 5.48 | 0.9 % (+0.5 %) |
| Mac synth 1080p, 20 s, ask every 2 s, 2500 kbit/s | videotoolbox | 10.04 | 4367 kbit/s (2500) | 10 × 917 KB avg | 9.2 KB | 9/9, 119 ms | 5.13 / 5.56 | 0.9 % (+0.6 %) |
| PC synth 1080p, 30 s, ask every 5 s, 800 kbit/s from 20 s | mediafoundation (NVENC) | 10.03 | 912 kbit/s (1451 → 800) | 6 × 179 KB avg | 8.0 KB | 5/5, 99 ms¹ | 0.71 / 1.03² | 2.0 % |
| PC window (Discord 1280x1392)³, 30 s, ask every 5 s | mediafoundation | 9.43 | 1005 kbit/s (1451) | 6 × 86 KB | 11.7 KB | 5/5, 108 ms¹ | 0.29 / 0.68² | 1.0 % |
| PC window (Task Manager 320x412)³, 30 s | mediafoundation | 9.41 | 575 kbit/s | 6 × 54 KB | 6.6 KB | 5/5, 108 ms¹ | 0.27 / 0.43² | 0.7 % |

¹ Measured by the first version of `share-dev`, from the frame the ask landed on to the keyframe, so it excludes the wait for that frame. That it is about one frame interval suggests Media Foundation forces the IDR one frame after the ask; the Mac's equivalent figure was 14 ms (the IDR came out on the same frame). ² Keyframes and P-frames mixed. ³ Before the aspect fix, so the window was stretched to 1920x1080; no yellow WGC border (moq-video asks for borderless) and no prompt. _Captured-window numbers on the Mac and the PC re-runs after the fixes: below._

What the numbers say:
- **CPU is not a concern** on either machine: about 1–2 % of one core all told.
- **Keyframes are the cost on the Mac:** 0.6–1 MB each. VideoToolbox spends what it likes on an IDR, so the bitrate holds only while keyframes are rare: asks every 5 s more than double the stream, and every 2 s it reaches 1.75× the target. NVENC's CBR keeps them at 50–180 KB. The sending layer has to ration keyframes: coalesce viewers' asks, at most one per few seconds, and a long `keyframe_every` (default 10 s).
- P-frames sit under the target, and a live bitrate change shows within a second (about 500 → 270 kbit/s on the Mac).

**Mac Screen Recording grant:** _filled in from the walk-through._

## Next steps (spikes, in order)
1. **Picker without TCC:** test `11_content_picker` on an ad-hoc-signed build, before and after a rebuild.
2. **Relay throughput:** 2 Mbps sustained plus bursts, relay-only, Mac↔PC, via `spike-call`.
3. **WebCodecs probe:** run `isConfigSupported` for High (`avc1.640028`, or the string from the encoder's real SPS) and decode an AVCC GOP in WKWebView, WebView2 and the Pixel's WebView.
4. **`moq-video` dependency weight:** `cargo tree` with `default-features=false` on the Mac and Windows, checking for `windows` or crypto (aws-lc-rs) conflicts.

## Sources
- crates.io API: https://crates.io/api/v1/crates/<name>, and https://crates.io/api/v1/crates/moq-tokio/0.19.34/dependencies
- https://github.com/doom-fish/screencapturekit-rs (README, `build.rs`, `swift-bridge/Package.swift`)
- https://github.com/NiiightmareXD/windows-capture/blob/main/src/encoder.rs
- https://github.com/nashaofu/xcap/tree/master/src/macos
- https://github.com/CapSoftware/scap
- https://github.com/AugmendTech/CrabGrab
- https://github.com/bryal/captrs
- https://github.com/quadrupleslap/scrap
- https://github.com/ralfbiedert/openh264-rs/issues/43
- https://github.com/rust-media/apple-media-rs
- https://github.com/madsmtm/objc2
- https://github.com/shiguredo/video-toolbox-rs
- https://github.com/moq-dev/moq/tree/main/rs/moq-video
- https://github.com/moq-dev/moq/pull/7345
- https://github.com/moq-dev/web-transport
- https://github.com/cloudflare/moq-rs
- https://github.com/n0-computer/iroh-live (README, Cargo.toml)
- https://developer.apple.com/forums/thread/795739
- https://developer.apple.com/forums/thread/760483
- https://developer.apple.com/documentation/screencapturekit
- https://developer.apple.com/documentation/screencapturekit/sccontentsharingpicker
- https://nonstrict.eu/blog/2023/a-look-at-screencapturekit-on-macos-sonoma/
- https://talk.tidbits.com/t/apple-reduces-excessive-sequoia-permission-requests-shifts-to-monthly/28621
- https://docs.openclaw.ai/platforms/mac/permissions
- https://developer.android.com/media/grow/media-projection
- https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/VideoDecoder.json
- https://caniuse.com/webcodecs
- https://docs.iroh.computer/relays/rate-limiting
- https://docs.byteplus.com/en/docs/byteplus-rtc/docs-80226
- https://multi.app/blog/making-illegible-slow-webrtc-screenshare-legible-and-fast
- Irori source (read-only): `crates/net/src/endpoint.rs`, `crates/net/Cargo.toml`, `app/tauri.conf.json`, `Cargo.lock`, `LICENSE`

---

## Fact-check notes
**Re-verified against primary sources, all correct:**
- crates.io version, date, licence and download figures for `screencapturekit`, `windows-capture`, `xcap`, `scap`, `crabgrab`, `captrs`, `moq-video`, `openh264`, `ndk`, `jni`, `objc2-video-toolbox`, `moq-native` (deprecated), `moq-tokio`, `web-transport-iroh` and `moq-transport`.
- `screencapturekit-rs`, `iroh-live` and `iroh-moq` are absent from crates.io.
- moq PR #7345: title, merge date and rationale.
- iroh-live: README wording (tech preview, Windows never run, no relay auth, Android tested), iroh 1.2 / gossip 0.101, `tls-aws-lc-rs`, and no `[patch]` block in Cargo.toml.
- CrabGrab is archived. scap stats. screencapturekit and windows-capture stars, issues and push dates.
- `moq-video` Encoder API, default features, dependencies, `CGRequestScreenCaptureAccess`, and Cargo.toml showing 0.1.10.
- windows-capture's MPEG4 default container. xcap uses `CGWindowListCreateImage` and `AVCaptureScreenInput`.
- MDN BCD VideoDecoder versions. The openh264 #43 Cisco quote.
- Irori facts: `RelayMode::Custom`, tauri 2.12.5, windows 0.62.2, MIT licence, no macOS minimum in `tauri.conf.json`.

**Changed or added:**
1. **`screencapturekit` is not pure Rust.** It compiles a Swift bridge with SwiftPM via `xcrun`, so it needs Xcode CLT. I added this to the table, the recommended stack and the risks, given the "Rust only" constraint.
2. **Picker evidence.** I added the crate README's "always requires user permission / purely TCC-gated" statement and changed the summary to say the evidence leans against the picker avoiding TCC.
3. **iroh-live depends on `moq-video` 0.1.2 and moq-net/mux/tokio itself.** Added.
4. **Irori's lockfile has no `aws-lc-rs`, only `ring`.** This turns part of the "crypto friction" inference into a verified fact: iroh-live would add aws-lc-rs.
5. **iroh-live licence.** The README says dual Apache-2.0/MIT, while GitHub's API detects only Apache-2.0. Kept as dual, with a note.
6. **iroh-live Android demo is Camera2.** It is not screen capture, so I tempered its relevance to phone-as-sharer.
7. **`moq-video` supports H.265 too, and `set_bitrate` takes a `moq_net` type.** Added.
8. **openh264 "binary must be downloaded separately".** Upgraded from [I] to "V: issue comment", which is a commenter's reading, not Cisco's licence text.
9. **Apple DTS ad-hoc quote.** Could not re-fetch (Apple's bot-verification page), so it is marked as not re-verified.
10. **`moq-video` macOS permission lines.** Corrected to 127-131 (Preflight, then Request).
11. Added the screencapturekit macOS 15.0+ requirement for recording/mic, and the swift-bridge target of macOS 12.3.

**Not re-checked (left at their original markers):** BytePlus and Multi.app bitrates, the Nonstrict and TidBits articles, the Android MediaProjection docs, the iroh relay rate-limit docs, the objc2 issue count, `video-toolbox`/`shiguredo` stars, the rav1e/svt/vpx/ffmpeg rows, and scap's `windows` ^0.58 pin.
