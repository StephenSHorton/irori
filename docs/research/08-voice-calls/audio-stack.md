# #571 Voice calls: Rust voice-pipeline survey (2026-11-06), fact-checked

**How to read this.** **[V]** means verified: I read crates.io API JSON, the GitHub API, or the source in a clone under `a scratch folder/voicesurvey/`. **[W]** means it comes from a WebFetch or WebSearch summary of a README or docs page, so treat it as second-hand. **[I]** means it is my own inference. **[FC✓]** means the fact-checker re-verified it against a primary source on 2026-11-06. **[FC✗]** means the fact-checker corrected it. Downloads are all-time / recent (90 days) from crates.io. No repo files were modified.

## TL;DR

- **Recommended:** build our own call layer on the iroh 1.4.1 connections Irori already has, sending **QUIC datagrams** as the spike did. Use these off-the-shelf parts for the audio work:
  - **cpal 0.18** for audio I/O
  - **sonora 0.2** for AEC3, noise suppression and AGC2 (pure Rust)
  - **`opus` 0.4 / opusic-sys** (libopus 1.6.1) with in-band FEC, DTX and PLC
  - **neteq 0.9**, or a small hand-written adaptive buffer, for jitter
  - **full mesh** for 2–6 people
  - **whisper-rs** run on each device's own AEC-processed mic
- **Runner-up / where to go later:** **n0's iroh-live (MoQ over iroh)**. It is the only maintained "whole stack over iroh". It already uses cpal + sonora + Opus with a NetEq-style jitter estimator, and it carries video and screen capture. Today it is an "early tech preview":
  - Windows has never been run.
  - Audio rides QUIC *streams*, so a lost packet waits for retransmission.
  - The MoQ crates churn weekly.
- **Not recommended:**
  - WebRTC stacks (webrtc-rs, str0m). They bring their own ICE/DTLS/UDP transport and would bypass iroh.
  - LiveKit. It needs an SFU server written in Go.
  - GStreamer. It is a C toolchain, which is heavy on Android and Windows.
  - callme / iroh-roq. Both are dead against iroh 1.x.

---

## 1. Whole-stack options

| Option | Crate / version | Last release | Licence | Downloads | Repo health | Platforms | Fit for Irori |
|---|---|---|---|---|---|---|---|
| webrtc-rs | `webrtc` 0.21.1 | 2026-10-11 [FC✓] | MIT/Apache-2.0 | 9.1M / 3.6M [FC✓] | 5.2k★, 15 open, pushed 2026-11-01 [V, not re-checked] | Desktop + mobile (pure Rust) [I] | Poor: its own transport |
| str0m | `str0m` 0.25.0 | 2026-11-03 [FC✓] | MIT OR Apache-2.0 | 3.0M / 1.3M [FC✓] | 648★, 38 open [V, not re-checked] | Win/mac/Linux tested; iOS/Android "compiled not tested" [W] | Poor: its own transport, no media |
| LiveKit | `livekit` 0.10.1 | 2026-11-04 [FC✓] | Apache-2.0 | 2.5M / 553k [FC✓] | 502★, ~200 open issues+PRs [V, not re-checked] | Win/mac/Linux/iOS/Android [W] | Poor: needs the server |
| callme | not on crates.io | GH v0.1.4, 2025-03-17 [V] | MIT OR Apache-2.0 | — | 102★, 4 open, last push 2025-03-17 [FC✓] | desktop + Android (egui) | Dead |
| iroh-roq | `iroh-roq` 0.1.0 | 2025-02-26 [FC✓] | MIT OR Apache-2.0 | 1.4k [FC✓] | 36★, last push 2025-06-18 [V] | — | Dead: depends on `iroh ^0.33` [FC✓ crates.io deps] |
| iroh-live | not on crates.io (git) | main 2026-09-29 (last commit dd80d44); repo pushed 2026-11-05 [FC✓] | MIT OR Apache-2.0 | — | 140★, 11 open [FC✓] | Linux primary; macOS lightly tested; Android tested; **Windows never run** [FC✓ README] | Strong conceptually; immature |
| MoQ (moq-dev) | `moq-net` 0.3.21, `hang` 0.21.22, `moq-audio` 0.1.21, `moq-tokio` 0.19.34, `web-transport-iroh` 0.8.4 | moq-net/hang/moq-audio 2026-11-05 [FC✓]; others [V, not re-checked] | MIT OR Apache-2.0 | moq-net 230k [FC✓] | moq-dev/moq 1.6k★, **48 open** [FC✗ was 50] | as moq-audio/moq-video | The engine under iroh-live |
| GStreamer | `gstreamer` 0.25.4 | 2026-09-21 [V] | MIT OR Apache-2.0 (bindings); GStreamer itself is LGPL [I] | 12.8M | gitlab freedesktop | Everywhere, with a C SDK | Poor: C toolchain, size, Android packaging |
| Own on iroh datagrams | iroh 1.4.1 (2026-10-21) [FC✓] | — | — | — | already in tree | wherever Irori runs | **Best fit** |

**webrtc-rs.**
- v0.21 is a pre-1.0 push toward a stable API. It is built on a sans-IO `rtc` core with a thin async `webrtc` layer [W: https://github.com/webrtc-rs/webrtc].
- It ships no capture and no echo cancellation; the README covers neither [W].
- It does its own ICE, DTLS, SRTP and SCTP over UDP sockets. Using it means running a second NAT-traversal stack next to iroh, which defeats the point of having iroh [I].

**str0m.**
- Sans-IO WebRTC: ICE, SDP, TWCC, NACK, simulcast, DTLS [W: https://github.com/algesten/str0m].
- It explicitly excludes an adaptive jitter buffer, capture, encode/decode and render, and it has no TURN.
- It is "heavily tested" as an SFU; peer-to-peer use has had less testing [W].
- Same transport objection as webrtc-rs.

**LiveKit Rust SDK.**
- It connects to "LiveKit Cloud or a self-hosted server" [W: https://github.com/livekit/rust-sdks]. That server is the Go SFU, so this breaks the Rust-only, peer-to-peer goal.
- It links Google's libwebrtc through `webrtc-sys`. I infer it then gets libwebrtc's APM/AEC [I], but the README fetch did not mention AEC.
- Big binaries, plus the clang ≥21 and `-ObjC` link caveats [W].

**callme / iroh-roq.**
- callme was n0's first attempt: cpal + Opus + RTP-over-QUIC (iroh-roq) + tonarino webrtc-audio-processing [V `callme/Cargo.toml`, README].
- It patched cpal for Android device names and patched webrtc-audio-processing to build on Android (that fix was merged upstream as tonarino PR #47 on 2025-03-12) [V].
- iroh-roq requires `iroh ^0.33` [FC✓], so it is incompatible with iroh 1.4.1. Both repos are idle; n0 moved to iroh-live. Read them only as reference code.

**iroh-live (n0-computer)** [V source read: https://github.com/n0-computer/iroh-live]
- README: "**Early tech preview.** … Windows builds in CI but has never been run, on-device testing has been limited, rooms are being redesigned, and the relay has no authentication. Expect frequent API changes." [FC✓]
- Its dependencies match ours closely: `iroh = "1.2.0"` (compatible with 1.4.1) and `iroh-gossip = "0.101.0"`, the same as Irori [FC✓ `Cargo.toml`].
- It includes `iroh_live::Call` (each peer publishes `calls/<endpoint id>` and subscribes to the other), Android two-way audio and video, and macOS ScreenCaptureKit through moq-video [V docs].
- Features can be gated: `default = ["capture","render","sound-server"]`, with separate `playback` and `aec` features [FC✓ `iroh-live/Cargo.toml`]. So an audio-only consumer can turn off `render` and use `capture,playback,aec`. Whether moq-video is still compiled in an audio-only build is unverified; `capture` enables both `moq-video/capture` and `moq-audio/capture` [V `moq-media/Cargo.toml`].
- **Patch block: the README and the manifest disagree** [FC✗, nuance added].
  - The README still says a `[patch.crates-io]` to `Frando/moq@iroh-live` is needed.
  - The current main `Cargo.toml` has no patch block and pins crates.io moq versions (`moq-audio 0.1.2`, `moq-video 0.1.2`, `web-transport-iroh 0.7.0`, …) [FC✓].
  - However, the commit 8b3c6c7 (2026-09-29) message says a patch to `Frando/moq@iroh-live-5` was *added*. It was needed because "moq-video 0.0.26 does not compile for Windows with `capture`".
  - The manifest is authoritative for what main builds against today. Whether the Windows+`capture` build works without that patch is **(unverified)**.
- Audio transport (important):
  - moq-audio puts **each 20 ms Opus packet group on its own QUIC stream** by default. `rs/moq-audio/src/encode/producer.rs` says: "zero puts every packet in its own group … stalls the rest of its group until retransmitted" [FC✓].
  - moq-net *does* support datagram tracks (`insert_datagram`) [FC✓ `moq-net/CHANGELOG.md`, `tests/integration/datagram.rs` exists].
  - moq-audio and hang do not use them. In moq-mux, "datagram" appears only in a doc comment (`clock.rs`) and a test helper, not as a transport [FC✗ was "no datagram in moq-mux"; the substance stands].
  - So a loss costs about one RTT of retransmission before that frame can play. On the Mac↔PC path (p95 269 ms, spikes to 900 ms) that pushes audio late. The jitter buffer then either grows or drops the late frame [I].

**Own stack on iroh datagrams.**
- iroh's `Connection::send_datagram` is already proven by the spike (#571 commit 419641e9).
- Datagrams are unreliable and unordered but still congestion-controlled by QUIC. A lost frame is simply lost, which is what Opus FEC/PLC and a jitter buffer are designed for [I].
- Cost: we write about 4 small pieces:
  - framing (seq + timestamp + speaker id)
  - the jitter buffer (or adopt `neteq`)
  - the mixer
  - call signalling over the existing gossip/streams

## 2. Opus codec

| Crate | Version | Last release | Licence | Downloads | Kind | Notes |
|---|---|---|---|---|---|---|
| `opus` (SpaceManiac) | 0.4.0 | 2026-08-23 [FC✓] | MIT/Apache-2.0 | 3.8M / 2.3M [FC✓] | safe binding | Its only normal dependency is `opusic-sys ^0.7.3` [FC✓]. Has `set_inband_fec`, `set_packet_loss_perc`, `set_dtx`, and Decoder `decode(input, output, fec: bool)` / `decode_float(.., fec)` [FC✓ GitHub master `src/lib.rs`; 0.4.0 tag not separately checked] |
| `opusic-sys` | 0.7.7 | 2026-11-04 [FC✓] | BSD-3-Clause | 2.5M / 1.9M [FC✓] | sys | Targets **libopus 1.6.1**. Bundled by default and built with **CMake** (only under `bundled`). Android needs `ANDROID_NDK_HOME` [FC✓ README, Cargo.toml] |
| `opusic-c` | 1.6.1 | 2026-04-27 [V] | BSD-3-Clause | 110k | safe binding over opusic-sys | Alternative high-level API |
| `opus2` | 0.4.0 | 2026-03-26 [V] | MIT/Apache | 270k | binding | 2★ repo |
| `opus-codec` | 0.2.0 | 2026-08-16 [V] | MIT/Apache | 88k | binding | — |
| `opus-head-sys` | 0.3.3 | 2026-08-30 [V] | MIT/Apache | 13k | vendored sys | 2★ |
| `audiopus` / `audiopus_sys` | 0.2.0 / 0.2.2 | **2021-04-22** [V] | ISC | 1.9M / 4.3M | binding | Stale; repo last push 2023-05 |
| `magnum-opus` | 0.3.2 | **2020-06-06** [V] | MIT/Apache | 15k | binding | Dead |
| `unsafe-libopus` | 0.2.0 | 2024-11-21 [FC✓] | BSD-3 | 138k [FC✓] | c2rust transpile of libopus **1.3.1** | No C toolchain. moq-audio uses it for encode [FC✓ moq-audio Cargo.toml]; its comments say "~20% slower" and "upstream idle since 2024-11" [V] |
| `opus-rs` (restsend) | 0.1.41 | 2026-11-05 [FC✓] | BSD-3 | 235k [FC✓] | **pure Rust** enc+dec | First published 2026-02-28 (39 versions in 8 months) [V]. Claims SILK/CELT/hybrid, FEC and PLC ported from 1.6, and "production-ready". Lists its own gaps (hybrid CELT concealment, tonality analysis) [W: https://github.com/restsend/opus-rs]. Young; 53★ |
| `mousiki` | 0.2.1 | 2026-03-26 [V] | MIT | 4k | pure Rust | Repo 404s now [V]: avoid |
| `opus-native` | — | — | — | — | — | Not on crates.io [V] |

- **Fit.** Irori already needs CMake, a C/C++ compiler and libclang for whisper.cpp [FC✓ `crates/voice/Cargo.toml` lines 25–26]. The workspace already cross-builds whisper for Android with a patched `whisper-rs-sys` [FC✓ root `Cargo.toml`]. So opusic-sys's CMake requirement **adds no new toolchain**, and the "pure-Rust Opus" argument is much weaker for us than for moq.
- **Recommendation:** `opus` 0.4 on opusic-sys (libopus 1.6.1: current, with FEC and DTX).
- **Pure-Rust fallback:** opus-rs, if Android CMake becomes painful [I].
- **Risk:** opusic-sys has one maintainer (21★, 1 open issue, pushed 2026-11-04) [FC✓]. It is mitigated because libopus itself is the stable part.

## 3. Audio I/O

| Crate | Version | Last release | Licence | Downloads | Notes |
|---|---|---|---|---|---|
| `cpal` | 0.18.2 | 2026-08-16 [FC✓] | Apache-2.0 | 27.7M / 10.2M [FC✓] | 4k★, ~100 open [V]. Already used by Irori on Win/mac/Android [FC✓ `crates/voice/Cargo.toml`] |
| `oboe` / `oboe-sys` | 0.6.1 | **2024-03-03** [FC✓] | Apache-2.0 | 12M (mostly via old cpal) [FC✓] | Repo idle since 2024-03 [V]. Effectively superseded |
| `cubeb` | 0.38.0 | 2026-08-18 [V] | ISC | 543k | Mozilla's. A C++ build; no reason to switch [I] |
| `ndk` | 0.9.0 | 2024-04-26 [V] | MIT/Apache | 68.8M | cpal's Android dependency |

**cpal details**
- On Android cpal 0.18.2 uses **AAudio through `ndk::audio`**, not Oboe. Its Android dependencies are `ndk ^0.9`, `ndk-context`, `jni ^0.22`, `libc`, `num-derive`/`num-traits`, and an optional `audio_thread_priority` [FC✓ crates.io deps].
- `AudioPerformanceMode::LowLatency` is set only behind the `realtime` feature, and `BufferSize::Fixed` sets frames-per-callback and capacity. This is verified on master **and on the v0.18.2 tag** (`src/host/aaudio/mod.rs`) [FC✓].
- cpal sets **no AAudio input preset**: grep finds no `input_preset`, `InputPreset` or `VoiceCommunication` [FC✓ master and v0.18.2]. So Android's platform AEC/NS (enabled by the VOICE_COMMUNICATION preset) is **off**, and we must do software AEC.
- On macOS, cpal uses the HAL/AUHAL output, not Voice-Processing I/O. On Windows it does not request the communications-mode AEC APO. Neither gives us OS echo cancellation [I].
- Adding an input-preset patch to cpal has precedent: callme carried a cpal fork for Android [V].

**Latency [I, not measured here]:**
- Mac CoreAudio and Windows WASAPI shared mode with cpal: roughly 10–20 ms each way.
- Android AAudio LowLatency on a Pixel: roughly 10–20 ms. Without LowLatency (that is, without cpal's `realtime` feature) it can be much higher.
- This matters less than the network. Mac↔phone measured p50 12 ms RTT, so the device buffers plus a 40–60 ms jitter buffer dominate.
- Measure mouth-to-ear on the real devices in the next spike.

## 4. Echo cancellation, noise suppression, AGC

| Crate | Version | Last release | Licence | Downloads | What | Platforms / build | Verdict |
|---|---|---|---|---|---|---|---|
| **`sonora`** | 0.2.0 | 2026-07-29 (0.1.0 2026-02-11) [FC✓] | BSD-3-Clause | 1.15M / 1.14M [FC✓] | **Pure-Rust port of WebRTC APM M145 (+15 upstream fixes to M156): AEC3, NS, AGC2, HPF**. C++ reference suite (2400+ tests) is validated with the Rust backend through FFI; about 1.07× C++ time on the 16 kHz mono bench [FC✓ README] | Build+test on Linux, macOS arm64, Windows x86_64, **Android aarch64/armv7/x86_64/i686 via cross**, iOS via simulator [FC✓ README]. Pure cargo | **Pick.** moq-audio chose it because webrtc-audio-processing "wants CMake and abseil", and its comment cites sonora#14 (fixed in 0.2) [FC✓ moq-audio Cargo.toml]. Risks: 99★, 2 open, pushed 2026-11-06 [FC✓]; young; neural residual echo estimator not ported [FC✓] |
| `webrtc-audio-processing` (tonarino) | 2.1.0 | 2026-05-13 [FC✓] | crates.io "non-standard" [FC✓]; GitHub BSD-3 [V] | 187k [FC✓] | C++ APM bindings (PulseAudio's fork) | System lib by default, or `bundled` with **meson + ninja** + submodule [W]. Android fix merged 2025-03 [V PR #47] | Proven, but adds meson/ninja to three platforms' builds |
| `aec3` (RubyBit) | 0.4.0 | 2026-09-16 [FC✓] | crates.io "MIT OR BSD-3-Clause" [FC✓]; GH NOASSERTION [V] | 57k [FC✓] | Rust AEC3-based canceller | pure Rust (rustfft) [V deps] | Less complete than sonora; 61★ |
| `aec-rs` (thewh1teagle) | 1.0.0 | 2024-12-08 [V] | MIT | 12k | speexdsp AEC binding | C build | Speex AEC is weaker than AEC3 on laptop speakers [I] |
| `speexdsp` (rust-av) | 0.1.2 | 2022-04-28 [V] | MIT | 5k | bindings | — | Stale |
| `nnnoiseless` | 0.5.2 | 2025-12-18 [FC✓] | BSD-3 | 1.1M / 363k [FC✓] | RNNoise port (NS only) | pure Rust | Good optional NS upgrade; **not AEC** |
| `deep_filter` (DeepFilterNet) | 0.2.5 | **2022-07-28** [FC✓] | MIT/Apache (crates.io) [FC✓]; GH NOASSERTION [V] | 1.26M [FC✓] | DNN noise suppression | Repo active (4.9k★, last GH release v0.5.6 2023-08) [V]; README lists Linux/mac/Windows, no Android [W] | crates.io release is stale; heavier; NS only |

- **The key point for laptops:** AEC needs the **exact far-end signal you send to the speaker**, time-aligned with the mic. That means the playback mixer must feed AEC's "render" side.
  - moq-audio does exactly this [V moq-audio lib.rs docs]. In iroh-live's current API the hook is `playback::canceller` returning a `moq_audio::aec::Control`, one per engine [FC✓ commit 8b3c6c7 message].
  - Copy that design.
- Run AEC at 48 kHz in 10 ms chunks (sonora/APM's native frame) [I].

## 5. Jitter buffer, PLC and FEC

- **Opus in-band FEC (LBRR).**
  - Set it with `set_inband_fec(true)` + `set_packet_loss_perc(n)` [FC✓ source].
  - The receiver recovers lost packet N by decoding packet N+1 with `fec=true` (the `decode(.., fec: bool)` API exists [FC✓]). That costs one frame (20 ms) of extra delay, applied only when a loss is detected.
  - It works only in SILK/hybrid modes and at bitrates where libopus chooses to spend the bits (≈ above 16–24 kbps for wideband voice) [I].
- **PLC.** Decode with an empty or missing packet. moq-audio does this: "An empty Opus packet marks one lost packet and conceals" [V `decode/decoder.rs`]. My grep found **no FEC usage in moq-audio** [FC✓ grep for inband/fec found nothing], so I infer it is not enabled there.
- **DTX.** `set_dtx(true)` saves bandwidth in silence [FC✓ API exists]. The receiver must tell DTX gaps from loss; moq-audio's `Activity` classification shows how [V].
- **Adaptive jitter buffer crates:**
  - **`neteq`** 0.9.1 (2026-08-16) [FC✓], MIT OR Apache-2.0, 111k downloads [FC✓], from security-union/videocall-rs (1.8k★).
    - Design: NetEQ-inspired, quantile delay estimator, accelerate / preemptive-expand / expand time-stretch, insert packets + `get_audio()` every 10 ms [W https://docs.rs/crate/neteq/latest].
    - Stated as "early preview – API is settling" [W].
    - Native decode via optional `ropus ^0.12`; the optional `cpal ^0.15` pin is stale [FC✓ deps]. The core (non-optional) dependencies are small: log, ringbuf, serde, thiserror, web-time [FC✓].
    - Verdict: usable, or a good reference.
  - **moq-audio's `jitter`/`ring`/`stretch` modules** are a NetEq-estimator implementation with a conformance corpus, but they are **private modules** [V lib.rs]. They cannot be reused without adopting moq-audio's decode/playback path. Their file is a good model to copy (MIT/Apache).
  - str0m explicitly has no adaptive jitter buffer [W].
- **What we must write ourselves [I]:**
  - The datagram header: ~8–12 bytes (stream id, seq, 48 kHz timestamp, flags).
  - A target-delay estimator: p95 of arrival jitter, clamped to 40–200 ms.
  - Simple time-stretch, or drop/insert-silence: start crude and add WSOLA later.
  - Playout clock and drift handling: rubato 5.0.1 (2026-10-01) [FC✓] for resampling.
  - A per-peer stream → mixer.
  - Roughly 500–1000 lines.
- **Why this matters for us:** the Mac↔PC path's p95 269 ms and 900 ms spikes came from a loaded PC. A buffer sized for p95 would add about 270 ms mouth-to-ear. Prefer a **capped** target (~120 ms), let PLC/FEC cover the tail, and keep showing per-peer health in the UI [I].

## 6. Mixing: mesh vs SFU (2–6 people)

- **Full mesh is fine at this size [I, arithmetic].**
  - Opus voice at 24–32 kbps + ~40 B QUIC/iroh overhead per 20 ms frame ≈ 40–48 kbps per stream.
  - With 6 people each device sends 5 copies (~240 kbps up) and receives 5 (~240 kbps down).
  - Decoding 5 Opus streams is trivial CPU, even on a phone.
- Encode **once** and send the same packet to every peer (no per-peer encode). Mix the N−1 decoded streams locally, then feed that mix to playback **and** to AEC render.
- An SFU or relay-forwarder only pays off past ~8–10 people or on weak uplinks. iroh's relay is only a packet relay, not a mixer.
  - The double-NAT PC, if it falls back to relay (p50 124 ms measured), only lengthens its own legs.
  - Irori already ships the `iroh-relay` server (`iroh-relay = { version = "1.4.1", features = ["server"] }`) in `crates/home` [FC✓], so that relay is our own.
- Later option: one well-connected peer forwards (MoQ's relay model) [I].

## 7. Live transcription during the call (whisper-rs)

- **whisper-rs 0.16.0** (2026-03-12) [FC✓], Unlicense, 1.9M downloads [FC✓].
  - The crates.io repository field points to Codeberg (https://codeberg.org/tazz4843/whisper-rs) [FC✓].
  - Irori already uses it on Windows, on macOS with the `metal` feature, and on Android [FC✓ `crates/voice/Cargo.toml`]. It uses a patched `whisper-rs-sys` (StephenSHorton fork, `fix/cross-compile-target-os`) for cross-compiling [FC✓ root `Cargo.toml`]. The model is base.en [V].
- **Per-speaker by construction:** each device transcribes **its own mic** and publishes timestamped segments to the call record. That gives speaker labels without diarization.
- **Transcribe the AEC-processed signal** [I]. Otherwise far-end voices leaking through laptop speakers get attributed to the local speaker.
  - That puts the transcriber after sonora, on a fan-out from the capture buffer: one mic, two consumers (the encoder and whisper).
  - moq-audio's capture producer → many consumers is the pattern [V lib.rs docs].
- **Streaming approach [I]:**
  - VAD-segment the processed mic, then run whisper on each utterance (≤ 30 s).
  - Optionally re-run a sliding 5–10 s window for live partials, as Irori's existing Partial/Final contract does.
  - VAD options:
    - `earshot` 1.2.2 (2026-08-19, pure Rust, MIT OR Apache-2.0, 463k downloads) [FC✓]
    - sonora's VAD in AGC2 [V README]
    - `voice-activity-detector` 0.2.1 (Silero, ONNX) [V]
  - Use the call's shared clock (sender timestamp) to interleave the transcripts.
- **Phone CPU cost of base.en: not verified.**
  - I found no trustworthy Pixel benchmark.
    - One blog claims Pixel 6a Vulkan 80–150 ms P50, but its results table is empty [W: https://dev.to/software_mvp-factory/wiring-whispercpp-to-androids-audiorecord-api-hjo].
    - A Voiceping benchmark puts whisper.cpp Base at RTF ≈0.59 on an unnamed 4 GB iOS device [W: https://voiceping-landingpage-prod.pages.dev/en/blog/research-offline-speech-transcription-benchmark/].
  - **Measure with whisper.cpp `bench` on the Pixel.**
  - Expectation [I]: base.en on 4 big cores is in the ~0.3–0.7 RTF range per utterance.
    - That is fine for after-utterance finals.
    - It contends with the call for CPU and thermal headroom.
    - It is tight for continuous partials.
  - Mitigations:
    - The phone does finals only.
    - Use tiny.en or a quantized q5 model on the phone.
    - Or the phone sends its processed Opus to the user's own desktop to transcribe (labels are still known from the stream id).
- **Android status (corrected) [FC✗].** The earlier draft called this a blocker: "`crates/voice/src/lib.rs` still says Android reports `Status::Unsupported`". That is stale.
  - The module doc comment (lines 22–25) does still say that.
  - But the code compiles the `whisper` engine for `any(windows, target_os = "macos", target_os = "android")`, and `other.rs` (Unsupported) only for other OSes [FC✓ `lib.rs` lines 142–150].
  - `crates/voice/Cargo.toml` has Android deps `cpal 0.18.2` and `whisper-rs 0.16.0` with "AAudio capture through cpal; whisper.cpp on the CPU (NEON)" [FC✓].
  - So the Android whisper path is wired in code. Whether it has been run on the Pixel is **(unverified)**, and on-phone performance is still the open question above.

## Recommended stack

| Layer | Pick | Why |
|---|---|---|
| Transport | iroh 1.4.1 **QUIC datagrams** on the existing connection; control/signalling on a stream or gossip | Spike proves it: 12 ms LAN, direct in < 1 s. A loss never stalls later frames |
| Capture/playback | cpal 0.18.2 (already in tree), with the `realtime` feature for AAudio LowLatency | One API on three OSes. A small fork is needed only for an AAudio input preset |
| AEC/NS/AGC | sonora 0.2 | Pure Rust, AEC3 quality, Android build+test in CI, chosen by moq for the same reasons |
| Codec | `opus` 0.4 → opusic-sys (libopus 1.6.1), 48 kHz mono, 20 ms, ~24–32 kbps, FEC on, DTX on | Current libopus. We already pay for CMake |
| Jitter/PLC | neteq 0.9 (evaluate), else hand-roll after moq-audio's `jitter.rs` design; Opus PLC + FEC | No mature drop-in exists |
| Resample | rubato 5 | Ecosystem standard (14.9M downloads) [FC✓] |
| Topology | full mesh, encode once | 2–6 people ≈ 240 kbps each way at most |
| Transcripts | whisper-rs on each device's AEC'd mic, VAD-segmented, timestamped, stored with the call | Free speaker labels, nothing leaves the device |
| Screen share (later) | evaluate moq-video / iroh-live then | That is where n0's stack is strongest |

**Risks.**
- We own the jitter buffer and sync logic.
- sonora is young (v0.2, first release Feb 2026).
- AEC alignment on Android: AAudio latency reporting varies by device.
- Any spike to 900 ms on the PC will be audible whatever the stack. The fix is the PC's load and NAT, not the codec.

## Runner-up: adopt iroh-live / moq-audio

- **For:**
  - Same iroh and gossip versions as Irori.
  - Audio (cpal + sonora AEC + Opus + NetEq-style buffer, PLC, DTX) is already integrated.
  - It has a `Call` API, Android two-way tested, screen capture on macOS, and a path to video and screen share.
  - Built by the iroh team.
- **Against:**
  - "Early tech preview"; **Windows never run**, and the PC is one of our three devices. The Windows+`capture` build needed a moq patch as recently as 2026-09-29, and current main has dropped that patch; status unverified.
  - The MoQ crates release very often with breaking changes (hang 0.21.22, moq-net 0.3.21, moq-audio 0.1.21 all published 2026-11-05).
  - iroh-live main itself pins older moq versions (`moq-audio 0.1.2`, `web-transport-iroh 0.7.0`) and has had no commit since 2026-09-29 [FC✓].
  - Audio rides QUIC streams with retransmit stalls rather than datagrams (moq-net has datagram tracks, but moq-audio doesn't use them).
  - There is no FEC.
  - Video/wgpu dependencies come along unless carefully feature-gated.
- **When to switch:** if screen sharing becomes the priority, or once iroh-live stabilises and runs on Windows. Our own datagram layer would then be the piece we drop.

## Sources

- Crate metadata: `https://crates.io/api/v1/crates/<name>` and `/<name>/<ver>/dependencies` (queried 2026-11-06). Repo stats: `gh api repos/<owner>/<repo>`.
- https://github.com/n0-computer/iroh-live (README, Cargo.toml on main, commit 8b3c6c7, docs/platforms.md, docs/guide/android.md)
- https://github.com/moq-dev/moq (rs/moq-audio Cargo.toml, src/lib.rs, src/encode/producer.rs, src/decode/decoder.rs; rs/moq-net CHANGELOG + tests/integration/datagram.rs)
- https://github.com/n0-computer/callme, https://github.com/n0-computer/iroh-roq
- https://github.com/dignifiedquire/sonora (README, Cargo.toml)
- https://github.com/webrtc-rs/webrtc, https://github.com/algesten/str0m, https://github.com/livekit/rust-sdks
- https://github.com/tonarino/webrtc-audio-processing (+ PR #47), https://github.com/RubyBit/aec3-rs, https://github.com/jneem/nnnoiseless, https://github.com/Rikorose/DeepFilterNet, https://github.com/thewh1teagle/aec
- https://github.com/SpaceManiac/opus-rs (src/lib.rs), https://docs.rs/opus/0.4.0/opus/struct.Encoder.html, https://github.com/DoumanAsh/opusic-sys (README, Cargo.toml), https://github.com/restsend/opus-rs, https://github.com/DCNick3/unsafe-libopus
- https://github.com/RustAudio/cpal (src/host/aaudio.rs on master; src/host/aaudio/mod.rs at v0.18.2), https://github.com/katyo/oboe-rs
- https://docs.rs/crate/neteq/latest, https://github.com/security-union/videocall-rs
- https://codeberg.org/tazz4843/whisper-rs, https://crates.io/crates/earshot
- https://dev.to/software_mvp-factory/wiring-whispercpp-to-androids-audiorecord-api-hjo, https://voiceping-landingpage-prod.pages.dev/en/blog/research-offline-speech-transcription-benchmark/
- Irori worktree (read-only): `crates/voice/src/lib.rs`, `crates/voice/Cargo.toml`, `crates/home/Cargo.toml`, root `Cargo.toml`

Clones used for verification are under `a scratch folder/voicesurvey/` (iroh-live, moq, callme, sonora, cpal).

---

## Fact-check notes

**Re-verified against primary sources, no change needed:**
- The crates.io version, date, licence and downloads were correct for 24 crates: sonora, opus, opusic-sys, cpal, neteq, iroh-roq, whisper-rs, iroh, webrtc, str0m, livekit, rubato, earshot, opus-rs, unsafe-libopus, webrtc-audio-processing, deep_filter, nnnoiseless, aec3, oboe, moq-audio, moq-net, hang.
- These were also confirmed:
  - `iroh-roq` depends on `iroh ^0.33`.
  - `opus` 0.4.0 depends on `opusic-sys ^0.7.3`.
  - opusic-sys targets libopus 1.6.1 and uses CMake when `bundled`.
  - neteq has the optional `cpal ^0.15` dependency.
  - cpal's Android deps are as listed, with no input preset.
  - iroh-live's README quote, `iroh 1.2.0`, `iroh-gossip 0.101.0`, and feature names.
  - moq-audio's one-group-per-packet comment.
  - moq-net's `insert_datagram`.
  - sonora's README platform matrix, M145/M156, 2400+ tests and 1.07×.
  - Irori ships `iroh-relay` 1.4.1 `server`.
  - whisper.cpp needs CMake + libclang.

**Changed:**
1. **Android whisper "blocker" was wrong.** `crates/voice` compiles the whisper engine on Android (`cfg(any(windows, macos, android))`). Only a stale doc comment says Unsupported. On-device status is marked unverified.
2. **iroh-live patch block.** Commit 8b3c6c7's own message says a `[patch.crates-io]` to `Frando/moq@iroh-live-5` was needed for the Windows `capture` build. Current main has no patch, so Windows build status is unverified. Also added that iroh-live main pins older moq crates (moq-audio 0.1.2, web-transport-iroh 0.7.0) and has had no commit since 2026-09-29.
3. **moq-mux "no datagram" grep** was slightly wrong. The word appears in a doc comment and a test helper, but not as a transport. The conclusion is unchanged.
4. **cpal LowLatency `realtime` gating** is now confirmed on the v0.18.2 tag, not just master. The recommendation now says to enable cpal's `realtime` feature.
5. **The opus decoder `decode(.., fec: bool)`** and `set_dtx` / `set_inband_fec` / `set_packet_loss_perc` are confirmed in the SpaceManiac/opus-rs master source. The 0.4.0 tag was not checked separately.
6. **Small number fixes:**
   - moq-dev/moq has 48 open issues, not 50.
   - sonora's Android CI also covers i686.
   - The sonora speed figure is narrowed to the 1.07× bench row.
   - opusic-sys repo stats were added.
   - The AEC hook name is now `playback::canceller` → `aec::Control`.

**Not re-checked:** these keep the original tag.
- GitHub star and issue counts for webrtc-rs, str0m and LiveKit.
- GStreamer, cubeb, ndk, opusic-c, opus2, opus-codec, opus-head-sys, audiopus, magnum-opus, mousiki, aec-rs, speexdsp and voice-activity-detector metadata.
- All [W] and [I] claims, including the latency estimates and the Pixel whisper RTF.
