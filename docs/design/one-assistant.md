# One Office Assistant across your devices: design

*2026-11-08, card #1095, from the person's words on thread #1093. Design only: nothing here is built yet. The build cards are in §12. Checked against main at `62e5f59a` by five adversarial reviews (split-brain, buildability, security, failure and what the person sees, consistency with the other docs), each checked by a skeptic. What they found is folded in. Line numbers are approximate: go by the names.*

The person asked on #1093 whether the Office Assistant starts on only one of their devices, "because all my devices can share that assistant". Then, after hearing that today each device runs its own:

> "I was kind of implying I want to commit to a design where there is only one assistant started. I think it would help conversation context share across the devices if one Claude that is started for that is handling it across devices. If you can't connect, you got more issues than that."

So this design has one assistant per person. It runs on one computer, the assistant's computer, and it handles the chat from every device, the phone included. Every device shows the same conversation. When that computer can't be reached, the chat says so plainly, and no other device starts a second assistant.

## 1. Decisions

| | Decision | Why |
|---|---|---|
| D1 | **One assistant per person. Its chat turns run only on the assistant's computer.** Every other device, the phone included, sends its questions there and shows the replies. No other device runs a chat turn. | The person's words on #1093. |
| D2 | **The assistant's computer is recorded in a row, not worked out from who is online.** It stays the same while that computer is off. | Today's pick moves to whichever desktop is up when the home drops (§2), and that is a second assistant starting. |
| D3 | **It moves only when the person picks another computer, or the computer is removed from their devices.** When it is off, asleep, signed out of Claude Code or missing Claude Code, the chat says so, names the computer and says what to do there. | "If you can't connect, you got more issues than that." Moving on a guess, or on a check that can fail for a moment, is how two assistants start. |
| D4 | **It is set by itself the first time.** A computer that can run the assistant claims itself when there is no claim. The first claim wins, and a person's pick beats any claim made by itself. | Decision 52: it works with no switching on. Settings only changes it. |
| D5 | **The conversation is rows on the person's private log**: questions, replies, the chat's follow-up lines and New chat. Every one of the person's devices shows the same conversation. No other person ever gets the rows, and no tool shows their text. | Shared context is the point. The private log reaches only the person's own devices. |
| D6 | **A question row is the request.** The assistant's computer answers each unanswered question as its row arrives, one at a time, oldest first. | It needs no new transport. A question reaches the computer through any device that carries it, and waits safely while the computer is away. One turn at a time keeps one conversation, and keeps #987's grants safe (§5). |
| D7 | **First, one turn per question over the shared rows. Then a held Claude session on the assistant's computer (card 8), measured first, and dropped only if the person agrees.** The rows are the record. The session is a warm copy that can always be rebuilt from them. | The person asked for "one Claude that is started for that". The rows are needed either way, since a session has to be seeded from them whenever it starts again. §8 compares the two. |
| D8 | **When the computer is away, the question waits, and the chat says so plainly.** When it is back, it answers what waited, the newest five at most, each told how long ago it was asked. A late question makes no changes. | Plain status, no second assistant. They are the person's own questions, so they get answers, not a "too late" line. But a change asked for hours ago may no longer be wanted (§5, Grants). |
| D9 | **The turn, its tools, its grants and thread posting run on the assistant's computer.** Going to a page (`<<go>>`) happens only on the device that asked, and only for a reply that just arrived. Taps work on any device. | Navigation belongs to the screen the person is looking at. Everything else is the one assistant's work. |
| D10 | **Answering other people (assistants-talk) and the huddle assistant (call-agents) use the same recorded computer.** The person's taps (**Let it answer**, **Answer this**) are sent to it too, so they work from a phone. | One assistant. Today those designs run turns on the device tapped, or fail over to the next device. §9 lists the changes. |

## 2. What is there today

Checked on main at `62e5f59a`:

- **The chat runs on whichever device it is typed on.** `Window::assistant_ask` (`crates/window/src/assistant.rs`, around line 1094) renders the prompt, sets the screen snapshot, begins grants and starts one `claude -p` through the window's `assistant` `Runner`. The flags include `--no-session-persistence`, so there is no session between turns. Model and effort come from `spawn_setting("")`, which is one row for every device (default Sonnet, low, since #1014). Replies stream to the panel over a Tauri `Channel<AssistantEvent>`.
- **Tools run on the device that runs the turn.** `irori mcp` dials the local Core (`app/src/mcp_cmd.rs`), and the caller is `user:<that device>` with kind `assistant` (`crates/api/src/server.rs`). Most of `ASSISTANT_TOOLS` read replicated rows. `screen` reads a slot that this Core's last ask set (`Core::set_assistant_screen`). `timeline` and `why` read this device's capture.
- **History is the webview's localStorage**, key `irori.assistant.history` (`frontend/src/lib/assistant.ts`, `loadHistory`/`saveHistory`), capped at 50 entries. The follower's memory of what it has seen and said is `irori.assistant.follow`. Both are per device. The frontend sends `historyFor(entries)` with each ask, and the host renders the last 12 turns (`render_prompt`, `HISTORY_TURNS`).
- **A reply can post a thread by itself.** `finish()` calls `autoPost()` when `postsByItself` holds, whether the panel is open or closed. It calls `BoardService.StartThread` with `assistant: true` and then follows the thread. Going to a page (`<<go …>>`) changes this window's route, only for a live reply and only while the panel is open: never from saved history.
- **#987's grants are one set per Core.** `Grants::begin` replaces the set on each run. It is built by `grants_for(text, earlier)` from the person's typed message and earlier typed messages in this device's history. The doc comment says it relies on one question at a time. The API lets any caller holding the assistant secret spend an open grant (`crates/api/src/v02.rs`), with no idea of which run began it.
- **The phone shows the chat**, but every ask ends in `NO_CLAUDE`: "The assistant can't run on this device yet" (`assistant.rs`). The phone runs a full Core and replicates the whole private log (`app/src/lib.rs`).
- **The assistant's computer is worked out on each device, and it moves.** `assistant_home_of` (`crates/core/src/service/spawn_capable.rs`) takes the first of `AgentDevices.homes` whose `spawn.host` says `assistantTurn`. `refresh_agent_devices` ranks `homes` by `(tier, no trusted folder, id)`, and tier 0 is "this device, or one connected now". So:
  - A desktop always ranks itself first among the devices it can't reach. Two desktops that can't see each other both pick themselves.
  - When the home goes quiet and another desktop is connected, that desktop becomes home within about 50 s, and the home moves back when the first returns. There is no grace period.
  - `is_assistant_home` has one caller, `mint_assistant_cards` (`crates/core/src/service/boards.rs`).
- **`assistantTurn` means the `claude` command was found, not that it is signed in.** It is `ready && os != "android"` in `publish_host`, and `ready` is `find_claude().is_ok()`, read again on every sweep. The comments say "signed in".
- **Of assistants-talk's build cards, only card 2 (#1058) is on main**: the assistant card, its profile pointer and `assistantTurn`. Card 1 (#1057, the channel) and #1045 (naming) have open PRs. Cards 3 to 8 (#1059 to #1064) are not built. The huddle assistant (call-agents §7) is not built.
- **Device to device, there are only rows and the private mesh.** A row is pushed to connected devices as soon as it is appended (`crates/mesh/src/node.rs`, `wake_all`), and every Core subscriber sees it as it is inserted (`Store::subscribe`). A subscriber more than 256 rows behind gets a resync instead of the rows. Request rows like `spawn.end.requested` are served by the 5 s sweep, not on arrival. Rows from different writers arrive in no promised order. The only request and answer stream on the mesh is the attachment fetch (`Node::fetch_file`, a `blob/<id>` stream), which reports `FetchError::Offline` when the holder isn't connected. There is no LAN HTTP between devices.
- **The mesh knows when this device is caught up with a peer.** `check_step` (`crates/mesh/src/node.rs`, card #580) notes when this device holds every row the peer's newest vector names.
- **Rows are permanent.** The private log has no size limit per row, no retention and no delete (storage.md §2, §4; decision 25). The mesh sends one message of at most 8 MiB.
- **The `events` tool and `GET /v1/events` return rows whole** for any kind except `message.sent` and `capture.turn`, whose text they leave out (`crates/tools/src/observe.rs`, `crates/api/src/redact.rs`). Every agent of the person's has `events`.

## 3. The assistant's computer

### The row

A new private kind, `assistant.home`, declared in `crates/store/src/kinds.rs` beside `assistant.names.set` as `Replicated, Refuses`:

```
{ "device": "<device id>", "how": "auto" | "person", "by": "user:<writer device>", "at": <GoTime> }
```

Actor `user`. Any of the person's certified devices may write it. The private log accepts only those devices, and the fold skips rows with another actor, as `newest_switch` does.

### Which row wins

A new fold in a new `crates/core/src/service/assistant_home.rs`. All ordering is the store's own order, `(hlc, device_id, seq)`, which holds causal order whatever the wall clocks say.

1. **A person's pick beats any automatic claim.** Among `how: "person"` rows, the newest wins.
2. **Among automatic claims, the first wins.** A late claim never takes over from an earlier one. So a newly paired desktop that claims itself loses as soon as it has synced.
3. **A removed computer ends the rows before its removal.** A row naming a revoked device never counts, whenever it was written. And if the winning row names a device that is revoked, every row older than that device's `person.device.revoked` row stops counting too, and the fold runs again on what is left. So after a removal, an old claim the person had moved away from never comes back. Only a claim written after the removal can win, and the next sweep on an able computer writes one.
4. **No row left means no assistant's computer yet.**

"Revoked" comes from `chain.devices()` (`DeviceState.revoked`, `crates/identity/src/chain.rs`) or the Core's `revoked` check. Never from `is_owner` or `hosts()`, which still count revoked devices.

The fold doesn't look at `spawn.host` at all. A computer whose `assistantTurn` drops, for a moment during a Claude Code update or for good after an uninstall, stays the assistant's computer, and the chat says Claude Code is missing there (§6). Every device folds the same rows to the same answer, whatever it can reach and whatever its `spawn.host` rows say. That is what makes the pick stable.

### Setting it by itself

In `sweep_spawns` (`crates/core/src/service/spawn.rs`), after `refresh_agent_devices` and before `mint_assistant_cards`, a device writes `{device: <this device>, how: "auto"}` when all of these hold:

- The fold has no assistant's computer.
- This device's own `spawn.host` says `assistantTurn`. From card 3 on, it must also say `assistantChat` (below).
- Today's ranking, `assistant_home_of(homes, hosts)`, names this device, taken over only the computers whose newest `spawn.host` says every field the claim needs (from card 3, `assistantChat` too). Two connected desktops then agree on which of them claims, and a desktop left on an older build never blocks an updated one.
- **This device is caught up with every one of the person's devices it is connected to** (below). That keeps a newly paired desktop, or one back from a long absence, from claiming before the existing claim has reached it.

A device only ever claims itself. It never writes an automatic row naming another device, and writes nothing while there is an assistant's computer, even one that has been offline for days.

### When it moves

Only these move it:

- **The person picks another computer**, in Settings › Agents or with the one tap on the away line (§6). Either writes a `how: "person"` row, from any device, the phone included.
- **The computer is removed** from the person's devices (rule 3).

Nothing else moves it: not being offline, asleep or unreachable, not a slow link, not a missing or signed-out Claude Code, not another desktop with a lower id or a trusted folder, and not Stop here.

### Caught up

A new `Node::caught_up() -> bool` (`crates/mesh/src/node.rs`): true when, for every one of the person's devices this one has a live session with, this device's vector covers that device's newest vector (the same `covers` check `check_step` makes). It leaves out writers in the node's `stopped` map (a fork or a bad HLC order: their rows will never arrive) and revoked devices' rows past their cut, so a stuck writer can't hold the assistant forever. With no own device connected, it is true. Both the claim and the worker (§5) use it.

*Built in card 1 (#1101).* The peer's vector it compares with is the one the peer last sent: at connect and after it takes rows. Rows a peer wrote since arrive by push and are not in that vector, so `caught_up` can be true for a moment while a peer's newest rows are on their way. It is false while the peer's rows are held back (a clock ahead) once the peer has said so. Until a session is up it is true (a device that has just started and is not yet connected can claim; the sweep runs several seconds after start).

### Saying it can answer from other devices

A new `assistantChat: true` field on `spawn.host`. Card 1 adds it to `SpawnHost` as a read-only field (`#[serde(default)]`), so the status can read it. Card 3 publishes it, never before. It is `assistantTurn && the chat hook is installed in this run` (`set_chat_turn`, §5). Like `assistantTurn`, it joins `publish_host`'s change tuple, or an upgraded desktop never publishes it. It must never ship in a build without the worker: then it would promise answers nobody gives.

It is read by the claim (from card 3), the Settings picker, **Use PC instead**, and the "needs the latest Irori" line (§6).

### Reading it

- `Services::assistant_home() -> Option<String>`: the fold's answer, cached. It is refolded when an `assistant.home` or `person.device.revoked` row arrives (branches in the kind loop beside `ASSISTANT_NAMES_SET`, `crates/core/src/service.rs` around line 2063) and on the loop's resync. When the answer changes, it tells a listener, which card 3's worker uses to run its catch-up scan, and publishes a new `CoreEvent::AssistantHome`, pumped to the window as `assistant:home`.
- `is_assistant_home()` reads it, not the ranking. So `mint_assistant_cards`, and every later caller, get the recorded computer.
- `Core::assistant_home_status()`, wrapped by a Tauri command of the same name (since `device_presence` is `pub(crate)`), returns:
  - `device`, `name`, `here`, and `state` and `lastSeen` from `device_presence` (`crates/core/src/service/liveness.rs`);
  - `claude` and `chat`: whether that computer's newest `spawn.host` says `assistantTurn` and `assistantChat`, read with a per-device query (`device_id`, newest first, limit 1), not from `hosts()`, which reads only the newest 1,000 rows;
  - with no assistant's computer, `none`: `"setting_up"` (a computer says every field the claim needs, so a claim is coming), `"needs_update"` (one says `assistantTurn` but, from card 3, none says `assistantChat`) or `"no_claude"` (none says `assistantTurn`).

  The chat and Settings show it (§6).

### Stop here and Stop everywhere

Decision 60's switches stop agents and hold their starts. The assistant isn't an agent: it is off the roster and counted against no limit. So:

- They don't stop the person's own chat. The person is asking.
- They hold what the assistant does by itself: automatic answers to other people become "Ask me" (as assistants-talk §5 already says for Pause all), and the huddle assistant stays out of calls.
- They never move the assistant's computer.

## 4. The conversation as rows

### Kinds

All are private, `Replicated, Refuses`, declared in `kinds.rs` with an `assistant.` prefix and added to `SINCE_GO_REFUSED` in `crates/store/tests/it/golden.rs`. The prefix is outside `TOLERATED_FAMILIES`, so `Refuses` is allowed.

| Kind | Actor | Written by | Payload |
|---|---|---|---|
| `assistant.chat.asked` | `user` | The device the person typed on, through a host command only | `turn` (a new ULID), `text`, `screen` (that device's snapshot, cut to 4,000 characters as today), `device`, `at`, optional `again` (the turn it repeats), optional `imported`, optional `callId` (call-agents card 11 adds it, for Divide) |
| `assistant.chat.said` | `irori` | The assistant's computer | `turn`, `text`, `changed` (the write tools' lines), `posted` (`{id, short, tag}` of a thread it posted), `looked` (the screen label, not its text), `error`, `skipped`, `device`, `at`, optional `imported` |
| `assistant.chat.update` | `irori` | Any device's follower; the assistant's computer for assistants-talk's `held` lines (§9) | `key` (a hash of the thread or post id and the fact's key), `thread`, `text`, `act` (as today's `{kind: "answer", card, blocker}`), `request` (when it ends a tap's request), `at` |
| `assistant.chat.posted` | `user` | The device where the person tapped **Post thread** | `turn`, `posted` |
| `assistant.chat.cleared` | `user` | The device where the person tapped **New chat** | `by`, `at` |
| `assistant.chat.stop` | `user` | The device where the person tapped Stop | `turn`, `by`, `at` |
| `assistant.answer.requested` | `user` | The device where the person tapped **Let it answer** or **Answer this** (§9) | `request` (a new ULID), `post`, `space`, `how` (`"let"` or `"draft"`), `by`, `at` |
| `assistant.answer.draft` | `irori` | The assistant's computer (§9) | `request`, `post`, `space`, `text`, `device`, `at` |

No tool writes any of these, and no tool may ever write `assistant.chat.asked`: #987's grants are made from it (§5). Card 2 declares all eight kinds. The last two are used by assistants-talk's cards (§9).

### The fold

A new `crates/core/src/service/assistant_chat.rs` folds the rows into one conversation, the same on every device:

- **A turn** is its `asked` row plus at most one `said` row. The first `said` row for a turn wins, by store order, as `notification.posted`'s `older` does. Two `said` rows for one turn should only happen in the moments described in §7. This rule is a safety net, not how one assistant is kept.
- A `said` row that arrives before its `asked` row waits for it. It is never shown alone.
- **Order:** turns go by their `asked` row's store order, and each reply shows under its question. Lines go by their own store order. Across devices, `insert_batch` keeps causal order: a reply written after its question was inserted sorts after it.
- **`update` lines:** the first row per `key` wins, as notifications do. Every device's follower may write one, and duplicates fold to one.
- **Drafts** (`assistant.answer.draft`) are standalone lines ordered by their own store order, the first per `request` wins.
- **`posted`:** the first per turn wins, and it fills the turn's `posted` when the `said` row has none.
- **New chat:** the newest `cleared` row hides every turn whose `asked` row is older, every line and draft older than it, and every `said` row of a hidden turn. Rows are never deleted, so this only hides them (storage.md §4).
- **Imported rows** (§10) sort before every turn that isn't imported. They also count as older than any New chat, whatever their store order: an import that lands after a New chat stays hidden (as built, #1102).
- **What is shown:** the turns, lines and drafts after the newest clear. The list command pages them, 50 at a time, newest first.
- A row whose actor isn't its kind's is skipped.
- It loads at start and on resync, in the same block as notifications (`load_notifications`, `crates/core/src/service.rs`), and applies arriving rows in the kind loop.
- **Events:** a new `CoreEvent::AssistantChat` and topic, pumped to the window as `assistant:changed` (`crates/window/src/events.rs`), as `notifications:changed` is. It says only that the fold changed. It never carries the reply as it streams (§5).

### Scrubbed before signing

A row is permanent on every device, so a secret that gets into one stays there for good (v0.4 §5.1). Before appending, the writer (`Core::assistant_chat_write`, as built in #1102) runs the Core's scrubber (the `scrubber` field on `Services`, which capture uses and which holds the exact secret values `add_secret` gives it) on every text field: the question on the asking device, the reply and its `changed` lines on the assistant's computer, and follow-up lines and drafts on their writer. A match becomes `[redacted:<kind>]`, as capture does.

- The exact values are the writer's own secrets. The pattern rules apply everywhere, so another device's secret that matches no pattern is kept, as in capture. Accepted: the rows reach only the person's devices, and tools leave their text out (below).
- The reply streams live before it is scrubbed (§5). The stream is only a view and may briefly show text the row redacts. When the `said` row lands, its text replaces the streamed text on every device.

### Size and growth

- A question is at most the person's typed text plus 4,000 characters of screen. A reply is the model's text, cut to `MAX_PERSON_TEXT` (512 KiB), which keeps a row well inside the mesh's 8 MiB message.
- `looked` keeps the screen label only. Today's `ChatEntry.looked` holds the whole screen text the model read, once per reply. That would double the screen in every row, and it isn't needed: the question's row already has it.
- A heavy day of 100 questions is under 1 MB, kept for good on every device, the phone included. Capture already writes a row per agent turn, so this is well within precedent. If storage shows the screen is the bulk, cut it in the row.

### Who can read them

- Only the person's devices hold the private log, end to end over QUIC. No relay or server stores it (public-relays.md).
- **No tool shows these rows' text.** The `events` tool and `GET /v1/events` without the person's secret return rows whole today (§2). Card 2 makes both redact copies (`crates/tools/src/observe.rs`, `crates/api/src/redact.rs`) treat the `assistant.chat.` and `assistant.answer.` prefixes as they treat `message.sent`: every text field becomes `[left out]`. So the person's agents don't see the conversation, as today. The assistant gets its history only in its prompt.
- The rows can quote what the assistant read: private cards, and the assistant's computer's agents' last text and capture (`agents`, `timeline`, `why`). Today that text sits in one device's localStorage. Now it is on every one of the person's devices, which is what sharing the conversation means. It never leaves the person's devices, which keeps decision 53's "Mine" scope. **This amends decision 30** ("agents' last-message text stays on its own device") for the assistant's replies: the person's own question pulls the text, and it reaches only their devices.

## 5. Asking from any device

### On the device that asks

- `ask(text)` calls a new host command, `assistant_chat_ask(text, screen, again?) -> turn`, in place of `assistant_ask`. The host scrubs the text and appends `assistant.chat.asked` with this device's id, and the fold shows the question at once.
- The frontend sends no history. The assistant's computer builds it from rows.
- **The composer stays on** while questions wait or run. The person can send a second question while the first waits. Today's guard (`if (!body || run || s.reply) return`) goes. Stop belongs to a turn, not to the composer.
- **Under a question while it waits:**
  - On the assistant's computer itself, its reply streams under it, labelled with the device that asked when that isn't this one. It never streams into the composer's slot.
  - On another device: "Mac is answering…" under the oldest unanswered question while Mac is connected, and "Waiting for Mac" under the rest. When Mac isn't connected, the away line (§6). Card 7 streams the text here too.
- **Stop** appends `assistant.chat.stop`. Any device can stop any turn.
- **The phone pushes** its newest rows when it is put away (`sync_out`), if another of the person's devices is reachable then. If none is, the question leaves the next time the phone is opened.

### On the assistant's computer

**The hook.** A new `Core::set_chat_turn`, installed on desktops from `app/src/shell/mod.rs` as `set_namer` is. Core keeps the queue, the checks and the rows. The window does the run. The types live in Core (`crates/core/src/service/assistant_chat.rs`), since Core can't name window types:

```rust
pub struct ChatEvent { pub kind: String, pub text: String }          // serde only
pub struct ChatJob { pub turn: String, pub prompt: String, pub screen: String,
                     pub grants: Vec<tools::Grant>, pub cancel: Arc<AtomicBool>,
                     pub kind: ChatKind }                    // Chat; #1064 adds OwnerOk
pub struct ChatDone { pub text: String, pub error: String, pub changed: Vec<String>,
                      pub posted: Option<Posted>, pub looked: Option<String> }
pub type ChatTurn = Arc<dyn Fn(&ChatJob, &dyn Fn(ChatEvent) -> bool) -> ChatDone + Send + Sync>;
```

The window's closure:

- sets the screen (`set_assistant_screen(job.screen)`), begins the grants, and runs `Job::assistant` on its existing `assistant` `Runner` with today's flags, tools and secret;
- blocks until the run's `done` or `error` (`Runner::start` returns at once);
- maps each `AssistantEvent` to a `ChatEvent` for Core, and forwards it to any open `assistant_chat_watch(turn, on: Channel<AssistantEvent>)` subscriber in its own window, as `assistant_ask`'s Channel does today. A watch may be opened for any unanswered turn before it starts, and is kept until that turn ends. On opening, it first gets the running turn's text so far as one delta;
- watches `job.cancel` and calls `self.assistant.stop(id)` at once when it trips, so a Stop ends the child even while it waits on a tool call;
- ends the grants and returns their `changed` lines;
- sets `looked` to `screen_label(job.screen)` only when a `tool` event starting with `StatusHead` was seen, as `assistant.ts` does today;
- posts a thread by itself when the reply calls for it (below).

**The queue.** Core keeps a `BTreeMap` of pending turns keyed by the `asked` row's store order, and a set of turns queued or running. A turn already in the set is never queued again. One worker thread always takes the smallest key, and survives a panic, like the namer's. Questions enter it two ways:

- **Live**, from the kind loop as each `asked` row arrives, local ones included. Not from the 5 s sweep.
- **By a catch-up scan** of the fold, run when the hook is installed, when this device becomes the assistant's computer (the listener in §3), and on every resync of the kind loop's subscription (rows past 256 behind are otherwise lost to the queue). As a backstop, the 5 s sweep also runs it when it finds an unanswered question that isn't queued.

**When the worker takes a question,** it first waits until this device is caught up with every one of the person's devices it is connected to (§3). That way it has seen any newer `assistant.home`, `said`, `stop` or `cleared` row that came in the same sync. Then it runs these checks against the fold at that moment. The first one that fails drops the question:

| Check | If it fails |
|---|---|
| The question isn't imported (§10) | Nothing, ever |
| This device is the assistant's computer (`is_assistant_home`) | Nothing. Another computer answers. |
| The turn has no `said` row | Nothing |
| It is after the newest New chat | Nothing |
| The turn has no `stop` row | Write a `said` row with `error: "Stopped"` |
| If the catch-up scan found it (not live): it is one of the newest five unanswered questions it found | Write a `said` row with `skipped: true` (§6) |

The five cap is only for the scan, where the computer really was away or behind. Questions queued live are never skipped, however many wait behind a slow turn.

**One at a time, oldest first.** A question sent from the phone while the Mac's own question is running waits for it, and its turn sees that answer. This also keeps `Grants`' "one set per Core" rule true, since no two chat runs overlap. Only the chat uses the window's `assistant` `Runner`, so its `MAX_RUNS` is never reached. Any other turn on the assistant's computer that holds the assistant secret also goes through this worker, never beside it. Today that is only #1064's owner's-OK turn (§9): `ChatJob.kind` picks its prompt and its `--allowedTools`, and it holds no grants. assistants-talk's tool-less turn holds no secret and no grants, so it keeps its own runner.

**Stopping a running turn.** When the kind loop folds a `stop` row for the running turn, or a `cleared` row newer than its question, Core trips that job's `cancel`. The `said` row says "Stopped". A turn hidden by New chat stays hidden.

**The prompt.** It is built in Core from rows, by a port of today's `historyFor` and `turnText` (`assistant.ts`) and `render_prompt` (`assistant.rs`):

- The turns before this one, after the newest New chat. Runs of `update` lines are grouped into one assistant turn, as `historyFor` does. A posted draft gets its "(This draft was posted as thread …)" note.
- Left out, as notes are today: turns with no `said` row yet, turns whose `said` row is `skipped` or has an `error`, and imported turns with no reply. The last 12 turns that go in are used, as `HISTORY_TURNS` says today.
- Drafts, and anything else carrying another person's words, never go in as the assistant's words (assistants-talk §8).
- **A late question** is judged by time only. Take when the row reached this computer: for a row seen live, the moment the kind loop got it; for a row the catch-up scan found, now. If that is more than five minutes after the row's `at`, corrected by the asking device's clock skew where the mesh knows it (`PeerInfo.skew`), the question is late. So a question asked ten seconds before the computer finished starting isn't late, a phone whose clock runs a few minutes off isn't late, and a question queued live is never late, however long it waits behind other turns. A late question is marked: "(Asked at 14:02 on the phone; it reached your computer at 17:40. Changes asked for this long ago aren't made: if the person still wants one, ask them to say it again.)" So an old question gets today's answer and says so, without claiming which device was off.

**Grants.** The question's row is the person's typed message.

- `grants_for(text, earlier)` takes its `text`. `earlier` is the `text` of the `asked` rows that come before it in the fold's order, after the newest New chat, at most 50, leaving out imported rows. Rows after it never count, whether or not they are still queued.
- **A late question gets no grants.** A write asked for hours ago may not be wanted now. The age note tells the model so. A question queued live keeps its grants, however long it waits.
- The parser is unchanged: it still reads only the person's words, and a grant is still spent once.

**The screen.** The closure sets `job.screen`, the snapshot of the device that asked, just before the run, under the worker, so the `screen` tool sees it.

**`timeline` and `why`** read the assistant's computer's capture. That is the one assistant's view, the same whichever device asked. Accepted.

**Posting a thread by itself.** `splitDraft` and `postsByItself` (`assistant.ts`) are ported to Rust in the window. When a finished reply posts by itself, the closure first checks again that this device is the assistant's computer and that the fold has no `said` or `posted` row for this turn, then calls `Window::start_thread_with(…, assistant: true)` and returns `posted`. It then appears on every device in the `said` row. Only the assistant's computer ever posts a draft by itself, so every device seeing the same reply posts nothing. The frontend's `autoPost` goes away.

**The reply row.** When the run ends, Core scrubs the text and `changed` lines, and appends `assistant.chat.said`. The stream is a view: the row is the reply.

**When the run fails.** `NO_CLAUDE`, `TOO_LONG`, a signed-out Claude Code, a usage limit, or any other error goes in the `said` row's `error`, in today's words plus the computer's name: "Claude Code on Mac couldn't answer: …". For a signed-out Claude Code: "… it isn't signed in. Run `claude` in a terminal on Mac to sign in." Every device shows it, with **Ask again** (§6).

### Markers

- **`<<thread>>` drafts.** The reply row carries the reply text, markers included, and every device renders the draft as today. **Post thread** (a check-first draft) works on any device. It starts the thread from that device, then appends `assistant.chat.posted`, and every device hides the button once a `posted` exists. Two taps on two devices within the second it takes rows to cross would start two threads. Accepted: it needs two taps at once.
- **`<<go …>>`** goes by itself only once, and only for a `said` row that arrives through `assistant:changed` during this page's life, for a turn this page asked in this run of the app (kept in memory), when the reply is done, and while the panel is open. It never goes from rows loaded at start, from a resync or from paging, and never for a reply that took more than ten minutes to come. Every other device, and every other case, shows the go button, which the person may tap.

### Follow-up lines

- The follower (`assistant-follow.ts`) follows each thread a turn posted, read from the fold's `posted`. It runs on every device, the phone included, so the person can answer a card's question from wherever they are.
- When it has a line to say, it appends `assistant.chat.update` with a key built from the thread id and the fact's key: `cant`, `perm:<card>:<blocker>`, `ready:<card>:<blocker>[:round]` or `ask:<card>:<blocker>[:round]`. The first row per key wins, so every device saying it at once still shows one line.
- What it has said comes from the fold's `update` keys. Its silent keys (`taken`, `made:`, `start:`, `done:`, `closed`) and its `done` flag stay in a small per-device store, renamed `irori.assistant.follow.silent`: they are never shown, and only save reads. A thread with no local entry starts from what the fold has said, not from nothing.
- #1000's cut is unchanged: only lines that wait on the person are written.

### The dot

- A reply lights the dot only on the device that asked, under the same "arrived live, asked in this run" rule as `<<go>>`, when its panel is closed.
- A follow-up line past this device's "seen up to" lights it on every device, whether it arrived live or at load, since it waits on the person wherever they are.
- Each device keeps its own "seen up to" in localStorage. It is a per-device convenience, not part of the conversation. Only replies use the live-only rule.

## 6. When the computer is away

The chat header says where the assistant runs: "on Mac", with the device dot from Settings › Devices.

| State (from `assistant_home_status`) | What the chat shows | Composer |
|---|---|---|
| Connected | Nothing extra. Under questions from another device: "Mac is answering…" or "Waiting for Mac" (§5) | On |
| Recent or offline | Above the composer: "Your assistant runs on Mac, which isn't connected (seen 2 hours ago). Questions wait for it." With **Use PC instead** when another computer that says `assistantChat` is connected now | On: the question waits |
| Connected, but its newest `spawn.host` lacks `assistantTurn` | "Your assistant runs on Mac, which can't find Claude Code. Install it there, or **Use PC instead**." (the tap only when another computer can) | On: the question waits |
| Connected, but its newest `spawn.host` lacks `assistantChat` (an older Irori) | "Mac needs the latest Irori to answer from your other devices." With **Use PC instead** when another computer can | On: the question waits |
| No assistant's computer, `setting_up` | "Setting up on Mac…" (the next sweep claims) | On: the question waits |
| No assistant's computer, `needs_update` | "Mac needs the latest Irori to answer from your other devices." | Off |
| No assistant's computer, `no_claude` | "Your assistant needs a computer with Claude Code. None of your devices has it yet." | Off |

- **No second assistant starts.** The question row sits on the asking device, and on any device it syncs to, until the assistant's computer gets it.
- **When the computer is back,** its waiting questions reach it two ways: a restart runs the catch-up scan when the hook is installed, and a wake from sleep delivers the rows through the kind loop, or runs the scan on a resync. The newest five the scan found are answered in order, each with its age note and no grants (§5). Older ones get a `said` row with `skipped: true`: "Mac didn't get to this. **Ask again**".
  - Why five: the questions are the person's own, so they should be answered, not dropped for being late, and the age note keeps a late answer honest. The cap keeps a day of questions from a phone from turning into a burst of turns.
- **Ask again** shows on any turn that is `skipped`, has an `error`, or says "Stopped". It calls `assistant_chat_ask` with the same text and `again` set. A failure during catch-up (a usage limit, a dropped network) is then one tap per question, not retyping.
- **Use PC instead** writes a `how: "person"` row naming the PC. It is the person's choice, made with one tap, never by itself. The PC's catch-up scan then answers what waited. If the Mac was only unreachable, it sees the newer row before its next turn (§5's caught-up wait) and stops answering (§7).
- **On the phone** the chat now works. There is no `NO_CLAUDE` on a phone any more: its questions go to the assistant's computer like anyone else's.
- **A known gap:** the Windows headless node (`irori serve --headless`) installs no hooks and publishes no `spawn.host`. If the assistant's computer is running that way, presence says connected, and the chat says "Mac is answering…" while nothing answers until the app's window runs there. Accepted: it is a fallback, rarely used. Card 7's stream would tell the asking device that no turn started.

Settings › Agents (`frontend/src/components/settings/agents-pane.tsx`), beside the assistant's model, gains **Assistant's computer**: a picker of the person's computers that say `assistantTurn` and `assistantChat`, each with its dot and "seen …". Picking one writes a `how: "person"` row. With no row yet, it shows the computer that will claim itself. It only changes the choice. The assistant works without anyone touching it (decision 52).

## 7. Two desktops at once

Every rule above reads replicated rows, not each device's view of who is online. And a computer starts a turn only once it is caught up with every own device it is connected to. That leaves a few short moments when two desktops can both run a turn:

| When | What happens | How it ends |
|---|---|---|
| **First claim, in a split or at the same moment.** Two desktops that can't see each other both claim. So can two connected desktops that update at about the same time, before their new `spawn.host` rows cross. | Each answers the questions that reach it. | When they meet, the first claim wins on both. A question both answered has two `said` rows, and the first wins on every device. Both may have posted a thread for it (below). Accepted: it happens once, at the first claim. |
| **The person moves it while the old computer is split off.** The phone writes "PC", and the Mac doesn't hear. | The Mac gets no new questions while it is split off: it is the rows that reach it. | Questions asked before the move may reach the Mac before the move row does, since rows from different writers come in no set order. The worker waits until the Mac is caught up before starting each turn (§5), and by then the move row is in. So the Mac stops. Only a turn it had already started when the split happened finishes, and if the PC answered it too, the first `said` row wins. |
| **A newly paired desktop, or one back from a long absence.** | Its rows come in no set order, so it may see questions before their answers or the existing claim. | It doesn't claim, and its worker starts nothing, until it is caught up with the device it is syncing from. By then it has the claim and the answers. |
| **An older build** on one of the person's devices. | It keeps its own local chat, as today, until it updates. It never claims after card 3 and never answers others' questions. A build with card 1 but not card 3 may hold the first claim: the other devices then show "Mac needs the latest Irori" with **Use PC instead**. | Updating it. Accepted. |
| **Asymmetric links**: the Mac hears the PC, but not the other way. | Nothing. Neither link decides who is the assistant's computer. | n/a |

**What first-`said`-wins doesn't undo.** It hides the second reply's text, but not its side effects. In the moments above, both computers may post a thread for the same question, and a granted write (closing a card, ticking a step) may run twice, which is harmless since it is the same write. Before posting a thread by itself, the closure checks again that it is the assistant's computer and that no `said` or `posted` row exists for the turn (§5). That leaves only two computers running the same turn at the very same moment during a split. Accepted, for the same reason as the first claim.

## 8. A session, or a turn per question

The person asked for "one Claude that is started for that". There are two ways to give that.

| | One turn per question over shared rows (today's turn, run on one computer) | A held session on the assistant's computer |
|---|---|---|
| Shared context | Yes: every turn gets the same history from the rows, whichever device asked | Yes, and it also remembers its own tool results and reasoning between turns |
| Speed and cost | A fresh `claude -p` each time: 3.7 to 5.0 s and $0.0004 to $0.0005 per question on Haiku, growing with the history (call-agents §7, measured) | After the first, 1.1 to 1.9 s and $0.00013 to $0.00026 on Haiku, with no tools. Not measured with tools, on Sonnet low, or over an hour |
| When the computer restarts, or the person picks another | Nothing lost | The session is gone. It has to be seeded again from the rows |
| What other devices see | The rows | The rows. The session's memory of tool output is invisible to them |
| New code | A worker and a prompt from rows | All of that, plus a process kept open with `--input-format stream-json`, per-message grants and screen, a size limit and a reseed |

**The decision: build the turn per question first (cards 3 and 4), then the held session (card 8).**

- The rows are what make the context shared. They are needed either way, because a held session has to be seeded from them whenever it starts again. So the first step is the same in both.
- With the rows and one computer, there already is one assistant: one place, one conversation, one history.
- Card 8 then keeps a Claude process running for the chat on the assistant's computer, which is "one Claude that is started" in the literal sense:
  - It is started with the first question after the app starts, not before, so an unused chat costs nothing.
  - Each question writes one user message. The screen and grants for that turn are set first, as in §5. Any rows that came in since its last message (follow-up lines, questions answered while it was down) go in that message as "since we last spoke" lines.
  - It is seeded again from the rows (§5's prompt) when the process dies, after New chat, when the assistant's computer changes, or past about 60k tokens of context, as call-agents §7 does.
  - It stays one turn at a time, through the same worker, so the grants rule holds.
- **Card 8 measures it first,** with tools on the chat's model, against card 3's turns. If the numbers say keep it, it is built. If they say not to, the card posts the numbers on #1093 and asks the person. It doesn't drop the session by itself, since the person asked for it in so many words.

## 9. Answering other people, and the huddle

### Assistants talk (docs/design/assistants-talk.md)

Its cards 3 to 8 (#1059 to #1064) aren't built yet. They change as follows. assistants-talk carries a note at each changed section, and a note under its §12 that rewrites the cards.

- **The gate for an automatic answer** (its §5, "This device is the assistant's computer") is `is_assistant_home`. After card 1 here, that reads the recorded computer, so a short link drop no longer moves who answers. Its catch-up scan ("whenever this device becomes the assistant's computer") hangs off the same listener (§3). The repeated answer id stays as the safety net for §7's moments.
- **Let it answer and Answer this go to the assistant's computer.** A tap on any device appends `assistant.answer.requested` (§4) through a host command. The assistant's computer serves it, only where `is_assistant_home` holds:
  - It checks the request against the post itself: the post's own space and channel, its author, the answer id and the hop. Never the row's `space`.
  - `let` runs #1060's tool-less turn on its own runner (no tools, no secret, no grants). `draft` runs the owner's-OK turn through the one chat worker (§5).
  - The tap skips its §5's waiting-line check, rate limit, hour and receive setting, as designed there, but not the home check.
  - Serving writes a record that names the request: the draft row, or an `update` line saying what happened (answered, held, failed). A request with a record is never served again, after a restart or a move.
  - A request the catch-up scan finds more than an hour old gets an `update` line instead: "Mac was away when you tapped this. Tap again if it still needs an answer." A tap made days ago never turns into a public post without a fresh one.
  - A phone gets both buttons, instead of "Answer from your computer", because it no longer runs anything itself.
- **The owner's-OK draft** (its §7) is an `assistant.answer.draft` row, its own kind, a standalone line in the chat. Every device shows it, with **Send to Eve's assistant** and **Don't send**. Send works from any device and posts as the assistant's card, as designed there, and the answer id still refuses a second Send. The draft is private text and stays in private rows. It never goes into the chat's prompt (§5).
- **The owner's-OK turn holds no grants.** It runs through the one worker as `ChatKind::OwnerOk`, so it starts only after the previous chat run's `Grants::end` has emptied the open set, and it never calls `Grants::begin`. Its `--allowedTools` names only `ASSISTANT_TOOLS`, never the write tools. Its card tests that its run config lists none of `ASSISTANT_WRITE_TOOLS`, that it never begins grants, that the open set is empty when it starts, and so that an `assistant_close` from it gets a 403.
- **`held` lines** (its §8) become `assistant.chat.update` rows, keyed by the question's post id, so every device shows them, not only the assistant's computer.
- **`from` lines** (`answer`, `needs`, `asked`, its §8) stay per device and are never rows. Each device rebuilds them from the replicated assistants channel (`assistant-talk-follow.ts`, on load and on `talk:changed`) and merges them into the shown list by time. They never go into the prompt as the assistant's words.
- **Stop here** on the assistant's computer turns automatic answers into "Ask me", as its §5 says for Pause all.
- **The ask-draft marker** of its card 7 (#1063) is taught in the Core-built prompt (card 3 here), not in `render_prompt`.
- **Order:** #1060 and #1061 after card 1 here; #1063 after card 3; #1062 and #1064 after card 4.

### The huddle (docs/design/call-agents.md)

- **Which device hosts it** (its §7) is the recorded assistant's computer, nothing else.
- Its rule "An `on` whose device has been away in presence for over 60 s counts as off … and the next device takes over" is dropped. When the assistant's computer is away, the roster shows "not connected: Mac isn't connected (seen …)", the same plain words as the chat. Its build card 7a (call-agents §13) is changed to match.
- `call.assist` stays, so the call knows whether the assistant is connected.
- Stop here or Stop everywhere on the assistant's computer keeps the assistant out of calls (§3).
- **Divide with my assistant** (its §9) shows on the phone too: its chat turn runs on the assistant's computer. The huddle context goes in as an optional `callId` on `assistant_chat_ask` (and the `asked` row), not on `assistant_ask`, which goes away.

### What stays where it is

Irori's own side turns aren't the assistant's conversation: thread naming, card weighing, draft rewriting and the merge desk's reviewer pick (merge-desk §12.5, card #1081). Each is a tool-less, single-shot turn with no history and no assistant secret, run by whichever device the work lands on. They keep running where they run today.

## 10. Moving today's history

Today each device has its own history in localStorage. Putting them all into one conversation would interleave unrelated chats in an order nobody had them in (the entries have no times), and feed the mix to the model. So:

- **The assistant's computer brings its own history in, once.** The frontend runs the import when `assistant_home_status` says this device is the assistant's computer: at load, and again whenever the status changes, since the first claim can land a few seconds after the window loads. It reads `irori.assistant.history` through `parseHistory` and sends it to a new host command, `assistant_chat_import(entries)`.
- **The host decides, not the frontend.** It refuses unless this device is the assistant's computer and the fold has no imported row yet. It doesn't matter if questions were asked since the update: imported rows sort before every other turn (§4).
- **How entries map to rows**, all written in one batch with `imported: true`, scrubbed like any row:
  - A person entry becomes an `asked` row.
  - The assistant entries after it, up to the next person entry, become its `said` row: the reply text, its `posted`, its `changed` notes, and an error note as `error`. `looked` keeps only the label.
  - A person entry with no reply gets a `said` row with `error: "(no reply)"`, so nothing imported is ever left unanswered.
  - `update` entries become `update` rows, keyed by thread and text, with their `act`.
  - Leading assistant entries (left when the 50-line cap cut the person's question) are dropped. So are old `to` entries.
- **Imported rows never run.** The worker's first check drops them (§5), the catch-up scan skips them, and they are left out of `grants_for`'s `earlier`.
- After a successful import, the frontend sets a local flag, `irori.assistant.imported`, and stops trying.
- **Every other device keeps its old history, read-only**, above a divider: "Earlier on this phone, before your chats were shared". It isn't sent to the model, isn't replicated, has no buttons that act, and goes when the person taps New chat.
- **`irori.assistant.follow`** is replaced by the fold's `update` keys, plus the small silent store (§5). A thread that still waits on the person may get its line said once more after the update. Accepted: it still waits on them.
- **Card 6 lands together with card 4,** in the same change. Otherwise a release with card 4 alone would hide the old history until card 6 comes.

## 11. Versions

Nothing needs a gate:

- Every new kind is private. Older builds store and pass on kinds they don't know (no-server.md: "The private log stores unknown kinds and folds them after an update").
- `assistantChat` is an optional field on `spawn.host`. Older readers ignore it. It must never ship in a build without the worker (card 3).
- An older build keeps its own local chat until it updates (§7).
- An older build's `events` tool and `/v1/events` don't know these kinds, so they show their rows whole to that device's own agents (found in #1102's review). The rows still reach only the person's devices. Cards 3 and 4, which first write them, can weigh whether that wants a note or a gate.

## 12. Build plan

Eight cards, each with Files, Steps and Done when. Each is normal weight unless marked, and links here. Every card:

- tests from the repo's patterns, and `sh scripts/check.sh`;
- scratch builds per CLAUDE.md: a copied `IRORI_HOME`, an empty `CLAUDE_CONFIG_DIR`, `IRORI_DEV_PORT_OFFSET`, stopped by PID;
- every new Tauri command added to `collect_commands!` in `crates/window/src/commands.rs`, `cargo xtask bindings` run, and the command wrapped in `frontend/src/lib/host.ts`.

**The order:** 1 and 2 side by side. 3 after both. 4 after 3, with 6 in the same change. 5 and 7 after 4, in any order. 8 after 4. assistants-talk's #1060 and #1061 after card 1, #1063 after card 3, and #1062 and #1064 after card 4 (§9).

**1. The recorded assistant's computer.**
Files: `crates/store/src/kinds.rs` and `crates/store/tests/it/golden.rs` (`assistant.home`); new `crates/core/src/service/assistant_home.rs` (the row, the fold and its four rules, `assistant_home()`, the change listener); `crates/mesh/src/node.rs` (`Node::caught_up`); `crates/core/src/service/spawn_capable.rs` (`is_assistant_home` reads the fold); `crates/core/src/service/spawn.rs` (the claim in `sweep_spawns`, before `mint_assistant_cards`); `crates/core/src/service/spawn_remote.rs` (a read-only `assistant_chat: bool` on `SpawnHost`, `#[serde(default)]`, written by nothing until card 3; fix the comments that say "signed in" to say "the `claude` command is found"); `crates/core/src/service.rs` (refold on `assistant.home` and `person.device.revoked` rows and on resync; the `pub` accessor; `Core::assistant_home_status`); `crates/core/src/events.rs` and `crates/window/src/events.rs` (`CoreEvent::AssistantHome` and the `assistant:home` pump); `crates/window/src/commands.rs` (`assistant_home_status`, `assistant_home_set`).
Steps: the kind and payload; the fold; `caught_up`; the claim; `is_assistant_home` on the fold; the listener; the status and set commands; tests.
Done when tests show:
- Two Cores that claim during a split agree on the first claim after they meet.
- A person's row beats an earlier automatic claim, and the newest person's row wins, even when its `at` is smaller.
- With the assistant's computer's Core stopped, another able Core never claims, and `is_assistant_home` stays false there.
- An automatic claim for the Mac, then a person's row for the PC, then the PC revoked: no assistant's computer, then a connected PC2 claims, and the Mac's old claim doesn't come back. Revoking changes the cached answer with no other row arriving.
- A computer whose `spawn.host` drops `assistantTurn`, for a moment or for good, stays the assistant's computer, no other Core claims, and the status says Claude Code is missing there.
- A Core that is connected to another of the person's devices but not caught up with it doesn't claim.
- `assistant_home_status` reports `setting_up`, `needs_update` and `no_claude` in their cases, with `assistantChat` set directly in hand-written `spawn.host` rows.
- A person's row naming a revoked device, sorted after the revocation, is ignored.
- A change of the folded answer publishes `assistant:home`.
- `mint_assistant_cards` mints on the recorded computer only (extend `crates/core/tests/it/assistant_card.rs`).

**2. The conversation rows and their fold.**
Files: `crates/store/src/kinds.rs` and `golden.rs` (the eight kinds of §4); new `crates/core/src/service/assistant_chat.rs` (payloads, the fold, the scrub helper, the list); `crates/core/src/service.rs` (load at start and on resync, the kind-loop branch); `crates/core/src/events.rs` (`CoreEvent::AssistantChat` and its topic); `crates/window/src/events.rs` (the `assistant:changed` pump); `crates/window/src/commands.rs` (`assistant_chat_list(before, limit)`); `crates/window/src/storage.rs` (`log_part` names the `assistant.` prefix); `crates/tools/src/observe.rs` and `crates/api/src/redact.rs` (leave out the text of `assistant.chat.` and `assistant.answer.` rows).
Steps: the kinds; the fold (turns, first `said` wins, `said` waits for `asked`, `update` first per key, drafts, `posted`, New chat, imported first); load, apply and the event; the list command; the scrub helper every writer uses; the redaction; tests.
Done when tests show:
- Rows written on two Cores fold to the same list, in the same order, on both.
- A `said` row that arrives before its `asked` row shows once the question arrives, and not before.
- Two `said` rows for one turn show one, the same on both Cores. Two `update` rows with one key show one.
- A draft row shows on both Cores with no `asked` row.
- New chat hides older turns, their replies, older lines and drafts, and a question asked after it shows.
- Imported turns sort before a turn asked earlier that isn't imported.
- A reply containing an Anthropic key is stored as `[redacted:anthropic]`.
- A row with the wrong actor for its kind (an `asked` row with actor `irori`) is skipped.
- The `events` tool, and `GET /v1/events` without the person's secret, show these rows' text as `[left out]` (beside `leaves_out_text_and_fences`).

**3. The assistant's computer answers questions from rows.**
Files: `crates/core/src/service/assistant_chat.rs` (`ChatEvent`, `ChatJob`, `ChatDone`, `ChatTurn`; the queue and its two ways in; the caught-up wait; the checks of §5; the catch-up scan and its triggers; the prompt from rows, ported from `historyFor`, `turnText` and `render_prompt`; grants from rows, with none for a late question; cancel on `stop` and `cleared`; the `said` row; the error words with the computer's name and the signed-out hint); `crates/core/src/service.rs` (`set_chat_turn`); `crates/core/src/service/spawn_remote.rs` (`assistantChat` on `SpawnHost`, in `HostShown` and `publish_host`'s tuple, true only with the hook installed); `crates/core/src/service/assistant_home.rs` (the claim, and the ranking it uses, also need `assistantChat`); `crates/window/src/assistant.rs` (the closure: screen, grants, `Job::assistant`, blocking until done, cancel through `Runner::stop`, `looked`, forwarding to watchers, porting `splitDraft` and `postsByItself`, posting with `Window::start_thread_with(…, assistant: true)` after the re-check); `app/src/shell/mod.rs` (install it on desktops); `crates/window/src/commands.rs` (`assistant_chat_ask(text, screen, again?)`, `assistant_chat_stop(turn)`, `assistant_chat_watch(turn, on)`).
Steps: the ask and stop commands; the hook and a fake turn; the queue, its dedupe and the caught-up wait; the checks; the scan and its triggers; the prompt; grants and screen; cancel; the real closure and the watch; posting by itself; `assistantChat`; tests.
Done when, with a fake turn returning fixed text, tests show:
- A question appended on one Core is answered once, by the recorded computer only, and the reply is under it on both Cores.
- Two questions from two Cores at once run one after the other, oldest first, and the second turn's prompt holds the first's answer. A question with an older store order that arrives while another is queued runs first.
- A question delivered both live and by the catch-up scan runs once.
- A question that arrived before the hook was installed is answered by the scan. So is one inside a burst that overflows the subscription (a resync). So is one that waited while another computer was recorded, after a person's row moves the assistant here.
- Six questions sent while the computer is connected are all answered. Twenty found by the scan give five answers and fifteen `skipped`.
- A question an hour old gets the age note and no grants: a "close #12" asked an hour before it arrived closes nothing. A question found by the catch-up scan ten seconds after it was asked isn't late and keeps its grants. A question from a device whose clock is four minutes behind, with that skew known, isn't late.
- `grants_for`'s `earlier` holds only the rows before the question: "close it" (with #5 named before it), then a question naming #9 sent while the first waits, gives the first turn Close #5 only. Imported rows are never in it.
- A Stop row before the turn starts gives "Stopped". A Stop row during a run that is waiting on a tool call ends the child within a second, and the `said` row says "Stopped". A New chat during a run stops it too.
- An imported question with no `said` row is never run.
- A Core that receives a question and a newer `assistant.home` row naming another device in one sync runs no turn. A fresh Core that gets a phone's questions before the old computer's `said` and `assistant.home` rows neither claims nor runs them.
- A reply whose draft posts by itself starts exactly one thread, from the recorded computer, and its `said` row carries `posted`. A second Core that gets the turn after the first Core's `said` or `posted` row has arrived posts nothing.
- A Core without the hook publishes no `assistantChat` and never claims. A connected, higher-ranked Core whose `spawn.host` has `assistantTurn` but no `assistantChat` doesn't stop an able Core from claiming.
- A "close #12" queued live behind a turn that runs three minutes still closes #12.
- A real run on scratch builds: the generated `assistantChatAsk` binding, called from the second instance's devtools console, is answered by the first, and `assistantChatList` shows the reply on both (the chat UI comes in card 4).

**4. The chat reads and writes rows.**
Files: `frontend/src/lib/assistant.ts` (entries from `assistant_chat_list` and `assistant:changed`; `ask` through `assistant_chat_ask`; the composer stays on; stop on a turn; New chat appends `cleared`; Post thread appends `posted`; `<<go>>` and the dot only for live replies to this page's questions; drop `autoPost`, `saveHistory` and the `history` argument), `frontend/src/lib/assistant-follow.ts` (follow `posted` from the fold; write `update` rows; what it said from the fold; the silent store), `frontend/src/components/assistant/assistant-chat.tsx` ("Mac is answering…" and "Waiting for Mac"; the streamed reply under its question on the assistant's computer, through an `assistant_chat_watch` opened for each unanswered turn it shows; Stop on a turn), `crates/window/src/commands.rs` (`assistant_chat_posted`, `assistant_chat_clear`, `assistant_chat_update`). Remove `assistant_ask`, `assistant_stop`, `AssistantService.Ask`/`Stop` and the window's history rendering once the chat reads rows.
Steps: read the fold; ask, stop and clear; the composer; drafts and Post thread; go and the dot; the follower on rows; the streamed reply; remove the old path; tests.
Done when:
- Tests show: a reply's `<<go>>` acts only where it was asked, only live, and never from rows loaded at start or after a resync; Post thread on one device hides the button on the other; two followers saying one line give one line; a reply lights the dot only on the asking device, and a follow-up line on both, including one written while this device was closed, when it opens; a second question can be sent while the first waits.
- A two-instance scratch run shows the same conversation on both. A question typed on the instance that isn't the assistant's computer is answered by the one that is, and both show it. The assistant's computer's composer stays usable while that turn streams.
- Screenshots at desktop and phone width.

**5. Away, and choosing the computer.**
Files: `frontend/src/components/assistant/assistant-chat.tsx` (the header's "on Mac" and dot; each line of §6; **Use PC instead**; the composer rules; **Ask again** on skipped, error and stopped turns), `frontend/src/lib/assistant.ts` (`assistant_home_status`, read again on `devices:changed` and every 5 s while the panel is open, as `useDevices` does), `frontend/src/components/settings/agents-pane.tsx` (**Assistant's computer**).
Steps: the header; each state of §6's table; Use PC instead; Settings; Ask again; the phone; tests.
Done when:
- Tests show each state of §6's table renders its words and composer rule. Use PC instead writes a person's row. Ask again calls `assistant_chat_ask` with `again`.
- A two-instance scratch run with the assistant's computer stopped shows the away line on the other and a question waiting. Started again, the answer arrives with the age note. Stopped again, **Use PC instead** on the other instance has the waiting question answered there.
- Screenshots of the away line at desktop and phone width, and of Settings.

**6. Moving today's history.** Lands in the same change as card 4.
Files: `frontend/src/lib/assistant.ts` (the import when the status says this device is the assistant's computer, at load and on `assistant:home` from card 1; the read-only "Earlier on this …" block elsewhere; the `irori.assistant.imported` flag), `crates/window/src/commands.rs` (`assistant_chat_import(entries)`), `crates/core/src/service/assistant_chat.rs` (the host's refusals; the mapping of §10; one batch, scrubbed, `imported: true`), `frontend/src/components/assistant/assistant-chat.tsx` (the divider and block).
Steps: the import command and its refusals; the mapping; when the frontend calls it; the read-only block; tests.
Done when tests show: the assistant's computer imports its history once, in order, and a second call is refused; a claim that lands after the first load still imports; a device that isn't the assistant's computer imports nothing and shows its history read-only, in no prompt; a history ending in an unanswered "close #12" imports with a "(no reply)" `said` row, runs no turn and closes nothing; an imported `to` entry never runs; New chat removes the read-only block.

**7. The reply streams to other devices.**
Files: `crates/mesh/src/node.rs` (a new `Options` field, for example `assistant_turns: Option<Arc<dyn Fn(&str) -> Option<std::sync::mpsc::Receiver<Vec<u8>>> + Send + Sync>>`, returning a running turn's framed events); `crates/mesh/src/files.rs` (an `assistant/<turn>` arm in `serve_stream`, served only to the person's certified devices, as `blob/` is); a new `crates/mesh/src/assistant_stream.rs` (client and server, framed like inline-terminals §6.4: `u8 type ‖ u32 len ‖ body`); `crates/core/src/meshnode.rs` (a `Parts` field threaded into `Options` where `Mesh::new` builds the node, filled with a closure over a `OnceLock` set once `Services` exists, as the `node_cell` pattern in `service.rs` does); `crates/core/src/service.rs` (set that cell; a `pub fn assistant_stream(&self, device, turn)` for the window, opening the stream as `fetch_file` does); `crates/mesh/tests/it/common/mod.rs` and `crates/pair/tests/it/common/mod.rs` (their `Options` literals gain the field); `crates/core/src/service/assistant_chat.rs` (a broadcast of the running turn's events); `crates/window/src/assistant.rs` and `assistant-chat.tsx` (open the stream on the asking device when the assistant's computer is connected; show deltas and tool lines).
Steps: the `Options` seam; the server arm, bridging the sync receiver into the stream with a thread feeding a tokio channel; the client; the fallback; tests.
Done when tests show: a second Core connected to the assistant's computer gets the deltas of a running turn in order; a dropped stream falls back to waiting for the `said` row with no error shown; after the end the shown text is the row's text, including a stream that carried a key the row has as `[redacted:anthropic]`. A two-instance scratch run shows the reply streaming on the device that asked.

**8. A held session on the assistant's computer.** *(heavy: a measurement and a judgment call)*
Files: `crates/window/src/assistant.rs` (a held run beside `Job`: `--input-format stream-json`, stdin kept open, the chat's tools and secret; one message per question; reseed), `crates/core/src/service/assistant_chat.rs` (the "since we last spoke" lines; when to reseed).
Steps: measure first, with tools on the chat's model, a held session against card 3's turns over 20 questions; if it is faster and answers no worse, build it; per-message screen and grants; reseed on process death, New chat, a change of computer and about 60k tokens; tests.
Done when: the measurement is written on the card and in this doc's §8. If kept, tests show a second question answered by the same process, grants that `begin` and `end` per message, and a reseed after New chat that holds no turn from before it, and a scratch run answers ten questions from two devices in one process. If the numbers say not to keep it, the card posts them on #1093 and raises the choice for the person. It doesn't close by itself.

## 13. Not in v1

- **Moving the assistant by itself** when its computer has been away for a long time. That is a second assistant starting on a guess, which is what D3 rules out. If the person wants it, a later tune could offer the move as a one-tap notice after some hours, never do it.
- **Several conversations** (one per space or topic). There is one conversation, as today.
- **Reading the conversation from tools.** No tool shows the rows' text, so the person's agents don't see the conversation. Adding one is a separate privacy choice.
- **Freeing the space** of old conversation rows. The log can't be trimmed (storage.md §4). New chat only hides.
- **Waiting out a usage limit.** A turn that hits one ends with an error and **Ask again**. Holding questions until the limit lifts could come later.

## 14. Found along the way

- `spawn_remote.rs` comments say `assistantTurn` means Claude Code is signed in. It means the `claude` command was found. Card 1 fixes the comments.
- assistants-talk.md said `spawn.answers` is a kind. It isn't: it is an in-memory map of brief notes in `spawn.rs`. It also said the chat and its answering turn run on Haiku, and that the assistant's computer has Claude Code "signed in". Fixed in that doc with this change.
- call-agents.md hid **Divide with my assistant** on the phone and put its `callId` on `assistant_ask`. Fixed in that doc with this change (§9).
- `hosts()` (`spawn_remote.rs`) reads only the newest 1,000 `spawn.host` rows and doesn't page, so a device whose newest row is further back drops out. Card 1 reads a named device's row directly instead. Not otherwise this card's.
