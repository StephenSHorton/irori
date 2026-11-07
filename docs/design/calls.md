# Calls: Irori's own call layer (card #654)

Huddles: voice calls of 2 to 6 devices in a space channel, a person DM, or a Personal channel (the person's own devices: Mac, phone, PC; card #663). The research behind the choices is in `docs/research/08-voice-calls/` (#571). This page says what was built, how the parts meet, how it was measured, and what's still open.

## The parts

| Layer | Where | What it does |
|---|---|---|
| Audio engine | `crates/call` (lib `call`) | One thread per call owns the mic, the speaker, echo cancelling, the Opus encoder and a jitter buffer per peer. It knows nothing of iroh: a peer is a `Link` that sends a datagram. |
| Transport | `crates/net/src/call.rs` | ALPN `irori/call/1`: one QUIC connection per pair of devices in the call, on the existing endpoint. Audio rides its datagrams. |
| Ringing | `crates/mesh/src/boardmesh.rs`, `crates/mesh/src/owncalls.rs` | A board `call` frame (`CallFrame`, ops `ring`, `join`, `leave`, `end`) to the space's devices that are online and said `calls >= 1` in their hello. In a Personal channel, the same steps as a private mesh `call` frame to the person's own devices whose mesh hello said `calls >= 1`. |
| Call service | `crates/core/src/service/calls.rs` | Wires it together: routes a conversation to its space (or the private mesh, `Ride`), starts the engine, joins the call on the endpoint, rings, dials, writes the start and end posts, publishes `CoreEvent::Call`. |
| Record | `talk.post` with a `call` mark | The start and the end are ordinary posts with a `CallMark` (`coord::talk`). An older Irori reads them as plain posts. |
| Window | `crates/window/src/call.rs`, `frontend/src/lib/call.ts` | `call_*` commands, the `call:state` event, the huddle button, call bar and ringing banner. |
| Phone | `app/src/shell/call_android.rs`, `CallService.kt`, `CallPlugin.kt` | A foreground service of type microphone, so a call survives the screen turning off. See `docs/android.md`. |
| Transcript | `crates/call/src/transcribe.rs`, `crates/voice`, `call.segment` rows | Each device writes down its own person's voice from the engine's tap: live captions, stored lines, one wake for the channel's agents at the end (card #656). See [call-transcripts.md](call-transcripts.md). |

## Audio

- **Codec:** Opus 48 kHz mono in 20 ms frames, at 28 kbit/s, with in-band FEC (planned for 10 % loss), DTX and the voice signal hint (`opus` 0.4 on libopus 1.6.1). One encode per frame, and the same datagram goes to every peer.
- **Datagram:** an 8-byte header (`wire.rs`): version, flags (DTX, muted, probe), a sequence number counting sent datagrams, and the 48 kHz timestamp. The connection names the sender.
- **DTX:** a frame Opus makes in two bytes or less isn't sent. The first one is sent, flagged DTX, so the far end plays comfort noise. After that, one goes every 20 frames as a keepalive.
- **Echo, noise, gain:** sonora 0.2 (WebRTC APM in Rust): AEC3, noise suppression (moderate), AGC2 adaptive, and a high-pass filter. It runs at 48 kHz in 10 ms chunks. The mixed playout is fed to the canceller as what the speaker plays (`Dsp::render`) before it is queued for the device.
- **Jitter buffer:** our own (`jitter.rs`), not neteq: with its optional features off, neteq never tells the decoder a packet was lost, so Opus FEC and loss concealment can't run through it.
  - Frames are played on our clock at `sender time + base + target`. `base` is the smallest recent one-way delay. `target` is the 95th percentile of the jitter plus 10 ms, clamped to 20–140 ms.
  - For a lost frame it decodes the next frame's FEC copy if it's there, and conceals otherwise. In DTX it plays comfort noise.
  - When the buffer runs ahead of or behind its target, it drops or inserts a frame, in silence when it can.
- **Devices:** cpal 0.18 with the `realtime` feature (AAudio LowLatency on Android). It prefers a 48 kHz f32 config from any range the device offers. Otherwise a cubic polynomial resampler (`rubato`) converts, adding about 2 ms; an FFT one would add tens of ms.
- **Speaker queue:** 20 ms ahead of the device.

## Transport and admission

- `irori/call/1` is listed on the endpoint, and `MemberAdmit` admits it exactly like `board/1`: the person's own chain, or a device of a board held here.
- The first bi stream is labelled `call` and carries a `CallOpenMsg` each way: `{v, spaceId, epoch, callId, deviceId, proof}`.
  - `proof = HMAC(topic key of (space, epoch), "irori/call-open/1" ‖ callId ‖ EKM("EXPORTER-irori-call") ‖ role)`.
  - `callId` is 32 lowercase hex characters.
- The acceptor checks the call is joined here, that the space and epoch match a key it holds, the proof, and the space's `accept`.
  - `accept` passes a counted member device whose key matches, or one of the person's own devices that is live in the space.
  - It then requires `max_datagram_size()`.
- **Personal channels** (#663): `spaceId` is `""`, the key is the mesh key of the person's current epoch (`mesh_secret(seed, personId, epoch).key`, the one before it too while its seed is held), and `accept` is the chain's check (`ChainAdmit`): a certified, unrevoked device of the person whose key matches. The dialer finds the peer in the private mesh's address book.
- **Duplicate connections** between two devices keep one by the boards' REPLACED rule (`keep_new`).
- **Closing:**
  - `recheck`, `drop_device`, `drop_board` and `close` reach call connections.
  - A revoked device's call ends with `revoked`.

## Who dials, ringing and the roster

- **Start:** the call service writes the start post, rings the space's online devices, and keeps re-ringing every 5 s for a minute. That reaches devices that come online or get counted a little later.
- **Join:**
  - The joiner dials every device it knows to be in the call.
  - Any device in the call dials a member it hears of in a `join` frame and isn't connected to.
  - Each new connection re-announces the roster.
  - Together these fill in the full mesh, even when the joiner knew only the device that rang it.
- **Heartbeat:** every device in a call sends a `join` frame every 15 s. A device that comes online sees the huddle as live, and a live entry nobody mentions for 50 s is dropped.
- **Leave:** a device that leaves while others are connected sends `leave`. The last one out sends `end` and writes the end post, with the call's length (from the first ring) and its people.
- **Personal channels** (#663) have no space, so ringing rides the private mesh `sync` channel instead: a `call` frame `{type, epoch, callId, op, convId, devices, atMs}` (Go-compatible keys, none folding onto an older one), sent only to a device whose mesh hello said `calls: 1`. An older build ignores the type and is never rung. A frame from the private mesh must name a Personal channel, and one from a board connection can't name one.
  - A `join` from another of the person's devices doesn't stop the ring there, as it does in a space: every device in a Personal channel is the person's, and each is rung to join.
  - The ring banner names the device that rang ("You started a huddle in #us on MacBook").

## Measured (2026-11-06)

**Tests:**
- `crates/call` has 22 tests:
  - jitter buffer: FEC recovery, concealment, jitter growth, the cap, reordering, stalls;
  - an echo-cancelling check: more than 15 dB of a simulated speaker echo removed;
  - DTX on silence;
  - the probe;
  - whole engines on fake devices: two ways, a third engine, mute, removing a peer.
- Software mouth-to-ear on fake devices is 66 ms.
- `crates/core/tests/it/huddle.rs` runs three whole huddles through real Cores on loopback iroh with fake audio:
  - one person's three devices (as the Mac, the phone and a third), with ringing, joining, voice to every speaker, mute, and leaving one by one with the end post;
  - two people in a space channel;
  - one person's three devices in a Personal channel, with no space at all: both others ring, the PC still rings after the phone answers, a full mesh of three, voice to both speakers, the end post (#663).

**MacBook Pro, its own speakers and mic (`call-dev`, two processes on one machine, direct path, RTT 0.7 ms):**
- **Mouth-to-ear:** median 154 ms, max 163 ms (n = 9). This was after moving the speaker from 44.1 kHz with an FFT resampler (median about 190 ms) to 48 kHz.
- **Echo cancelling:** with the speakers playing a far-end voice, the canceller takes 11 dB out of the mic. With it off, 1 dB (the high-pass filter). Noise suppression comes on top of that in a real call.
- **Loss:** none lost, and no underruns.

**Not measured yet:** the Pixel, the PC, a real Mac↔Pixel call, and anything over the relay.

## How to measure: call-dev

`cargo run -p irori-call --release --example call-dev -- [--dial <endpointId>]... [--mute] [--probe] [--say] [--no-aec] [--no-ns] [--no-agc] [--secs N]`

- Each run is one participant. It prints its endpoint id and dials every `--dial`. It uses throwaway keys and n0's relays, and never touches an Irori data dir.
- **Mouth-to-ear:**
  - Put two devices side by side and turn the volume up. Run the far one with `--mute` and this one with `--probe`.
  - This one plays a 20 ms 1 kHz click on its own speaker, outside the echo canceller, every 2.5 s.
  - Its mic hears the click straight from its own speaker (T0), then again from the far speaker after the trip through the call (T1).
  - Both are times in one mic stream, found with a 1 kHz Goertzel filter, so T1 − T0 is mouth-to-ear with everything in between.
  - `IRORI_CALL_PROBE_DUMP=<file>` writes the raw mic and each click's tone envelope.
- **Echo:** run the far one with `--say --mute` (a stand-in voice, every other 1.5 s) and this one plain. `reduction=` is how much quieter the cleaned mic is than the raw mic while the speaker plays. Compare with `--no-aec`.

## Open

- **Real devices:** Mac↔Pixel with echo cancelling on laptop speakers, under 200 ms mouth-to-ear, and a third device joining. This is the card's done-when, and it needs the person and the phone.
- **Earpiece:** cpal plays with media usage and sets no AAudio input preset. So Android's own echo canceller is off (sonora does it), and earpiece routing may not apply. Speakerphone is the v1 route. The earpiece and the platform canceller need AAudio directly, which means API 28 (minSdk 26 today).
- **A pair with no route:** in a space, two devices that can't reach each other (no address in the board book) never connect. A three-way call then has a missing leg, and the peer stays "connecting". There is no forwarding through a third device.
- **Transcription** is card #656: see [call-transcripts.md](call-transcripts.md). The transcriber hears the engine's tap, not the mic. Dictation (the mic button) still waits while you're in a huddle, since `crates/voice` opens the mic itself.
- **A DM mark race:** a person DM's end post written just as the other side leaves the DM's space could found a new hidden space. The window is tiny; the fix is a no-found write path for marks.
- **The `tolerant` widening** (irori-fit.md option E) landed in #652. `call.segment` (#656) is written only when every device in the space reads it (`space_tolerance`).
