# Toru: screen share, Send Files and audio sources, and what Irori #571 can take from them

**Source:** a shallow clone of `StephenSHorton/toru` (`gh repo clone`) in `a scratch folder/toru`, at HEAD `47cd861d` (2026-11-02). Paths below are relative to that checkout. **[V]** means I read it in the code (file and line given where useful). **[I]** means I inferred it and did not run or trace it. Nothing was built or run. The report is also saved at `a scratch folder/toru-report.md`. (Fact-check note: that saved copy is the uncorrected original. This corrected version was not written to disk.)

## TL;DR

- **Toru has two screen-share paths.** The README describes only the simpler one.
  - **(a) "Share to a browser on the same Wi-Fi":**
    - ffmpeg grabs the screen with DXGI Desktop Duplication (falling back to GDI) and encodes it with a hardware H.264 encoder.
    - The video is served as **HLS** (1 s segments) over plain HTTP. With no hardware encoder it falls back to **MJPEG multipart over HTTP**.
    - There is no signalling, no discovery and no NAT handling. It just shows a LAN URL and a QR code. Delay is about 1–2 s.
  - **(b) In-call Toru-to-Toru share** (`internal/stream`). **This is the one that matters for Irori.**
    - Same ffmpeg grab and hardware encoder, but the output is **RTP to a loopback UDP socket**.
    - pion **WebRTC** fans it out, with one PeerConnection per viewer.
    - Signalling is SDP sent through **encrypted MQTT on public brokers**. The topic and key both come from a room code.
    - NAT handling is public STUN, plus a TURN relay that is built but not deployed.
- **Calls are not Go code.** Voice runs in a hidden WebView2 window using Chromium's WebRTC:
  - a mesh of audio-only RTCPeerConnections;
  - Chromium's echo cancellation (AEC), gain control (AGC) and noise suppression (NS);
  - or, in "AI" mode, **DeepFilterNet3 compiled to WASM in an AudioWorklet**.
- **There are two separate noise-suppression systems.**
  - The README's "Off / Light / Strong" is an ffmpeg filter chain on the **recording** mic (`highpass + afftdn + agate`).
  - The **call** has its own Off / Standard / AI switch (Chromium's processing, or DeepFilterNet3).
- **Separate audio sources** come from WASAPI loopback and Windows 10 2004+ **process loopback**, which can include or exclude one process tree.
  - Each source is raw PCM sent over a named pipe into ffmpeg.
  - ffmpeg runs an `aformat → volume → asplit → amix normalize=0` graph, and can also write each source as its own track.
- **Send Files** is plain **magic-wormhole** (the `wormhole-william` library): a short PAKE code, the public rendezvous and transit relay, and no hole punching.
- **Best ideas for Irori:**
  1. Keep each speaker's stream separate and transcribe each one, which gives speaker-attributed transcripts for free.
  2. Exclude your own process when capturing desktop audio, so call voices never leak into a screen share. **In Rust, `wasapi` 0.26 already supports this.**
  3. Toru's RTP sequence/timestamp rewriter lets the encoder restart without renegotiating.
  4. Its low-latency encoder flags.
  5. Its silence synthesis for loopback capture.
- **Don't copy:** Toru ignores keyframe requests (it relies on periodic keyframes only), and it depends on public MQTT brokers and a public STUN list. iroh already covers the latter.

---

## 1. Screen sharing

### 1a. "Share your screen to a browser on the same Wi-Fi" (the README feature)

**Files:**
- `internal/share/server.go`
- `internal/share/lan.go`
- `internal/share/viewer.html`
- `internal/capture/share.go`
- `internal/share/third_party/hls.min.js` (hls.js 1.6.13; `frontend/package.json` also lists `hls.js` 1.6.13)

| Aspect | What the code does |
|---|---|
| **Capture** [V] | Primary: ffmpeg `ddagrab` (**DXGI Desktop Duplication**, D3D11) with `output_idx`, `offset_x/y`, `video_size` and `draw_mouse`, then `hwdownload,format=bgra`. Fallback: ffmpeg `gdigrab` (GDI BitBlt). See `BuildShareArgsDDA` / `BuildShareArgsGDI` in `capture/share.go`; `shareHLSCandidates` tries DDA first, then GDI. Still screenshots elsewhere use DXGI through `kbinani/screenshot`. |
| **Encoding: HLS path** [V] | Hardware H.264 only. `SelectShareEncoder` probes `hwH264Encoders` (NVENC, QSV, AMF), allowing 4 s per probe. Per a code comment (`capture/share.go:21-22`), **libx264 is never selected** because of the AVC-patent rule. 30 fps, downscaled to at most 1920 px wide. `shareBitrate` sets `w*h*30/20` bps (about 0.05 bits per pixel), clamped to **2.5–12 Mbps**, with `-maxrate` and `-bufsize` equal to the bitrate (near-CBR). `-bf 0`, `-g 30` and `-force_key_frames expr:gte(t,n_forced*1)` give **one keyframe per second**. Presets: nvenc `p1`, qsv `veryfast`, amf `quality speed`. Optional audio is AAC 160k through `injectAudioMixCodec`; I didn't read that function, but it is presumably the recording mix graph [I]. |
| **Muxing** [V] | `hlsMux` uses `-f hls -hls_time 1 -hls_list_size 4 -hls_flags delete_segments+append_list+omit_endlist+independent_segments`. MPEG-TS segments are written to a temp directory. |
| **Encoding: fallback** [V] | MJPEG at **15 fps**, at most **1280 px** wide, `-q:v 8`, written to stdout with `image2pipe`. `readMJPEG` splits frames on JPEG start/end markers (FFD8/FFD9) and drops any frame over 8 MB. The latest frame is held in an `atomic.Value` with a generation counter (`ShareSession.Frame`). |
| **Transport** [V] | Go `net/http` on `0.0.0.0`, port 47321–47340 or an ephemeral port (`listenShare`). Routes: `/` (viewer HTML), `/hls.min.js`, `/api/info` (returns `{kind, playlist}`), `/live/*` (the HLS directory via `http.FileServer`, `no-store`), and `/live.mjpg` (`multipart/x-mixed-replace; boundary=frame`; `serveMJPEG` checks every 40 ms and sends only when the generation has changed). |
| **Discovery / NAT** [V] | **None.** `lanIPv4s` lists the machine's IPv4 addresses ranked 192.168 > 10/8 > 172.16/12, so a Wi-Fi address wins over a Hyper-V or WSL adapter. The card shows `http://ip:port/`, a QR code (`skip2/go-qrcode`) and a hint to allow Toru through Windows Firewall. There is no authentication and no TLS: anyone on the LAN with the URL can watch. |
| **Browser side** [V] | `viewer.html` fetches `/api/info`. MJPEG plays in `<img src=/live.mjpg>`. HLS waits for the playlist, plays natively in Safari, and otherwise uses hls.js with `liveSyncDurationCount:1, liveMaxLatencyDurationCount:4, maxLiveSyncPlaybackRate:1.5, lowLatencyMode:false`. The UI tells users to expect "about a second or two of delay" (`cardInfo`). |
| **Robustness** [V] | `StartShare` tries ffmpeg in order: DDA, GDI, both again without audio, then MJPEG. A process that survives a 1.2 s grace period counts as started (`spawnShare`). Stop writes `q` to ffmpeg's stdin, kills it after 5 s, and deletes the HLS directory. |

### 1b. In-call screen share / "go live" (not in the README feature list, and the one that matters)

**Files:** `internal/stream/broadcast.go`, `args.go`, `feed.go`, `room.go`, `wire.go`, `ice.go`, `settings.go` and `call.go`. The viewer is `frontend/src/routes/Watch.tsx`.

| Aspect | What the code does |
|---|---|
| **Capture** [V] | ffmpeg `ddagrab=output_idx=…:framerate=…:draw_mouse=…` (`args.go:videoArgs`). At native resolution, frames stay on the GPU as D3D11 textures from ddagrab to NVENC/AMF ("zeroCopy"). Any scaling forces `hwdownload` and CPU scaling. Comments in `capture/replay.go` report zero-copy at "~0.3% total CPU at 3440x1440@60" and the scaled path at about one CPU core. **Windows Graphics Capture** (ffmpeg `gfxcapture`) is only a *backup* source, and only in the replay path (`capture/replay_retry.go`, `ReplaySourceWGC`). |
| **Encoding** [V] | Hardware **H.264** (default; `codecH264`, payload type 102, `profile-level-id=64001f`) or **AV1** (`codecAV1`, payload type 45). `encoderArgs` tunes each encoder for latency (`args.go:55-82`, re-checked). **NVENC:** `-preset p4` (p2 above 120 fps), `-tune ull -rc cbr -multipass disabled -zerolatency 1 -delay 0 -bf 0 -forced-idr 1`. **AMF:** `-usage ultralowlatency -quality speed -rc cbr -bf 0`. **QSV:** `-preset veryfast -async_depth 1 -bf 0`. All use CBR with a **¼-second rate buffer** (`bufsize = kbps/4`) and a keyframe interval of `fps*KeyframeSec` (**default 2 s**, clamped to 1–10). Defaults are 60 fps and Auto bitrate = `w*h*fps*bpp`, where bits per pixel is 0.1 for H.264 and 0.06 for AV1, clamped to 1.5–150 Mbps (`settings.go:173-193`, re-checked). |
| **Desktop audio** [V] | Loopback PCM goes through a named pipe into ffmpeg `libopus` with `-application lowdelay -frame_duration 10`, 160 kbps by default, 48 kHz stereo (`args.go:audioArgs`). The SDP advertises `useinbandfec=1;stereo=1`. |
| **Packetization** [V] | ffmpeg sends RTP with `-pkt_size 1200` (room for SRTP, TURN and IPv6 headers within a 1500-byte MTU) to `rtp://127.0.0.1:<port>?rtcpport=<sink>`, with `-flush_packets 1`. |
| **Fan-out** [V] | `feed.go`: a loopback UDP socket with a 32 MB read buffer, sized because 4K keyframes arrive in bursts. Packets are parsed with `pion/rtp` and written to **one shared `TrackLocalStaticRTP`**. Each viewer gets its own `webrtc.PeerConnection` that adds that same track (`broadcast.go:addViewer`). In effect: one encoder, many peer connections, no media server. |
| **Encoder restarts** [V] | The `rewriter` in `feed.go` maps each new ffmpeg run's random sequence numbers and timestamps onto one continuous timeline. Sequence numbers continue at +1, and timestamps advance by the wall-clock time that passed. So settings changes and lost captures don't force renegotiation. One caveat from the code: WebView2's decoder still chokes on new SPS/PPS headers mid-stream, so `reconnectViewers()` reconnects viewers after a video restart anyway. |
| **Loss recovery** [V] | The SDP advertises `nack`, `nack pli` and `ccm fir`. pion's NACK responder interceptor keeps a 16,384-packet history for retransmits (`newAPI`). `drainRTCP` (`broadcast.go:791`) reads incoming RTCP only so that pion's interceptors (NACK retransmits, receiver reports) run, and the application drops every packet after that. A grep of `internal/` finds no `PictureLossIndication` or `FullIntraRequest` handling, so viewers' keyframe requests (PLI/FIR) are effectively ignored; they couldn't reach ffmpeg anyway. After heavy loss, recovery waits for the next 2 s periodic keyframe, plus NACK retransmits. There is **no congestion control or bitrate adaptation**: the bitrate is fixed. |
| **Signalling** [V] | In `room.go:95-104`, a 16-symbol room code (about 79 bits, per a code comment) is run through scrypt (N=2^15, r=8, p=1, 48 bytes of output). The first 32 bytes are the secretbox key, and the last 16 become the MQTT topic `toru/v1/<hex>`. Every message is published to **two public MQTT brokers** (`broker.emqx.io` and `broker.hivemq.com`), each over TLS on 8883 with a WSS fallback (8084 and 8884). Messages are NaCl-secretbox-sealed JSON envelopes `{id, from, to, type, at, body}`, de-duplicated by id and dropped if older than 2 minutes. Message types (`wire.go`): presence, leave, watch, offer, answer, unwatch, restart, ended, notlive, call, lan. **ICE candidates are not trickled:** the streamer waits up to 4 s for gathering to finish before sending its offer (`GatheringCompletePromise` in `addViewer`), and the call window waits up to 2.5 s (`waitForIce` in `Call.tsx`). |
| **NAT** [V] | `ice.go` uses public STUN (Google, Cloudflare). TURN is used only if `stream.json` sets a `CredentialURL`. Credentials use the TURN REST scheme and are refreshed at 80% of their lifetime, with backoff on failure. `cmd/toru-relay` (pion/turn v5, HMAC credentials, refuses private-range peers, per-user caps) **is built but not deployed** (README, `docs/lan.md`). In practice, two PCs that are both behind hard NATs can't connect [I]. |
| **Viewer** [V] | `Watch.tsx` is an RTCPeerConnection in WebView2. It sets `jitterBufferTarget = 0` and `playoutDelayHint = 0` on its receivers, and samples `getStats` once a second for a stats overlay (from grep hits only). |

### Rust equivalent for Irori

| Piece | Rust option (crates.io data fetched 2026-11-06) | Notes |
|---|---|---|
| Windows capture | **`windows-capture` 2.0.1** (MIT): released 2026-08-08, 2.57 M downloads, repo pushed 2026-10-05, 21 open issues/PRs. [crates.io](https://crates.io/crates/windows-capture), [GitHub](https://github.com/NiiightmareXD/windows-capture) | It does both WGC and DXGI Desktop Duplication: the 2.0.1 source calls `DuplicateOutput`. Its encoder uses the WinRT **`Windows.Media.Transcoding.MediaTranscoder`** (`Media_Transcoding` / `Media_MediaProperties` features), which runs on Media Foundation [V, crate source; that it uses Media Foundation underneath is inferred]. Alternative: `xcap` 0.9.8 (Apache-2.0, 2.78 M downloads, released 2026-08-01) enables the `windows/Graphics_Capture` and DXGI features [V, Cargo.toml], but its own description says video recording is "WIP". |
| macOS capture | `xcap` 0.9.8, or `scap` 0.0.8 (stale: **last release 2024-12-10**; crates.io metadata last touched 2025-08-04; 60 k downloads) | Checked against the crates.io dependency lists. **`scap` depends on `screencapturekit`/`screencapturekit-sys`**, so it uses ScreenCaptureKit. On Windows it is built on `windows-capture`. **`xcap` 0.9.8 has no ScreenCaptureKit dependency**: its macOS dependencies are `objc2-core-graphics`, `objc2-av-foundation`, `objc2-core-media` and `objc2-core-video`, so it uses CoreGraphics and AVFoundation. Since scap is stale, calling ScreenCaptureKit directly through `objc2` bindings is still the safe route [I]. |
| Android capture | No crate | MediaProjection and MediaCodec through JNI [I]. |
| Hardware encode | `ffmpeg-next` 9.0.0 (WTFPL bindings; released 2026-08-05, 8.7 M downloads) gives in-process NVENC, AMF, QSV, VideoToolbox and MediaCodec with the same flags Toru uses, but it links FFmpeg (LGPL/GPL depending on the build). The native alternatives are the Media Foundation H.264 encoder (via the `windows` crate), VideoToolbox (via `objc2`) and MediaCodec (via JNI). | `openh264` 0.9.9 (BSD-2, released 2026-10-15) is **software** H.264. It's fine for a 720p/15 fps meeting share, but it's the CPU cost Toru deliberately refuses. Hardware encoding is the hardest part of "Rust only" [I]. |
| Transport | **Irori's existing iroh QUIC connection.** Voice uses datagrams (already measured). Video can use a unidirectional stream per keyframe group or per frame (reliable and ordered within a group), or datagrams with app-level retransmit requests [I]. | Only needed if a *browser* has to watch: `str0m` 0.25.0 (released 2026-11-03) or `webrtc` 0.21.1 (released 2026-10-11). `iroh-roq` 0.1.0 (RTP over iroh) is **stale and incompatible**: released 2025-02-26, last commit and push 2025-06-18, 1.4 k downloads, and it **depends on `iroh ^0.33`**, which won't resolve alongside Irori's iroh 1.4.1. Worth a look: `hang` 0.21.22 (Media over QUIC, released 2026-11-05) and `web-transport-iroh` 0.8.4 (released 2026-10-20). I didn't check which iroh version they need. |
| Browser-on-LAN share (1a) | `axum` 0.8.9 serving MJPEG multipart, or fMP4/HLS | Optional for Irori; the pattern is easy to port. |

---

## 2. Separate audio sources and noise suppression

### 2a. Capture: how the sources are kept apart [V]

- **System mix** (`capture/audio_windows.go:startLoopbackAudio`)
  - Raw COM vtable calls from Go, no cgo: `IMMDeviceEnumerator::GetDefaultAudioEndpoint(eRender)`, then `IAudioClient::Initialize(SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 1 s buffer, mix format)`, then `IAudioCaptureClient`.
  - The format comes from `GetMixFormat`, usually float32 stereo at 48 kHz.
  - **WASAPI loopback delivers no packets while nothing is playing.** The capture loop counts frames against the wall clock and **inserts silence**, so the audio track doesn't drift against the video.
- **One app only, or everything except one app** (`capture/audio_process_windows.go`)
  - Calls `ActivateAudioInterfaceAsync` on `VAD\Process_Loopback` with `AUDIOCLIENT_ACTIVATION_PARAMS { PROCESS_LOOPBACK, pid, INCLUDE_TARGET_PROCESS_TREE | EXCLUDE_TARGET_PROCESS_TREE }`. Needs Windows 10 2004+.
  - The completion handler is a COM object built by hand from Go callbacks; it answers `IAgileObject` so the callback can arrive on any thread.
  - The format is fixed at 48 kHz s16 stereo, because `GetMixFormat` isn't supported on this client. Capture runs in event-callback mode.
- **Splitting Discord out** (`capture/discord_windows.go:DiscordPID`, `capture/replay.go:openReplayAudio`)
  - "Game & desktop" is everything except Discord's process tree; "Discord" is only Discord's tree.
  - A source that isn't available becomes a silence placeholder (`anullsrc`), **never** a substitute whole-system mix. So a user who opted out of Discord audio can't end up recording their call.
- **Toru's own process is left out of streams**
  - `capture/stream_hooks.go:StartDesktopAudio(os.Getpid())` captures everything except Toru's own process tree, which holds the WebView2 call audio. Call voices therefore never re-enter the screen-share audio.
  - If that exclusion can't be set up, the stream goes without desktop audio and shows an `AudioWarning`.
  - Mixer apps such as SteelSeries Sonar re-play the microphone from their own process. For those, `audio_appmix_windows.go` builds a **per-app mix**:
    - one capture client per process that owns an audio session, skipping Toru and the mixers;
    - rescanned every 2 s, with a 60 ms lead and a 300 ms cap;
    - summed with int16 clipping.
  - `audio_mixers_windows.go` documents the measurements behind this.
- **Microphone** is captured by ffmpeg `dshow` (`-f dshow -i audio=<name>`), not WASAPI.
- **Into ffmpeg:** each source is a Windows **named pipe** of raw PCM (`-f <fmt> -ar -ac -i \\.\pipe\…`).

### 2b. Mixing and separate tracks [V] (`capture/replay.go:replayAudioArgs`)

```
[i:a] aformat=fltp:48000:stereo [,mic noise chain] [,volume=v] , asplit=2 [m_i][s_i]
[m_0][m_1]… amix=inputs=N:duration=longest:normalize=0 [aout]
-map [aout] (-map [s_0] -map [s_1] … when SeparateTracks)
```

- `normalize=0` keeps each source at its own level. Per-source volume runs from 0 to 200%.
- Track titles: "All audio", "Game & desktop", "Discord", "Microphone".
- Silence placeholders are read with `-re` so they don't run ahead of the live sources.
- Output is AAC (160k by default), in MPEG-TS segments that can be joined byte for byte.

### 2c. Noise suppression: two separate systems

| Where | Method | Code |
|---|---|---|
| **Recording / replay mic** (the README's "Off / Light / Strong") [V] | ffmpeg filters, no machine learning. **Light** = `highpass=f=80,afftdn=nr=12:nf=-50:tn=1`. **Strong** = `highpass=f=100,afftdn=nr=24:nf=-45:tn=1,agate=threshold=0.01:ratio=4:attack=5:release=300:range=0.08`. `afftdn` is an FFT spectral-subtraction denoiser that tracks the noise floor (`tn=1`); `agate` mutes the gaps between words. | `capture/replay.go:micNoiseChain` |
| **Voice call** (Off / Standard / AI) [V] | **Standard** = Chromium's WebRTC audio processing via `getUserMedia({echoCancellation:true, autoGainControl:true, noiseSuppression:true, channelCount:1})`. **AI** = Chromium's suppressor off, echo cancellation and gain control still on, then **DeepFilterNet3** as WASM in an AudioWorklet (`@lofcz/deepfilternet-web` 0.1.0, `attenuationLimit: 100`, 48 kHz mono). If the model fails to load it falls back to Chromium's suppressor. **Off** still keeps echo cancellation and gain control on; only `noiseSuppression` becomes false (`Call.tsx:305-310`, re-checked). | `frontend/src/routes/Call.tsx:loadDenoiser` and `ensureMic`; `frontend/src/stream/workletShim.js` (adds TextDecoder to the worklet scope); `internal/stream/settings.go` (`NoiseOff/Standard/AI`) |

### 2d. The voice call itself [V] (`frontend/src/routes/Call.tsx`, `internal/stream/call.go`)

- **Topology:** a hidden WebView2 window runs a **full mesh**, one audio-only RTCPeerConnection per other member (`bundlePolicy: max-bundle`).
- **Connecting:** the peer with the lower id makes the offer. A `hello` message asks the other side to re-offer. There is a 12 s connect timeout, and a failed connection is retried.
- **Signalling path:** SDP goes through the Go side (`CallSend` → room MQTT → `call:signal` event).
- **Audio parameters:**
  - Opus mono, capped at **96 kbps** (`VOICE_BITRATE = 96_000`, applied with `params.encodings[0].maxBitrate`; re-checked).
  - `receiver.jitterBufferTarget = 0` (`Call.tsx:354`).
- **Speaking indicator:** RMS above 0.015 with a 350 ms hold (`Call.tsx:41-42`). It is measured in an AudioWorklet, so throttled timers in the hidden window don't affect it.
- **Per-peer volume:** 0–200%. Above 100% it goes through a WebAudio GainNode, because `<audio>.volume` stops at 1.0.
- **Device changes** use `replaceTrack`, so they don't need renegotiation.

### Rust equivalent for Irori

| Need | Option | Maintenance (fetched 2026-11-06) |
|---|---|---|
| Mic and speaker I/O | `cpal` 0.18.2 (already in Irori) | Apache-2.0, 27.7 M downloads, released 2026-08-16 |
| Windows loopback (system mix) | **`wasapi` 0.26.0** (MIT; wraps `AUDCLNT_STREAMFLAGS_LOOPBACK`) | Released 2026-10-11, 499 k downloads, repo pushed 2026-10-31, 5 open. [crates.io](https://crates.io/crates/wasapi) |
| Windows process loopback (to exclude Irori's own pid) | **`wasapi` 0.26.0 already exposes it**: `AudioClient::new_application_loopback_client(process_id, include_tree)` (`src/api.rs:809`). It uses `ActivateAudioInterfaceAsync` with `VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK` and `PROCESS_LOOPBACK_MODE_{INCLUDE,EXCLUDE}_TARGET_PROCESS_TREE` [V, crate source] | No hand port of `audio_process_windows.go` is needed. Pass `include_tree = false` with Irori's own pid to exclude it. |
| Echo cancellation, noise suppression and gain control (**needed on the phone and on laptop speakers**) | **`sonora` 0.2.0** (BSD-3-Clause): a pure-Rust port of WebRTC's audio processing (M145) with AEC3, NS, AGC2 and a high-pass filter (per its README; not checked against the code). Its crates.io owner is `dignifiedquire` (Friedel Ziegelmayer), the **top contributor to `n0-computer/iroh`** on GitHub [V]. Whether he is employed by n0 is unverified. Alternative: **`webrtc-audio-processing` 2.1.0** (tonarino; C++ bindings, so it needs a C++ toolchain for Android; crates.io licence field is "non-standard"). | sonora: released 2026-07-29, 1.15 M downloads, 99 stars, 2 open, pushed 2026-11-06 ([GitHub](https://github.com/dignifiedquire/sonora)). webrtc-audio-processing: released 2026-05-13, 187 k downloads, 8 open (unverified; I didn't re-check the open count). |
| Noise suppression only (lightweight) | `nnnoiseless` 0.5.2 (BSD-3): pure-Rust RNNoise, 48 kHz, 480-sample frames | Released 2025-12-18, 1.1 M downloads, 4 open (unverified). It has **no echo cancellation**. |
| ML denoising (Toru's "AI" mode) | DeepFilterNet's `libDF`, published as the crate `deep_filter` | **Caution:** the latest on crates.io is **0.2.5, from 2022-07-28** [V]. The repo's `libDF/Cargo.toml` says **0.5.7-pre** [V] and the repo was pushed 2026-11-06 ([GitHub](https://github.com/Rikorose/DeepFilterNet), 66 open). Use a git dependency pinned to a commit. It runs on `tract` [I, not re-checked], and its CPU cost on the Pixel needs measuring [I]. |
| Mixing and separate tracks | Plain Rust: a ring buffer per peer, summed with gain and clipping | No crate needed. For Opus: `opus` 0.4.0 (released 2026-08-23, 3.76 M downloads) or `opus2` 0.4.0 (released 2026-03-26). `audiopus` is stale: its last release, 0.2.0, came out 2019-10-11, and its metadata was last updated 2021. |

---

## 3. Send Files (peer-to-peer file transfer)

**Files:** `internal/transfer/send.go`, `recv.go`, `manager.go`, `extract.go`, `names.go`; `frontend/src/transfer/SendFiles.tsx`; docs in `docs/send-files.md`.

- **Protocol [V]:** standard **magic-wormhole**, via `github.com/psanford/wormhole-william` v1.0.8 (`go.mod:18`, re-checked).
  - `manager.go:87` builds `wormhole.Client{PassPhraseComponentLength: 2, RendezvousURL: $TORU_WORMHOLE_RENDEZVOUS, TransitRelayAddress: $TORU_WORMHOLE_TRANSIT}`.
  - When those variables are empty, the library defaults apply: `ws://relay.magic-wormhole.io:4000/v1` and `transit.magic-wormhole.io:4001` (`docs/send-files.md:118-119`).
  - It keeps the default app ID, so it works with the Python `wormhole` command line (doc, line 62).
- **Flow [V]:**
  - The sender calls `client.SendFile(ctx, name, f, WithProgress)`, or `SendDirectory`, which zips the folder into `%TEMP%` first.
  - That returns a code like `7-guitarist-revenge`: a nameplate number plus two words.
  - The receiver calls `client.Receive(ctx, code)` (`recv.go:59`) with a 1-minute connect timeout.
  - A PAKE (SPAKE2, per the magic-wormhole spec [I]) derives a session key from the code.
  - The mailbox server only sees ciphertext. A wrong guess uses up the code, so an attacker gets one guess [I, protocol property].
  - The data goes over the **transit** protocol: a direct TCP connection if the address hints are reachable, otherwise the transit relay. Toru's doc says plainly: **"No hole punching"** (`docs/send-files.md:93`).
- **Receiver safety [V]:**
  - File names are reduced to a base name with Windows-illegal characters stripped.
  - Existing files are never overwritten; a ` (2)` suffix is added.
  - Folders are unzipped into `<dir>\<name>\`.
  - The code is also shown as a QR code.
- **Limits [V, doc]:** no resume, one send and one receive at a time, and speed across NATs is limited by the public relay.

### Rust equivalent for Irori

- **Use what Irori already has: `iroh-blobs` 0.104.1** (MIT OR Apache-2.0; released 2026-11-04, 937 k downloads, repo pushed 2026-11-06, 42 open).
  - BLAKE3-verified, resumable transfers that can fetch byte ranges.
  - Runs over the same iroh connection, with iroh's hole punching and relay.
  - **Compatible with Irori's iroh 1.4.1:** iroh-blobs 0.104.1 depends on `iroh ^1.0.0` and `iroh-base ^1.0.0` [V, crates.io dependency API].
- `magic-wormhole` 0.8.1 (released 2026-05-07, 107 k downloads, 61 open issues/PRs) is the Rust port. Only choose it if working with the wormhole CLI, or sending to someone without Irori, matters. **Licence: EUPL-1.2** (copyleft), which matters before linking it into Irori.
- **Idea worth keeping:** the short spoken **PAKE code** as a pairing step. Irori could use a wormhole-style code to swap an iroh `NodeAddr` ticket once, then use iroh for everything after. For the SPAKE2 step, use the `spake2` crate (RustCrypto/PAKEs). Its latest stable is 0.4.0 (2023-07-28); 0.5.0-pre.0 came out 2026-01-25.

---

## 4. What Irori #571 should take from Toru

### Directly applicable

1. **Keep speakers separate end to end.** Toru keeps each source on its own track (`asplit` plus one `-map` per source).
   - For Irori: decode each remote peer's Opus stream into its own buffer.
   - Run **Whisper on each peer's stream**, and on the local mic before it's mixed.
   - That gives speaker-attributed transcripts without a diarization model, stored per call for agents.
   - Mix only for playback.
2. **Leave your own process out of desktop-audio capture** when sharing a screen with sound.
   - Use Windows process loopback that excludes your own pid's process tree: `wasapi::AudioClient::new_application_loopback_client(own_pid, false)`.
   - Never fall back to the whole mix, or viewers will hear the call's voices twice.
   - Also watch for mixer apps (like Sonar) that re-play the mic, as `newAudioMixer` does.
3. **Insert silence on loopback capture.** WASAPI sends nothing during silence, so pace the stream by the wall clock or it drifts.
4. **Low-latency encoder recipe:**
   - No B-frames (`-bf 0`), CBR with a ¼-second rate buffer, no lookahead.
   - NVENC: `-tune ull -zerolatency 1 -delay 0`. AMF: `-usage ultralowlatency`. QSV: `-async_depth 1`.
   - A keyframe every 1–2 s.
   - Zero-copy from Desktop Duplication to NVENC at native resolution.
   - Do the same through `ffmpeg-next` or Media Foundation.
5. **Restart the encoder without renegotiating.** `feed.go` splices sequence numbers and timestamps across encoder runs. Since Irori controls the framing over iroh, a cleaner version is a per-share frame counter plus "new decoder config" frames that carry SPS/PPS [I].
6. **Voice parameters:**
   - Opus at 48 kHz mono, capped at **96 kbps**. Toru's comment: noise is a suppression problem, not a bitrate one.
   - Desktop audio: Opus `lowdelay`, **10 ms frames**, with in-band FEC.
   - Receiver jitter buffer: Toru sets it to zero (`jitterBufferTarget = 0`). The Mac↔PC link measured p95 of 269 ms (spikes to 900 ms), and phone↔PC p95 680 ms. Irori needs an **adaptive** jitter buffer plus Opus FEC and loss concealment, not zero [I].
7. **Processing order:** Toru keeps echo cancellation and gain control on and only swaps the noise-suppression stage for DeepFilterNet. In Rust: `sonora` (AEC3 + AGC2), then optionally `nnnoiseless` or DeepFilterNet, then Opus.
8. **Speaking indicator:** RMS above 0.015 with a 350 ms hold, computed on the audio thread.
9. **Per-peer controls:** volume 0–200%, plus mute and deafen (deafen also mutes), announced in presence (unverified: I didn't re-check the mute/deafen detail).
10. **Fallback ladders:** try DDA, then GDI, then without audio, then MJPEG. Treat a process that survives about 1.2 s as started. Show in the UI *why* a fallback happened.

### Don't copy (iroh already does these better)

- **Public MQTT brokers, public STUN and an undeployed TURN relay** for signalling and NAT.
  - Irori already dials by NodeId over iroh, starting on the relay and moving to direct (measured in under 1 s), and has gossip for presence. That covers everything Toru's stack does.
  - Toru's own LAN design ("relay first, then upgrade to direct, make-before-break", in `docs/lan.md`) rebuilds what iroh does natively.
- **Ignoring keyframe requests** (no PLI/FIR handling after `drainRTCP`).
  - On a lossy path like the double-NAT PC, waiting up to 2 s for the next periodic keyframe is visible.
  - Irori should send a "need keyframe" control message on a reliable iroh stream and have the encoder force a keyframe.
- **No congestion control.**
  - Toru's fixed CBR at 0.1 bits per pixel × 60 fps (about 12.4 Mbps at 1080p60) is far too much for a call share over a relay.
  - Irori should start around 1.5–3 Mbps for 1080p at 15–30 fps and adapt using iroh's connection stats (RTT, loss) [I].
- **Full mesh with one encode per receiver for voice.** Fine at Irori's scale of three devices. Toru's video share already does "encode once, send the same packets to everyone", which is the right model.
- **Waiting for ICE gathering instead of trickling candidates.** Not relevant over iroh.
- **`iroh-roq` as a shortcut:** it pins `iroh ^0.33` and hasn't been touched since 2025-06, so it can't be dropped into Irori's iroh 1.4.1 stack.

### Optional

- The **browser-on-LAN viewer** is a cheap way to show a share on a TV or a device without Irori: HLS with 1 s segments, or MJPEG multipart, plus a QR code. In Rust that's `axum` plus fMP4 or MJPEG. Expect 1–2 s of delay.

---

## Verification log

**Read in the code [V]:**
- `internal/share/{server.go, lan.go, viewer.html}`
- `internal/capture/`:
  - `share.go`
  - `replay.go` (lines 1–330 and 560–800)
  - `audio_windows.go` (1–200)
  - `audio_process_windows.go` (1–120)
  - `audio_appmix_windows.go`, `audio_appmix.go`, `stream_hooks.go`
  - `audio_mixers_windows.go` (header comment)
- `internal/stream/`:
  - `call.go`, `room.go`, `wire.go`, `ice.go`, `args.go`, `feed.go`
  - `broadcast.go` (1–140, 430–560, 671–800)
  - `settings.go` (parts)
  - `lan.go` (header)
- `internal/transfer/`: `send.go` (1–120), `manager.go:87`, `recv.go:59`
- `frontend/src/routes/Call.tsx` (1–60, 92–106, 255–504)
- `frontend/package.json` (deepfilternet-web 0.1.0, hls.js 1.6.13)
- `go.mod`, `docs/send-files.md`, `docs/lan.md` (parts)

**Not read:**
- `Watch.tsx` beyond grep hits.
- The rest of `broadcast.go`, `service.go` and `lan_service.go`.
- The internals of the Windows Graphics Capture fallback in the replay path.
- `wormhole-william`'s internals. The "no hole punching" claim comes from Toru's own doc.

**Sources for the numbers:**
- Crate data comes from the crates.io API (`https://crates.io/api/v1/crates/<name>`, plus `/<ver>/dependencies` and `/owner_user`). "Released" is the max stable version's `created_at`, not the crate's `updated_at`.
- GitHub stats come from `gh api repos/<repo>`. Its `open_issues_count` includes pull requests.
- The `deep_filter` version mismatch is confirmed from `https://raw.githubusercontent.com/Rikorose/DeepFilterNet/main/libDF/Cargo.toml`.
- sonora's features come from its README.
- The `wasapi` and `windows-capture` claims come from their published crate sources (`static.crates.io/crates/<name>/<name>-<ver>.crate`), extracted under `a scratch folder/{wasapi,wcap}`.

**Still not verified:**
- Whether `hang` and `web-transport-iroh` resolve against iroh 1.4.1.
- sonora's feature list and real-world quality, beyond its README.
- Whether DeepFilterNet's libDF still runs on `tract` at HEAD.

---

## Fact-check notes

Re-checked against primary sources on 2026-11-06:

1. **`wasapi` process loopback (changed).** The original left open whether `wasapi` 0.26 supports this and suggested a hand port of about 150 lines. It does support it: `AudioClient::new_application_loopback_client(process_id, include_tree)` at `src/api.rs:809` of the 0.26.0 crate source. I removed the hand-port advice from §2 and §4.
2. **`iroh-roq` (strengthened).** Its last crates.io release was 0.1.0 on 2025-02-26, and the last commit was 2025-06-18. More importantly, it depends on `iroh ^0.33`, so it is incompatible with Irori's iroh 1.4.1. The original called it only "stale".
3. **`scap` release date (corrected).** The original said the last release was 2025-08-04. In fact 0.0.8 was released 2024-12-10; 2025-08-04 is only the crate's metadata `updated_at`.
4. **macOS capture backends (resolved).** From the crates.io dependency lists: `scap` uses ScreenCaptureKit (`screencapturekit` crate). `xcap` 0.9.8 does **not**; it uses CoreGraphics and AVFoundation.
5. **`windows-capture` encoder (resolved).** It uses WinRT `MediaTranscoder` (Windows.Media.Transcoding), and its DXGI path calls `DuplicateOutput`, both seen in the 2.0.1 source.
6. **`iroh-blobs` compatibility (resolved).** 0.104.1 requires `iroh ^1.0.0`, so it works with iroh 1.4.1. I removed "check version compatibility [I]".
7. **sonora author (refined).** The crates.io owner is `dignifiedquire`, who is the top contributor to `n0-computer/iroh`. That he works for n0 is still unverified.
8. **`magic-wormhole` (Rust) licence (added).** It is EUPL-1.2, which is copyleft and matters before linking it into Irori.
9. **`spake2` (filled in).** Latest stable is 0.4.0 (2023-07-28); 0.5.0-pre.0 came out 2026-01-25.
10. **`drainRTCP` wording (refined).** It does read RTCP, and pion's NACK interceptor still runs on it. What is missing is any PLI/FIR handling anywhere in `internal/`, which a grep confirmed. The conclusion that keyframe requests are ignored still stands.
11. **Confirmed unchanged, re-read in Toru's code:**
    - Rooms and signalling: scrypt N=2^15 (48-byte output split into a 32-byte key and a 16-byte topic) and the two public MQTT brokers on 8883 (WSS on 8084/8884).
    - Encoder: the NVENC/AMF/QSV flags, `bufsize = kbps/4`, the 2 s keyframe default, and Auto bitrate at 0.1/0.06 bpp clamped to 1.5–150 Mbps. libx264 is never used for patent reasons.
    - Voice call: 96 kbps voice, `jitterBufferTarget = 0`, the 0.015 RMS / 350 ms speaking indicator, and "Off" keeping AEC and AGC on.
    - Send Files: `wormhole-william` v1.0.8, the default relay URLs, and "No hole punching".
    - deep_filter: crates.io 0.2.5 (2022-07-28) against 0.5.7-pre in the repo.
    - All other crate versions, release dates and download counts quoted in the original matched crates.io. I also added licences.
12. **Marked (unverified):** open-issue counts I didn't re-check (`webrtc-audio-processing`, `nnnoiseless`) and the mute/deafen detail.
13. **The saved report file is out of date.** The copy at `a scratch folder/toru-report.md` is the uncorrected original, and this corrected version was not written to disk.
