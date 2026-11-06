# #571: how calls fit Irori (read-only study, 2026-11-06)

**Repo:** `this repo` @ 419641e9. I changed nothing in it. A copy of this report is at `a scratch folder/571-calls-fit.md`.

**Labels:** **V** means I read the file or line named. **I** means inferred or proposed.

**Sources:** the five cards #571, #541, #544, #537 and #240, read through irori `show`. Crate versions come from the crates.io API today.

---

## 0. Summary

1. **Where a huddle lives.** It belongs to a conversation: a space channel (the main case), a person DM, or a Personal channel. Its rows go in the log that conversation already uses. It never goes on a board.
2. **How row kinds work today.**
   - The private log stores any kind it doesn't know.
   - A space log is strict: an unknown kind stops that writer's rows on older devices.
   - Since **v0.12.4**, a space log keeps a pulled unknown `task.*` or `space.*` kind (the "tolerant reader").
   - `talk.*` and `call.*` are not covered by it. A new `call.*` kind would stall even v0.12.4 devices.
   - Each device announces a `tolerant` level in its hello and peers save it, but nothing gates on it yet.
3. **Safe path for new rows.**
   - First, ship a release that also tolerates `call.*` (and `talk.*`), at `TOLERANT_PROTOCOL = 2`, with a gate that reads that level.
   - Until that has spread, carry the few durable call markers inside `talk.post`. Older Irori shows them as plain posts.
   - Transcript segments go in rows only once that gate passes.
4. **Live media.**
   - A new ALPN `irori/call/1`, as its own QUIC connection on the existing endpoint. Opus audio goes in QUIC datagrams.
   - Admission works like boards: the space's verifier, plus a call-open proof keyed on the space secret.
   - Ringing goes live, as a new board `Wire` frame. Older builds skip frame types they don't know.
5. **The phone.**
   - It runs the full Core and iroh endpoint, so it joins like a desktop.
   - Not running agents (#541) doesn't matter for calls.
   - It needs an Android foreground service of type microphone, or the call drops when the screen goes off.
6. **Transcript.**
   - Each device transcribes only its own mic and writes its own signed segments. That gives speaker labels and per-person consent for free.
   - Whoever can read the conversation can read the transcript.
   - Agents read it through `talk_read` on a `call:<id>` conversation, or a new `call_read` tool.
   - Agents never see a person-DM call under today's rules.
7. **Whisper.**
   - whisper-rs 0.16.0 with the base.en model. It re-transcribes the current phrase every 700 ms and settles it after an 800 ms pause.
   - It opens the mic itself and runs one session at a time. Using it in a call needs a shared audio feed.
   - On Android it compiles and installs. Nobody has recorded a live result on the Pixel.

---

## 1. Where a huddle lives

### Verified

- **A space is the people.** It holds its boards, channels and threads; a board is only a task list. Sources: `docs/decisions.md` row 49 and `docs/design/talk-channels.md` §1 b4.
- **Talk never names a board.** `talk-channels.md` §2: "No `talk.*` row names a board."
- **Channel rows** go in the log their `spaceId` names (`talk-channels.md` §5–§6, `spaces.md` §6, the `crates/coord/src/talk.rs` module doc):
  - a space's log, `spaces/<id>.db`;
  - or the private log, when `spaceId` is empty (Personal).
- **Threads:**
  - A thread's `convId` is its task id.
  - A thread started from a space channel lives in that space's log (#558).
- **Person↔person DMs** ride a hidden two-person space:
  - Only `talk.post` and join `talk.member` rows pass its fold rule (`talk-dms.md` §4; `space_talk_row` at `crates/core/src/service/spaces.rs:718`).
  - `bridge_row` skips hidden spaces, so nothing in a DM reaches an agent (`crates/core/src/service/bridge.rs:107`).
- **DMs with an agent** stay in the private log.
- **Wakes (decision 46, b3):** channels wake agents only on @mention. Agents in a conversation always see it at their next turn.

### Recommendation

The huddle's `convId` is a channel id (`ch_…`), a person-DM id (`dm:…`), or a Personal channel. Its rows go in that conversation's log. That one choice settles who can join, who can read the transcript, and which gate applies, because all three already follow the log.

| Huddle in | Log | Who can join | Who can read the transcript | Notes |
|---|---|---|---|---|
| **Space channel** (main case) | `spaces/<id>.db` | Counted member devices of the space | Space members and their agents in that channel | Needs the space gate (§2) |
| **Person DM** | The hidden DM space | The two people's devices | The two people; no agents | The DM fold rule must be widened for any new row. The person can share the transcript on purpose (Make a card, or post it). |
| **Personal channel** | Private log | Your own devices (`ChainAdmit`) | You and your agents | Solo voice memos; testing Mac↔phone. No gate needed. |
| Board | — | — | — | No. Talk names no board. |
| Thread | — | — | — | Not in v1. Thread posts are mapped from ask and comment rows. A huddle started from a thread can post its summary there. |

In the window, the channel shows a one-line huddle post, like b2's thread pointer: "Huddle · 3 people · live", then "ended · 23 min · Transcript". The transcript opens as its own conversation.

---

## 2. Row kinds, without breaking older Irori

### Current mechanism (verified)

1. **The kinds registry** is `crates/store/src/kinds.rs`. Each kind has a `Side` and a `Board` verdict: `Accepts`, `Refuses` or `Unclassified`.
2. **The private log is permissive.**
   - `insert_one` (`crates/store/src/db.rs:1369-1376`) runs `check_pulled` only for a board store.
   - `check_row` (`db.rs:1255-1279`) refuses only Local kinds and invalid JSON.
   - So in Personal and the private log, any new kind is stored by older devices and folded after they update.
3. **A space log is strict.**
   - A device's own writes go through `check_shape`, which refuses anything not `Accepts` (`crates/store/src/board.rs:256`).
   - Pulled rows go through `check_pulled` (`board.rs:267`). It also lets through `board_tolerated(kind)`: an unknown kind starting with exactly `task.` or `space.` (`kinds.rs:399-404`).
   - Unknown `board.*`, `member.*`, `person.*` and `agent.*` kinds stay refused on purpose.
4. **The pin test** is `crates/store/tests/it/board.rs:234`, `no_tolerated_family_kind_becomes_refused`.
   - It asserts that the only `task.*`/`space.*` kinds not marked `Accepts` are these eight: `task.archived`, `task.comment.edited`, `task.deleted`, `task.edited`, `task.paused`, `task.pinned`, `task.shared`, `task.where`.
   - Its doc says: "A new kind of these families is Accepts, or a family of its own."
5. **The tolerant reader first shipped in v0.12.4.** `git tag --contains df2835a5` (the #579 merge) lists only `v0.12.4`. Devices on v0.12.3 or older refuse even unknown `task.*`/`space.*` kinds.
6. **A refusal stalls that writer's rows on that device** (`no-server.md` §2).
   - Rows behind it go unseen, including removals and revokes.
   - It heals when the device updates.
   - Since #579, the stalled writer backs off per peer, up to an hour, and pulls take turns between writers.
7. **Hello levels** live in `crates/mesh/src/boardmesh.rs:73-106`: `SPACES`, `SPAWNS`, `MOVES`, `DMS` and `TOLERANT_PROTOCOL`, each 1.
   - `tolerant` is in `BoardHello` (`:161`), `BoardReads` (`:302-307`, which feeds `board.server` `reads`) and `SavedHello` (`:326`).
   - But `DeviceWords` only has a `spaces(dev)` accessor (`:410-417`).
   - The gate (`space_readers` and `space_not_ready`, `spaces.rs:275-331`) blocks only devices known to be below `SPACES_PROTOCOL`. Devices nobody has heard from get a warning (decision 48).
   - `crates/core/src` never mentions `tolerant`, so no gate uses it.
   - Space Talk rows already pass through this gate (`SPACE_TALK_KINDS` at `spaces.rs:673`, `gate_refusal` at `:662`).
8. **Older decoders drop unknown fields.** `coord::talk::PostRow` is `#[serde(default)]` with no `deny_unknown_fields` (`talk.rs:249-264`). So a `talk.post` with an extra `call` object reads as a normal post on any Talk-era Irori.
9. **The DM rule.** A hidden space's `append_space_talk` (`spaces.rs:808`) refuses every kind except `talk.post` and join `talk.member`, before appending.
10. **`talk.post.attach` exists but isn't wired.**
    - The field is there (`talk.rs:260`), but Talk always writes `attach: None` (`talk.rs:887`).
    - Board file serving needs an `attach.offered` row (marked `Accepts`) in that space's log (`boardmesh.rs:1174`, `:1513-1523`).

### What a call records

**Rule:** rows are the durable, signed record. Live state is not rows. Phones lag minutes behind (`docs/android.md` "In the background"), and relayed rows took 3.5–19 minutes in #544.

| Thing | Row? | Proposed shape | Written by |
|---|---|---|---|
| Call started | Yes | `call.started {callId, convId, by, at}` | The starter's device |
| Joined / left | Yes | `call.joined {callId, device, transcribe, at}` / `call.left {…}` | Each participant's own device |
| Ended | Yes (soft) | `call.ended {callId, by, at, durationMs}` | The last one out. The fold also ends a call with no live participant after a timeout, for a crashed device or frozen phone. |
| Transcription consent | Yes | `call.joined.transcribe`, plus `call.consent {callId, transcribe, at}` | Each person's own device |
| Transcript segments | Yes, but only after the gate | `call.segment {callId, seg, startMs, endMs, text, lang}`, one phrase per row | The speaker's own device only |
| Screen share start/stop | Optional | `call.share {callId, device, op, at}` | The sharer |
| Ringing, mute, speaking, live captions | **No** | A `Wire::Call` frame and the call connection | — |

**Size:** one phrase per row comes to roughly 400–800 rows for an hour with two speakers. That fits the 2000-row pull cap. But every member device keeps them forever, and there is no hard delete.

### Options

**A. New `call.*` kinds now, behind today's gate: unsafe.**
Every device refuses `call.*`, v0.12.4 included. The `spaces >= 1` gate doesn't help, since every current device already says 1.

**B. Name them `space.call.*`: works on v0.12.4+, but it's a loophole.**
- Pulled rows are tolerated (`kinds.rs:399`).
- They don't collide with board folding:
  - `fold_space_boards` (`spaces.rs:103`, with helpers `:79-162`) compares exact kinds.
  - `FOLLOW_KINDS` holds the prefix `"space.board."`, not `"space."` (`crates/coord/src/board/fold.rs:100-122`).
- They still need a new gate on `tolerant >= 1`, because v0.12.3 and older refuse them. That means adding `DeviceWords::tolerant()`.
- They blur the `space.*` family, which is meant for space structure.

**C. Markers inside `talk.post`: works today, with no new kind and no new gate.**
- Shape: `talk.post {text: "Stephen started a huddle", mentions: [], agentsNow: false, call: {callId, op, …}}`.
- Older Irori shows the text. It passes the existing space Talk gate and the DM rule.
- On older builds, each post gives channel-member agents a next-turn reading. So use it for start and end (and at most join and leave), never for segments.

**D. Transcript as a file attachment.**
- Offer `transcript-<callId>.jsonl` with `attach.offered` and link it from the "ended" post's `attach` field. This needs wiring, but no new kind.
- The file is fetched on demand from a device that holds it. If that device sleeps, nobody can read it.
- Agents would need `attachment_get` plus a file read.
- It doesn't work in a person DM: the DM rule refuses `attach.offered`. A DM call gets option C only until `call.*` is tolerated.

**E. Widen the tolerant reader: the lasting fix (`no-server.md` §3.5).**
- `board_tolerated` also accepts unknown `call.*`, and `talk.*` too, which is harmless content rather than membership. Set `TOLERANT_PROTOCOL = 2`.
- Add `DeviceWords::tolerant()` and a gate that blocks only devices known to be below 2.
- Extend the pin test's rule:
  - The test's list must also hold `talk.read`, which is `Refuses` today. That's safe, since every Talk-era build already knows it.
  - Every new `talk.*`/`call.*` kind must be `Accepts`, never `Refuses`.
- Ship it one or two releases before calls, so that with "Keep this device up to date" (#578) most devices are past it by the time calls arrive.

### Recommended path

1. **Release N, independent of calls:** option E.
2. **Calls v1:**
   - Option C for the started and ended markers, with a participant summary on "ended".
   - The live transcript goes over the call connection.
   - Stored transcript:
     - as `call.segment` rows where every space device is known to be at `tolerant >= 2`;
     - otherwise as an option D file;
     - in a DM, widen the DM rule to cover `call.*` (rules 1–4 of `talk-dms.md` §4 apply unchanged).
3. **Personal:** any `call.*` kind can be written from day one.

---

## 3. Live media on the existing iroh endpoint

### Verified

**The endpoint.**
- `IrohNet::build` (`crates/net/src/transport.rs` ~`:875-905`).
- Its key is the device seed, so `EndpointId` equals the certified `devicePub`.
- It's built from `Builder::empty()` with pinned relay TLS, no pkarr, and n0's four relays when public relays are on (decision 43).
- ALPNs: `irori/mesh/1`, `pair/1`, `board/1`, `invite/1`, `gossip/1` (`:60-65`, `:882-889`).

**Admission.**
- It runs in `after_handshake` (`Gate`, `:292-322`).
- `MemberAdmit::admits` returns false for any other ALPN (`crates/net/src/board.rs:103-111`).
- `accept_loop` closes an unknown ALPN with `PROTOCOL` (`:1314-1340`).
- Board connections then send a `board_open_proof`: an HMAC over the board-topic key, `"irori/board-open/1"`, the TLS exporter key and the role.

**Who dials.** The lower device id. A duplicate connection closes as `REPLACED`.

**The mesh seam has no datagrams.** `Conn` in `crates/meshseam/src/lib.rs:258-281` has `send`, `recv` and `open_stream` only. `IrohConn::connection()` (`transport.rs:1432`) exposes the raw iroh `Connection`.

**Board frames.**
- Older builds skip frame types they don't know (`boardmesh.rs:1898`).
- `Wire` (`:601`) holds `hello`, `vector`, `ping`, `pong`, `board.secret` and `dm`.
- `send_to` (`:1119`) and `live_peers()` (`:1102`) already exist for the DM hand-off.

**The spike** (`crates/net/src/bin/spike-call.rs`, path reporting in `path.rs`):
- It used `presets::N0` and n0's address lookup.
- In iroh 1.4.1, the default `QuicTransportConfigBuilder::new()` that IrohNet starts from is the same base the spike used (`src/endpoint/quic.rs:152-164`). IrohNet only sets bidi streams on top.
- So datagrams should also work on the real endpoint. That's inferred; I didn't run it.
- Results:

  | Link | Path | p50 | p95 |
  |---|---|---|---|
  | Mac↔phone, same LAN | direct | 12 ms | 17 ms |
  | Mac↔PC | direct | 26 ms | 269 ms (spikes to 930 ms) |
  | Mac↔PC | relay only | 124 ms | — |
  | phone↔PC | direct | 33 ms | 680 ms |

  Every run started on relay and went direct within 1 s.

### Proposed design (inferred)

**The connection.** `irori/call/1` is its own QUIC connection, so it has its own congestion control. It doesn't share bandwidth with 2000-row pulls or 256 MiB blob fetches on the board connection.
- It reuses what the endpoint knows about relays and addresses. iroh 1.x does NAT traversal per connection, which took under 1 s in the spike.
- An older peer doesn't list the ALPN, so the handshake fails cleanly: "their Irori is too old for calls."
- Add a `calls` level to the board hello, so the window can say so before dialing.

**Admission.**
1. `MemberAdmit`: `CALL_ALPN` is admitted when `chain.admits(..)` holds or the peer is on a board held here. Add a branch in `accept_loop`.
2. A first frame `CallOpen {v, spaceId, epoch, callId, deviceId, proof}`:
   - `proof = HMAC(DeriveBoardTopic(spaceId, epoch, secret).key, "irori/call-open/1" || callId || EKM32 || role)`.
   - Then the space's verifier checks the device with `admitted_by_pub`, as `irori/board/1` does.
   - Personal calls use the mesh secret and `ChainAdmit`.
   - DM calls use the hidden space's secret.
   - A member removal or device revoke closes call connections through the same `recheck` path boards use.

**Ringing.**
- `Wire::Call {spaceId, callId, op: ring|join|leave|end, convId}`, sent to each live device that said `calls >= 1`.
- A sleeping phone isn't rung. It sees the `talk.post` marker when it next syncs. There is no push and no server (decision 48).

**Who dials whom.** Within the call, the lower device id dials and a duplicate closes as `REPLACED`. This matches mesh and board.

**Topology and media.**
- **Topology:** a full mesh for 2–5 people. Opus at about 32 kbit/s times (N−1) uplinks is fine. No SFU (an in-call forwarding server).
- **Audio:**
  - One cpal capture runs through echo cancellation and noise suppression.
  - It feeds an Opus encoder, in 20 ms frames sent as datagrams with a sequence number and timestamp. The same capture feeds the transcriber.
  - An adaptive jitter buffer handles the PC's ~270 ms p95. Some frames will be concealed rather than played.
- **Screen share:**
  - It's the heavy part: about 1–3 Mbit/s per viewer, and the sharer uploads once per viewer.
  - Frames go on unidirectional QUIC streams.
  - **Mac:** it needs the Screen Recording (TCC) grant. The grant is tied to the code signature, so it survives updates only once releases are Developer ID signed. That's #537, which is waiting on 0.12.5 to confirm a grant survives an update.
  - **Windows** needs no grant.
  - **Android** capture is out of scope for v1.
- **Libraries** (from the crates.io API today; the real comparison is s1's job):
  - `opus` 0.4.0, released 2026-08-23. Avoid `audiopus` 0.2.0: its last release was 2021-04-22. (https://crates.io/crates/opus, https://crates.io/crates/audiopus)
  - `webrtc-audio-processing` 2.1.0, released 2026-05-13, for echo cancellation and noise suppression.

**The phone.**
- It runs the same Core and endpoint. `docs/android.md`: "The iroh endpoint and the timers keep going for as long as Android lets the process run."
- `spawn_capable.rs` only concerns spawning agents. A call starts none.
- What's missing:
  1. **A foreground service of type `microphone`.** Today there's no service, no `FOREGROUND_SERVICE` permission and no wake lock, so Android freezes the backgrounded app. Android 14+ needs `FOREGROUND_SERVICE_MICROPHONE` (developer.android.com/develop/background-work/services/fgs/service-types; I didn't fetch it this session).
  2. **Audio routing and the voice-communication input preset.** Whether cpal 0.18.2 exposes that preset is unverified.
  3. **Ringing** only works while the app is open.

---

## 4. Transcript

### Recommendation (inferred)

**Each device transcribes only its own mic** (the audio after echo cancellation), when its person has transcription on, and writes its own signed segments.
- Speaker labels come free.
- No audio leaves a device for transcription, which keeps #240's promise.
- Consent is built in: a person who opts out produces no text, and nobody else's device can transcribe them.
- A participant without Whisper shows as "[not transcribed]". Transcribing someone else's received stream would need their explicit opt-in. Leave that out of v1.

**Live:** segments travel as a `transcript` frame on the call connection, and the window shows live captions.

**Stored:** in the conversation's log, as `call.segment` rows behind the gate, or as the option D file.

**Who can see it** is whoever can read the conversation:
- **Space channel:** the space's members, and their agents in that channel (b3: agents see it at their next turn).
- **Person DM:** the two people only. `bridge_row` skips hidden spaces, so no agent sees it unless the person shares it.
- **Personal:** you and your agents.
- `talk.read` cursors stay private, since board logs refuse them.

**How agents read it.** The tools are `crates/tools/src/talk.rs` (`talk_read` at `:1400`), registered in `crates/tools/src/lib.rs:355,412`.
- **Simplest:** `convId = call:<callId>`. `talk_list` lists it, and `talk_read {conv: "call:<id>"}` pages through segments as posts (by the speaker, at the segment time), up to `MAX_PAGE` 200 (`talk.rs:125`).
- **Optional:** a `call_read {call, since?, format: text|summary}` tool that merges each speaker's phrases into paragraphs. `search` and `show` could index segments.
- **Wakes:** one reading notice per huddle, when it ends. Never one per segment. A name heard in speech never wakes anyone, since decision 39 needs mention keys, not names.
- **Trust:** the tools already label text as untrusted. Decision 50 counts other people's words as other people's text.
- **During a call:** an agent can call `talk_read` mid-call if segments are written as each phrase ends. With the file fallback, only after the call.

**Consent and notice.**
- Decision 52 says a feature works by default and a setting only turns it off. So: transcription is **on by default**, with a per-person switch "Transcribe what I say in calls".
- The join sheet says, before the mic opens: "This huddle is transcribed; the transcript stays in #channel for its members and their agents."
- An indicator stays up during the call, and each participant shows as transcribing or not.
- `call.joined.transcribe` records each person's choice, signed by their device.
- No audio is stored in v1.
- For Stephen (not legal advice): some places require every party's consent, and a transcript is permanent on every member device, with no hard delete. Does he want a per-space switch, and a "hide my lines" row like `task.deleted`?

---

## 5. Whisper today, calls, Android

### Verified

**Versions.** `whisper-rs 0.16.0` (with Metal on macOS) and `cpal 0.18.2` (`crates/voice/Cargo.toml`).
- `whisper-rs-sys` is patched to the StephenSHorton fork for the Android cross-compile (root `Cargo.toml:183-187`; #240 b3).
- All three are the latest releases: whisper-rs 0.16.0 (2026-03-12, ~1.9 M downloads, repo on Codeberg), cpal 0.18.2 (2026-08-16), iroh 1.4.1 (2026-10-21).

**Model.** `ggml-base.en.bin`, 148 MB, English only, SHA-256 checked, downloaded once from Irori's `models-whisper` release (`lib.rs:41-46`, `model.rs`). #240 notes it's weak on numbers and names.

**Algorithm** (`crates/voice/src/whisper.rs`):
1. cpal opens the default mic; audio is mixed to mono at 16 kHz.
2. An energy gate over 30 ms frames, with an adaptive noise floor, marks speech.
3. Every 700 ms (`TICK`) the whole phrase so far is re-transcribed as a partial.
4. After 800 ms of silence (`PAUSE`), or at 25 s (`MAX_PHRASE`), the phrase gets one last full pass as a `Final` (`:28-37`, `:239-290`, `:410-460`).
5. Decoding is greedy, with no context, a single segment and no timestamps.

**Sessions.**
- Each `start()` gets its own thread and one `WhisperState`. The loaded model is cached between sessions (`:150-190`).
- `start()` always opens the default mic itself.
- The only other source is a whole `Vec<f32>` buffer, through the `#[doc(hidden)]` `start_with_samples` (`lib.rs:189-198`), which exists for tests.

**Measured (#240):**
- Mac (Metal): 19–45 ms per pass.
- PC (CPU): exact finals on text-to-speech audio, with about 0.6 s partials.

**Platforms.** The Whisper engine compiles for Windows, macOS and Android (`lib.rs:149-160`).
- The fallback in `other.rs` now covers only other OSes.
- The module doc at `lib.rs:22-25` still says Android is unsupported. That doc is stale.

### Can it run during a call? (inferred)

Yes, with three changes:
1. **One capture, two consumers.** Add a push source, e.g. `Source::Feed(Receiver<Vec<f32>>)`. The call's capture, downsampled to 16 kHz after echo cancellation, feeds the transcriber, so the voice crate doesn't open the mic a second time.
2. **Finals only during calls.** Drop the 700 ms partials and cap `set_n_threads`, especially on the loaded PC. A finished phrase (cut at a pause, at most 25 s) is the natural unit for a segment row.
3. **Transcribe after echo cancellation.** Otherwise the other side's voice from your speakers gets transcribed as yours.

The text lags by about the pause plus one pass, roughly 1 s on the Mac. That's fine for captions and storage.

### Android

- **Verified:**
  - The engine builds for aarch64-android: API 26, with the toolchain file `scripts/android/arm64.toolchain.cmake` (`docs/android.md` "Voice input").
  - `RECORD_AUDIO` is in the committed manifest.
  - #240's close-out says on the Pixel it "builds, installs and starts, and RECORD_AUDIO is requested".
- **Not verified:**
  - A live dictation result on the Pixel. #240 b5 left the phone test to Stephen and shipped untested.
  - CPU speed of base.en on the phone's CPU.
  - Battery and heat over a long call.
- **Recommendation:** in v1 the phone joins audio, and transcribes only if a measured Pixel test shows it keeps up. Otherwise it shows as "not transcribed".
- Android's Open Settings for the mic still returns `Err`.

---

## 6. Questions for Stephen

1. Huddles in channels and person DMs from v1, or channels only?
2. Transcription on by default with a per-person off switch (decision 52) plus the notice in §4? A per-space switch too? A "hide my lines" option?
3. Should person-DM call transcripts stay away from agents unless shared?
4. Ship the tolerance widening (option E) a release ahead of calls?
5. Should the phone be a full participant (a foreground service and a notification during calls), or only join while the app is open?

## 7. File index

- **Design docs:** `docs/design/{talk-channels,talk-dms,spaces,spaces-boards,no-server,public-relays,boards2-transport}.md`, `docs/architecture.md` §3, `docs/decisions.md` rows 43, 46, 48–52, `docs/android.md`.
- **Store:**
  - `crates/store/src/kinds.rs`: the registry; `board_tolerated` at `:399`.
  - `crates/store/src/board.rs`: `check_shape` `:256`, `check_pulled` `:267`.
  - `crates/store/src/db.rs`: `insert_one` `:1369`.
  - `crates/store/tests/it/board.rs:234`: the pin test.
- **Mesh:** `crates/mesh/src/boardmesh.rs`:
  - protocol levels `:73-106`, hello `:128-164`;
  - `BoardReads` and `SavedHello` `:302-360`, `DeviceWords` `:384-417`;
  - `Wire` `:601`, `live_peers` `:1102`, `send_to` `:1119`, skipping unknown frames `:1898`.
- **Core:**
  - `crates/core/src/service/spaces.rs`: `fold_space_boards` `:103`, `space_readers` `:275`, `space_not_ready` `:317`, `gate_refusal` `:662`, `SPACE_TALK_KINDS` `:673`, `space_talk_row` `:718`, `append_space_talk` `:808`.
  - `crates/core/src/service/dm_spaces.rs`, `bridge.rs:107`, `spawn_capable.rs`.
- **Talk and tools:**
  - `crates/coord/src/talk.rs`: rows `:213-283`, fold `:1141`.
  - `crates/coord/src/board/fold.rs:100`: `FOLLOW_KINDS`.
  - `crates/tools/src/talk.rs:1400`, `crates/tools/src/attachments.rs`.
- **Net:**
  - `crates/net/src/transport.rs`: ALPNs `:60-65`, `Gate` `:292`, `build` `:875`, `accept_loop` `:1314`, `connection()` `:1432`.
  - `crates/net/src/board.rs:103`: `MemberAdmit`.
  - `crates/net/src/endpoint.rs`, `crates/net/src/path.rs`, `crates/net/src/bin/spike-call.rs`.
- **Seam:** `crates/meshseam/src/lib.rs:258-281`.
- **Voice:** `crates/voice/src/{lib,whisper,other}.rs`, `crates/voice/Cargo.toml`.
