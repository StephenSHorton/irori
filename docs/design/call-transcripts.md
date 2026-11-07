# Call transcripts (card #656)

Huddles write a transcript. Each device writes down only its own person's voice. Live lines show as captions in the call. The stored lines sit in the conversation's log, signed by the speaker's device. When the huddle ends, the end post links the transcript and the channel's agents hear once. Research: `docs/research/08-voice-calls/irori-fit.md` §4–5. The call layer it builds on: [calls.md](calls.md).

## The parts

| Layer | Where | What it does |
|---|---|---|
| Tap | `crates/call/src/engine.rs` | `Engine::set_tap(Option<Tap>)`: the engine thread hands each 10 ms chunk of the cleaned mic to the tap, 48 kHz mono. Cleaned means after echo cancelling, noise suppression and gain, and after mute zeroes it. The tap must never block. |
| Transcriber seam | `crates/call/src/transcribe.rs` | `Transcriber`, `TranscriberFn`, `Segment`: what core asks for, so core never links whisper.cpp. `fake::energy` is the tests' stand-in. |
| Whisper feed | `crates/voice` | `start_feed(model, input_rate, Options, sink) -> (Session, Feed)`: a session that hears pushed audio instead of the mic. `Options::call()` is finals only (`Event::Segment{text, start_ms, end_ms}`), at most 2 threads, 4 on Android. |
| Whisper transcriber | `crates/window/src/voice.rs` | `call_transcriber(data_dir) -> TranscriberFn`: the seam backed by `voice::start_feed(model, 48_000, Options::call(), …)`. The shell installs it on desktop and Android. Each start returns `Err` when the engine is unsupported or the model isn't downloaded, checked at every start, so a model downloaded mid-session works for the next huddle. |
| Live lines | `crates/net/src/call.rs`, `crates/core/src/service/calls.rs` | `ControlMsg` frames on each call connection's `call` stream (reliable, ordered, already open on every pair): `{"t":"transcript", ...}` and `{"t":"state","transcribe":bool}`. An unknown `t` is skipped. |
| Stored lines | `call.segment` rows | One row per finished phrase, written by the speaker's own device into the conversation's log: the space's board log, a person DM's hidden space log, or the private log for a Personal channel. Behind `Core::space_tolerance(space).ready()` (#652). |
| Fallback | `talk.post` with a `transcript` mark | Lines that weren't stored as rows are kept by the speaker's device and written as posts when it leaves the call. An older Irori shows them as plain posts. |
| Fold | `crates/coord/src/talk.rs` | Folds `call.segment` rows and fallback posts into a per-call index. `call_posts(call_id)` gives the lines as posts by the speaker at the phrase's time, conv `call:<id>`; `calls()` lists the huddles. |
| Agents | `crates/tools/src/talk.rs`, `crates/core/src/service/bridge.rs` | `talk_read {conv:"call:<id>"}`, a huddles section in `talk_list`, and one wake for the channel's agents when the huddle ends. A person DM's transcript is never shown to an agent. |
| Setting | `transcribe.set` private row, `crates/core/src/service/call_settings.rs` | "Transcribe what I say in calls", per person, synced to their devices, on when no row says otherwise (decision 52). |
| Window | `crates/window/src/{call,talk,voice}.rs`, `frontend/src/lib/call.ts`, `frontend/src/components/talk/{huddle,transcript-pane}.tsx` | The notice before the mic opens, the "Transcript on" badge, who is transcribed, live captions, the transcript pane from the end post, and the switch in Settings > App. |

## Contracts

### Tap (crates/call)

```rust
/// Called on the engine thread with each 10 ms chunk (480 samples, 48 kHz
/// mono) of the cleaned mic, after mute. Must not block, and neither must
/// dropping it (the engine drops it on its thread).
pub type Tap = Box<dyn FnMut(&[f32]) + Send>;
impl Engine { pub fn set_tap(&self, tap: Option<Tap>); } // a message to the engine thread
```

- The tap sits in the engine's capture loop after mute's `fill(0.0)`, and before call-dev's stand-in voice is added.
- Muted audio arrives as zeros. Mic audio the engine drops when it falls behind also arrives as zeros, in whole chunks. So counting the tap's samples keeps the call's time.
- Muted speech is never written down. Zeros never pass the speech gate: its threshold never goes under 0.008 RMS (`whisper.rs` `Gate`), however long the mute.
- Blank finals are dropped by the transcriber, and core drops a blank line again, so no blank segment is stored.

Order when stopping: `engine.set_tap(None)` first, then `transcriber.stop()`, then drop the engine. At app exit the shell calls `call_shutdown()` before `voice_shutdown()` (which calls it again first), so the call and its transcriber stop before `voice::release()` frees the model. Otherwise macOS's Metal backend asserts at exit.

### Transcriber seam (crates/call/src/transcribe.rs)

```rust
pub struct Segment {
    pub text: String,
    pub start_ms: u64, // ms of audio since the first tapped sample
    pub end_ms: u64,   // ms of audio since the first tapped sample
}
pub type SegmentSink = Arc<dyn Fn(Segment) + Send + Sync>;
pub trait Transcriber: Send {
    /// The tap to give the engine.
    fn tap(&mut self) -> Tap;
    /// Finishes the phrase in progress (its Segment goes to the sink before this returns) and stops.
    fn stop(self: Box<Self>);
}
pub type TranscriberFn = Arc<dyn Fn(SegmentSink) -> Result<Box<dyn Transcriber>, String> + Send + Sync>;
```

- Segment times are counted in samples, not read off a clock. Core anchors them to wall time: it reads `now_ms()` just before it sets the tap, and a line's Unix times are that anchor plus `start_ms` and `end_ms`.
- Whisper's `start_ms` includes a short lead-in of silence; `end_ms` is where the speech stopped.
- The feed never blocks the tap. A chunk that meets Whisper taking the backlog is dropped but still counted in the times, and the backlog keeps only the newest 25 s (`MAX_PHRASE`) when Whisper lags.

Core gets it as `Options.call_transcriber: Option<TranscriberFn>`, next to `call_audio`. `None` (nothing installed, as in some tests) means this device can't transcribe. A start that returns `Err` is logged and the call goes on with this device not transcribed (`scribe_failed`). The ticker doesn't retry a failed start until the switch is turned off and on again.

The window says before any call whether this device can transcribe: `CallView.can_transcribe` is true when the machine has a speech engine and the model is downloaded, the same two checks a start makes. In a call, `CallMineView.can_transcribe` also needs a transcriber installed.

The model loads before the mic opens: `call_enter` makes the transcriber (`scribe_open`) right after the keys check, before the phone's service, the engine and `call_join`, with no lock held. It goes on the tap (`scribe_attach`) once the call is joined. So `call_start` and `call_join` wait for the model to load (seconds the first time).

### Live frames (crates/net/src/call.rs)

```rust
pub const CONTROL_MAX: usize = 16 * 1024;
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ControlMsg { Transcript(TranscriptLine), State(CallPeerState) }
pub struct TranscriptLine { device_id, seq: u64, start_ms: i64 /* Unix ms */, end_ms: i64, text }  // camelCase
pub struct CallPeerState { transcribe: bool }
pub async fn write_control(send: &mut SendStream, m: &ControlMsg) -> Result<(), String>; // cuts text to fit
pub async fn read_control(recv: &mut RecvStream) -> Result<Option<ControlMsg>, String>; // skips unknown "t"
```

- Each new connection first gets `State{transcribe}` for this device. A mid-call switch sends `State` again to every peer.
- A line's `seq` is its `seg`. A line is cut to 12 KiB (`LINE_MAX`) before it goes anywhere.
- Each peer has a writer queue of 64 frames. A line is offered with `try_send`, so a full queue drops it from the live view only; the stored line is unaffected.
- The receiver takes the speaker from the connection, never from the frame. It drops a `seq` it has already seen from that device, and forgets what it has seen whenever a new connection to that device comes up.
- `RecvStream` is held for the connection's life even when its reader stops: dropping it would stop the peer's writes.
- The window keeps the newest 40 captions (`CAPTIONS`), this device's and its peers'.

### Rows

`call.segment` (`Replicated, Accepts`; a `call.*` kind must never be `Refuses`):

```json
{"callId": "<32 hex>", "convId": "ch_… | dm:…", "seg": 0, "startMs": 1762470000000, "endMs": 1762470002400, "text": "…", "by": "<sender key>", "at": "<GoTime>"}
```

- `convId` is the real conversation, so the spaces rules (`space_may_touch`, the hidden-DM rule `convId == pair && by == actor`) hold unchanged. `startMs`/`endMs` are Unix ms.
- `seg` counts from 0 per device per call. `(callId, device, seg)` is unique: the fold keeps the first row and drops a repeat. Its line id is `seg_<callId>_<device>_<seg>`.
- A device that leaves and rejoins the same call carries on its numbering: `call_enter` starts at one past its highest `seg_` line already folded for the call.
- Written only by the speaker's device, so the device's signature is the speaker's. `write_segment` refuses a `by` that isn't a person, an archived channel, an empty call id and a negative seg.

**Rows or fallback.** The gate is read once, when the device enters the call, and kept for the whole call (`Script.rows`). A Personal channel (space `""`) has no board and no gate: its rows go to the private log.
- Gate ready: each line is written as a row as it's finished. If a write is refused mid-call, the error is logged and the line goes to the fallback buffer, so nothing is lost.
- Gate not ready: every line goes to the fallback buffer.
- In fallback mode `seg` starts at 0 again on a rejoin. Fallback lines are told apart by their post, so this is harmless.

**Fallback posts.** At leave the buffer is written as `talk.post`s in the call's conversation, split so no post's text passes 64 KiB.
- The text is the lines as plain text, timed from the huddle's start: `"0:12 I'll take the PC build"`.
- The post carries a lenient `transcript: {callId, lines: [{startMs, endMs, text}]}` mark. Newer builds fold the lines into `call:<id>` (ids `<postId>#<i>`) and keep the post out of the conversation. An older Irori shows it as a post.
- In a person DM it rings the other side's notification once per post, as any DM post does.

**The switch.** `transcribe.set` (private log, `Replicated, Refuses`; the switch can't be a `call.*` kind, since those must accept): `{on: bool, by, at}`. Only rows with actor `user` count. The largest `at` wins, ties going to the larger device id, so every device settles on the same row. With none it is on. It is set from the app only; no tool reaches it.

**The end mark.** `CallMark` (op `end`) gains `#[serde(default, skip_serializing_if)] transcript: bool`. The last device out sets it when it heard any line in the call (its own or a peer's, live) or the call has any line folded here. The window shows "Transcript" on the end post, and agents hear about it.

## Leaving

`call_leave` runs in this order:

1. Sends `leave` (or `end` when it's the last one out) to the space.
2. Closes the call's connections.
3. `set_tap(None)`, then stops the transcriber. The phrase in progress finishes and is queued.
4. Waits up to 10 s (`DRAIN_FOR`) for the writer to store every queued line, then marks the transcript closed. A line that arrives after that is counted and logged ("N lines came after the call was left: not written"), never written after the end post.
5. Writes the fallback posts, if any.
6. The last one out writes the end post and tells its person's agents (below).

The connections close before the transcriber stops, so the phrase in progress at leave is stored but never reaches the peers live. Stopping the transcriber first would fix that, at the cost of a longer leave.

A switch-off mid-call stops the transcriber the same way but doesn't close the transcript, so turning it back on writes again. The call's ticker (every 400 ms) notices the switch and starts or stops the transcriber (`scribe_toggle`).

## Who sees it

- **Space channel:** its members and their agents in that channel.
- **Person DM:** the two people only. Hidden spaces never bridge to agents. `talk_read` and `talk_list` refuse `call:<id>` to an agent when the call's conversation is a person DM ("a huddle between two people is theirs: agents don't read it"). No wake.
- **Personal channel:** the person and their agents.
- Sharing a person DM's transcript with an agent is not in v1.
- The window's `talk_call_transcript` goes through the same read rule, then reads every line unpaged.

## Agents

- **`talk_read conv=call:<id>`:** with no read cursor, the first 30 lines under "The transcript, from the start", and "N more: talk_read conv=call:<id> since=<last>" when there are more. With a cursor, the unread lines under "Posts since you last read". An explicit `since=<line id>` pages by position.
- **The read cursor** for a `call:` conversation is `lines:<src>=<n>,…`: per source, the highest line index read. The source is the writing device for `call.segment` lines (index `seg`) or the post id for a fallback post's lines (index `i`).
  - Lines sort by when they were said, but arrive in each source's order. A late line said earlier would hide behind a single post-id cursor; per source it stays unread.
  - The cursor never goes back. An older cursor holding a plain line id still means "everything up to that line".
- **`talk_list`:** "Huddles with a transcript, newest first:", at most 10, each with its place, line count, live or ended, and its unread count (not counting the reader's own lines). The person sees all of them; an agent sees those of channels it is in, never a person DM's.

## The one wake

- **Text:** "The huddle in #x ended. talk_read conv=call:<id> has the transcript (N lines)." With no lines folded yet, "(its lines may still be arriving)".
- **Kind:** a reading notice (not a mention), sent with a wake, so agents learn of it at once without being addressed. Its reference is `talk:<conv>`, so a reply goes to the channel.
- **The last device out** (`tell_call_end`, only when the end mark has `transcript`): tells its own person's agents in the channel. That includes a Personal channel's agents; a person DM tells nobody.
- **Another person's devices** (`call_end_bridge`): tell their own agents when an end post with `transcript: true` arrives from the board log, through the bridge (receive setting, holds and rate limit apply).
- **Once per huddle:** both paths key the wake on the row `call-end/<id>` (`call_end_row`), so two end posts of one huddle wake an agent once.
  - `tell_call_end` skips an agent the bridge already woke for that row.
  - `call_end_bridge` skips when one of this person's own devices already wrote an end mark with `transcript` for the call, since that device told them.

## Consent

- On by default. "Transcribe what I say in calls" in Settings > App turns it off ("Huddles write down what each person says, on their own device. Turn this off and your voice isn't written down; you still hear and speak."). Turning it off mid-call stops the transcriber within a tick.
- Before the mic opens (Huddle, Join in the header or on a start post, Join on the ring banner), a dialog shows the notice with Start or Join and Cancel (`transcriptNotice`):
  - In a channel: "This huddle is transcribed. What you say is written down in #channel for its members and their agents."
  - In a DM: "This huddle is transcribed. What you say is written down for the two of you."
  - With the switch off: "Your voice isn't transcribed (Settings > App)."
  - When this device can't transcribe (no engine, or the model isn't downloaded): "Your voice isn't transcribed on this device."
- During the call: a "Transcript on" badge in the huddle's header while anyone is transcribed (its title: "Transcript on: each device writes down only its own person's voice."), "not transcribed" on the tile of anyone who isn't, captions under "Transcript, as it's written", and a red dot on the call bar.
- No audio is kept: only text leaves the transcriber.

## Android

The phone hears the same tap; no second mic is opened. Measured on the Pixel 10 Pro Fold (Tensor G5) with `crates/voice/examples/bench.rs`, base.en's final pass takes 5.7–6.2 s on 2 threads and 3.7–4.3 s on 4. So `Options::call()` uses 4 threads on Android, and a line lands about 4 s after it's said. A slower phone only lags: the feed keeps at most 25 s (`MAX_PHRASE`) of backlog. How to rerun it: [android.md](../android.md), Whisper during calls.

## Open

- The notice drops "this huddle is transcribed" when your own voice isn't. Others in the huddle may still be written down. A second clause ("Others' voices may still be written down in #x.") is a wording question.
- From irori-fit.md §4, for Stephen: a per-space switch, and a "hide my lines" row. A transcript is permanent on every member device, with no hard delete.
