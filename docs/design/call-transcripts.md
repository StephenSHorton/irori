# Call transcripts (card #656)

Huddles write a transcript: each device writes down only its own person's voice, live lines show as captions in the call, the stored lines sit in the conversation's log signed by the speaker's device, and when the huddle ends the channel's end post links the transcript and the channel's agents hear once. Research: `docs/research/08-voice-calls/irori-fit.md` §4–5. The call layer it builds on: [calls.md](calls.md).

## The parts

| Layer | Where | What it does |
|---|---|---|
| Tap | `crates/call/src/engine.rs` | `Engine::set_tap(Option<Tap>)`: the engine thread hands each 10 ms chunk of the cleaned mic (after echo cancelling, noise suppression and gain, and after mute zeroes it) to the tap, 48 kHz mono. The tap must never block. |
| Transcriber seam | `crates/call/src/transcribe.rs` | `Transcriber`/`TranscriberFn`/`Segment`: what core asks for, so core never links whisper.cpp. Tests inject a fake. |
| Whisper feed | `crates/voice` | `start_feed(model, input_rate, Options, sink) -> (Session, Feed)`: a session that hears pushed audio instead of the mic, finals only (`Event::Segment{text, start_ms, end_ms}`), at most 2 threads. |
| Whisper transcriber | `crates/window/src/voice.rs` | `call_transcriber(data_dir) -> TranscriberFn`: the seam backed by `voice::start_feed`, used by the app; `None` when the model isn't there or the engine is unsupported. |
| Live lines | `crates/net/src/call.rs`, `core/service/calls.rs` | `ControlMsg` frames on each call connection's `call` stream (reliable, ordered, already open on every pair): `{"t":"transcript", ...}` and `{"t":"state","transcribe":bool}`. Unknown `t` is skipped. |
| Stored lines | `call.segment` rows | One row per finished phrase, written by the speaker's own device into the conversation's log (space board log, hidden DM log, or the private log for a Personal channel). Behind `Core::space_tolerance(space).ready()` (#652). |
| Fallback | `talk.post` with a `transcript` mark | When the gate isn't ready, the speaker's device keeps its lines and writes one post with all of them when it leaves the call. An older Irori shows it as a plain post. |
| Fold | `crates/coord/src/talk.rs` | Folds `call.segment` (and fallback posts) into a per-call index: `call_posts(call_id)` gives the lines as posts by the speaker at the phrase's time, conv `call:<id>`. |
| Agents | `crates/tools/src/talk.rs` | `talk_read {conv:"call:<id>"}`, a huddles section in `talk_list`, and one wake for the channel's agents when the huddle ends. A person DM's transcript is never shown to an agent. |
| Setting | `call.transcribe` private row | "Transcribe what I say in calls", per person, synced to their devices, on when no row says otherwise (decision 52). |
| Window | `crates/window/src/call.rs`, `frontend/src/components/talk/huddle.tsx` | A notice before the mic opens, the "Transcript on" badge, who is transcribing, live captions, the transcript pane from the end post, and the switch in Settings > App. |

## Contracts

### Tap (crates/call)

```rust
/// Called on the engine thread with each 10 ms chunk (480 samples, 48 kHz mono)
/// of the cleaned mic, after mute. Must not block.
pub type Tap = Box<dyn FnMut(&[f32]) + Send>;
impl Engine { pub fn set_tap(&self, tap: Option<Tap>); }
```

The tap sits in `Core::capture` after the mute `fill(0.0)` and before call-dev's stand-in voice is added. Muted audio arrives as zeros, so the transcriber's clock stays the call's clock and muted speech is never written down. Zeros never pass the speech gate: its threshold never goes under 0.008 RMS (`whisper.rs` `Gate`), however long the mute. Empty finals are dropped by the transcriber, so core never gets a blank segment.

Order matters when stopping: `engine.set_tap(None)` first, then `transcriber.stop()`, then drop the engine. At app exit the call (and its transcriber) stops before `voice::release()` frees the model, or macOS's Metal backend asserts at exit.

### Transcriber seam (crates/call/src/transcribe.rs)

```rust
pub struct Segment { pub text: String, pub start_ms: u64, pub end_ms: u64 } // ms from the transcriber's start
pub type SegmentSink = Arc<dyn Fn(Segment) + Send + Sync>;
pub trait Transcriber: Send {
    /// The tap to give the engine.
    fn tap(&mut self) -> Tap;
    /// Finishes the phrase in progress (its Segment goes to the sink before this returns) and stops.
    fn stop(self: Box<Self>);
}
pub type TranscriberFn = Arc<dyn Fn(SegmentSink) -> Result<Box<dyn Transcriber>, String> + Send + Sync>;
```

Core gets it as `Options.call_transcriber: Option<TranscriberFn>` (next to `call_audio`). `None` means this device can't transcribe: it shows as "not transcribed".

### Live frames (crates/net/src/call.rs)

```rust
pub const CONTROL_MAX: usize = 16 * 1024;
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ControlMsg { Transcript(TranscriptLine), State(CallPeerState) }
pub struct TranscriptLine { device_id, seq: u64, start_ms: i64 /* Unix ms */, end_ms: i64, text }  // camelCase
pub struct CallPeerState { transcribe: bool }
pub async fn write_control(send: &mut SendStream, m: &ControlMsg) -> Result<(), String>;
pub async fn read_control(recv: &mut RecvStream) -> Result<Option<ControlMsg>, String>; // skips unknown "t"
```

The receiver takes the speaker from the connection, never from the frame. `RecvStream` is held for the connection's life even when its reader stops (dropping it would stop the peer's writes).

### Rows

`call.segment` (`Replicated, Accepts`; a `call.*` kind must never be `Refuses`):

```json
{"callId": "<32 hex>", "convId": "ch_… | dm:…", "seg": 0, "startMs": 1762470000000, "endMs": 1762470002400, "text": "…", "by": "<sender key>", "at": "<GoTime>"}
```

- `convId` is the real conversation, so the spaces rules (`space_may_touch`, the hidden-DM rule `convId == pair && by == actor`) hold unchanged. `startMs`/`endMs` are Unix ms.
- `seg` counts from 0 per device per call; `(callId, device, seg)` is unique.
- Written only by the speaker's device, so the device's signature is the speaker's.
- The gate is read once, when the device enters the call, and kept for the whole call: rows or the fallback, never half of each. A Personal channel (space `""`) has no board and no gate: its rows go to the private log.

Fallback `talk.post` (in a person DM it rings the other side's notification once per speaker, at leave): text is the lines as plain text (`"0:12 I'll take the PC build"`), with an optional lenient `transcript: {callId, lines: [{startMs, endMs, text}]}` that newer builds fold into `call:<id>` and don't show as a channel post.

`call.transcribe` (private log, actor `user`, `Replicated, Refuses` is not allowed under `call.*`, so the kind is `transcribe.set`): `{on: bool, by, at}`; newest wins; none means on.

The end mark (`CallMark`, op `end`) gains `#[serde(default, skip_serializing_if)] transcript: bool`: the window shows "Transcript" on the end post and agents hear about it.

## Who sees it

- **Space channel:** its members and their agents in that channel.
- **Person DM:** the two people only. Hidden spaces never bridge to agents; `talk_read`/`talk_list` refuse `call:<id>` to an agent when the call's conversation is a person DM; no wake.
- **Personal channel:** the person and their agents.
- Sharing a person DM's transcript with an agent is not in v1.

## The one wake

When the last device out writes the end post, it tells its own person's agents in the channel (`tell_call_end`, wake). Another person's devices tell their own agents when the end post arrives (`bridge_row`, once per row). Text: "The huddle in #x ended. talk_read conv=call:<id> has the transcript." The reference stays `talk:<conv>`. It is a reading notice (not a mention), sent with a wake: agents learn of it at once without being addressed.

## Consent

- On by default; "Transcribe what I say in calls" in Settings > App turns it off, and turning it off mid-call stops the transcriber at once.
- Before the mic opens (Huddle and Join), the window says: "This huddle is transcribed. What you say is written down in #channel for its members and their agents." (in a DM: "for the two of you"). With the switch off: "Your voice isn't transcribed (Settings > App)."
- During the call: a "Transcript on" badge, and each tile shows whether that person is transcribed.
- No audio is kept: only text leaves the transcriber.

## Android

The phone hears the same tap (no second mic open). Whether base.en keeps up on the Pixel's CPU is measured with `examples/bench.rs`, cross-built for aarch64-linux-android and run over adb; until then the phone transcribes like any device, and a slow phone only lags (the feed keeps at most `MAX_PHRASE` of backlog).
