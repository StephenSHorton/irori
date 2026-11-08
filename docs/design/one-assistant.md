# One Office Assistant across your devices: design

*2026-11-08, card #1095, from the person's words on thread #1093. Design only: nothing here is built yet. The build cards are in §12. Checked against main at `62e5f59a`. Line numbers are approximate: go by the names.*

The person asked on #1093 whether the Office Assistant starts on only one of their devices, "because all my devices can share that assistant". Then, after hearing that today each device runs its own:

> "I was kind of implying I want to commit to a design where there is only one assistant started. I think it would help conversation context share across the devices if one Claude that is started for that is handling it across devices. If you can't connect, you got more issues than that."

So this design has one assistant per person. It runs on one computer, the assistant's computer, and it handles the chat from every device, the phone included. Every device shows the same conversation. When that computer can't be reached, the chat says so plainly, and no other device starts a second assistant.

## 1. Decisions

| | Decision | Why |
|---|---|---|
| D1 | **One assistant per person. Its chat turns run only on the assistant's computer.** Every other device, the phone included, sends its questions there and shows the replies. No other device runs a chat turn. | The person's words on #1093. |
| D2 | **The assistant's computer is recorded in a row, not worked out from who is online.** It stays the same while that computer is off. | Today's pick moves to whichever desktop is up when the home drops (§2), and that is a second assistant starting. |
| D3 | **It moves only when the person picks another computer, or the computer is removed from their devices.** When it is off, asleep, signed out of Claude Code or missing Claude Code, the chat says so, names the computer and says what to do there. | "If you can't connect, you got more issues than that." Moving on a guess about reachability is how two assistants start. |
| D4 | **It is set by itself the first time.** A computer that can run the assistant claims itself when there is no claim. The first claim wins, and a person's pick beats any claim made by itself. | Decision 52: it works with no switching on. Settings only changes it. |
| D5 | **The conversation is rows on the person's private log**: questions, replies, the chat's follow-up lines and New chat. Every one of the person's devices shows the same conversation. No other person ever gets the rows. | Shared context is the point. The private log reaches only the person's own devices. |
| D6 | **A question row is the request.** The assistant's computer answers each unanswered question as its row arrives, one at a time, in order. | It needs no new transport. A question reaches the computer through any device that carries it, and waits safely while the computer is away. One turn at a time keeps one conversation, and keeps #987's grants safe (§5). |
| D7 | **First, one turn per question over the shared rows. Then a held session on the assistant's computer (card 8).** The rows are the record. The session is a warm copy that can always be rebuilt from them. | It gives "one Claude that is started for that" without making the conversation depend on one process staying alive. §8 compares the two. |
| D8 | **When the computer is away, the question waits, and the chat says so plainly.** When it is back, it answers what waited, the newest five at most, each told how long ago it was asked. | Plain status, no second assistant. They are the person's own questions, so they get answers, not a "too late" line. |
| D9 | **The turn, its tools, its grants and thread posting run on the assistant's computer.** Going to a page (`<<go>>`) happens only on the device that asked. Taps work on any device. | Navigation belongs to the screen the person is looking at. Everything else is the one assistant's work. |
| D10 | **Answering other people (assistants-talk) and the huddle assistant (call-agents) use the same recorded computer.** The person's taps (**Let it answer**, **Answer this**) are sent to it too, so they work from a phone. | One assistant. Today those designs run turns on the device tapped, or fail over to the next device. §9 lists the changes. |

## 2. What is there today

Checked on main at `62e5f59a`:

- **The chat runs on whichever device it is typed on.** `Window::assistant_ask` (`crates/window/src/assistant.rs`, around line 1094) renders the prompt, sets the screen snapshot, begins grants and starts one `claude -p` through the window's `assistant` `Runner`. The flags include `--no-session-persistence`, so there is no session between turns. Model and effort come from `spawn_setting("")`, which is one row for every device (default Sonnet, low).
- **Tools run on the device that runs the turn.** `irori mcp` dials the local Core (`app/src/mcp_cmd.rs`), and the caller is `user:<that device>` with kind `assistant` (`crates/api/src/server.rs`). Most of `ASSISTANT_TOOLS` read replicated rows. `screen` reads a slot that this Core's last ask set (`Core::set_assistant_screen`). `timeline` and `why` read this device's capture.
- **History is the webview's localStorage**, key `irori.assistant.history` (`frontend/src/lib/assistant.ts`, `loadHistory`/`saveHistory`), capped at 50 entries. The follower's memory of what it has said is `irori.assistant.follow`. Both are per device. The frontend sends `historyFor(entries)` with each ask, and the host renders the last 12 turns (`render_prompt`, `HISTORY_TURNS`).
- **A reply can post a thread by itself.** `finish()` calls `autoPost()` when `postsByItself` holds, whether the panel is open or closed. It calls `BoardService.StartThread` with `assistant: true` and then follows the thread. Going to a page (`<<go …>>`) changes this window's route, and only while the panel is open.
- **#987's grants are one set per Core.** `Grants::begin` replaces the set on each run. It is built by `grants_for(text, earlier)` from the person's typed message and earlier typed messages in this device's history. The doc comment says it relies on one question at a time. The window's `Runner` allows two runs at once (`MAX_RUNS`).
- **The phone shows the chat**, but every ask ends in `NO_CLAUDE`: "The assistant can't run on this device yet" (`assistant.rs`). The phone runs a full Core and replicates the whole private log (`app/src/lib.rs`).
- **The assistant's computer is worked out on each device, and it moves.** `assistant_home_of` (`crates/core/src/service/spawn_capable.rs`) takes the first of `AgentDevices.homes` whose `spawn.host` says `assistantTurn`. `refresh_agent_devices` ranks `homes` by `(tier, no trusted folder, id)`, and tier 0 is "this device, or one connected now". So:
  - A desktop always ranks itself first among the devices it can't reach. Two desktops that can't see each other both pick themselves.
  - When the home goes quiet and another desktop is connected, that desktop becomes home within about 50 s, and the home moves back when the first returns. There is no grace period.
  - `is_assistant_home` has one caller, `mint_assistant_cards` (`crates/core/src/service/boards.rs`).
- **`assistantTurn` means the `claude` binary was found, not that it is signed in.** It is `ready && os != "android"` in `publish_host`, and `ready` is `find_claude().is_ok()`. The comments say "signed in".
- **Of assistants-talk's build cards, only card 2 (#1058) is on main**: the assistant card, its profile pointer and `assistantTurn`. Card 1 (#1057, the channel) and #1045 (naming) have open PRs. Cards 3 to 8 (#1059 to #1064) are not built. The huddle assistant (call-agents §7) is not built.
- **Device to device, there are only rows and the private mesh.** A row is pushed to connected devices as soon as it is appended (`crates/mesh/src/node.rs`, `wake_all`), and every Core subscriber sees it as it is inserted (`Store::subscribe`). Request rows like `spawn.end.requested` are served by the 5 s sweep, not on arrival. The only request and answer stream on the mesh is the attachment fetch (`Node::fetch_file`, a `blob/<id>` stream), which reports `FetchError::Offline` when the holder isn't connected. There is no LAN HTTP between devices.
- **Rows are permanent.** The private log has no size limit per row, no retention and no delete (storage.md §2, §4; decision 25). The mesh sends one message of at most 8 MiB.

## 3. The assistant's computer

### The row

A new private kind, `assistant.home`, declared in `crates/store/src/kinds.rs` beside `assistant.names.set` as `Replicated, Refuses`:

```
{ "device": "<device id>", "how": "auto" | "person", "by": "user:<writer device>", "at": <GoTime> }
```

Actor `user`. Any of the person's certified devices may write it. The private log accepts only those devices, and the fold skips rows with another actor, as `newest_switch` does.

### Which row wins

A new fold, `assistant_home(store)`, in a new `crates/core/src/service/assistant_home.rs`. It considers only rows whose `device` is still a valid computer: certified, not revoked, and with a `spawn.host` row that says `assistantTurn`. Then:

1. **A person's pick beats any automatic claim.** Among `how: "person"` rows, the newest by `(at, device_id)` wins, as `newest_switch` folds.
2. **Among automatic claims, the first wins**, by `(hlc, device_id, seq)`, the order the store lists rows in. A late claim never takes over from an earlier one. So a newly paired desktop that claims itself before it has synced loses as soon as it syncs.
3. **No valid row means no assistant's computer yet.**

Every device folds the same rows to the same answer, whatever it can reach. That is what makes the pick stable.

### Setting it by itself

In `sweep_spawns` (`crates/core/src/service/spawn.rs`), after `refresh_agent_devices` and before `mint_assistant_cards`:

- If the fold has no valid row, and `assistant_home_of(homes, hosts)` (today's rule, also filtered on `assistantChat`, below) names this device, write `{device: <this device>, how: "auto"}`.
- A device only ever claims itself. It never writes an automatic row naming another device.
- It writes nothing while a valid row exists, even when the assistant's computer has been offline for days.

### When it moves

Only these move it:

- **The person picks another computer**, in Settings › Agents or with the one tap on the away line (§6). Either writes a `how: "person"` row, from any device, the phone included.
- **The computer is removed** from the person's devices (revoked). Its rows stop counting, and the next sweep on a computer that can run the assistant claims itself.
- **A computer that no longer says `assistantTurn` stops counting.** This happens only when its own newest `spawn.host` row says so, for example after Claude Code is uninstalled. That row is a fact the computer wrote about itself, so other devices read the same thing, and it is never a guess about reachability. Note that `assistantTurn` stays true when Claude Code is merely signed out (§2). A signed-out computer stays the assistant's computer, and each turn's error says to sign in there (§6).

Nothing else moves it: not being offline, asleep or unreachable, not a slow link, not another desktop with a lower id or a trusted folder, and not Stop here.

### Saying it can answer from other devices

A new `assistantChat: true` field on `spawn.host`. It means "this build answers chat questions sent from other devices". Like `assistantTurn`, it joins `publish_host`'s change tuple, or an upgraded desktop never publishes it. Automatic claims and the Settings picker only consider devices that say it. An older build never claims, so questions never wait on a computer that will never read them.

### Reading it

- `Services::assistant_home() -> Option<String>`: the fold's answer, cached and refolded when an `assistant.home` or `spawn.host` row arrives (a branch in the kind loop beside `ASSISTANT_NAMES_SET`, `crates/core/src/service.rs` around line 2063), and when a device is revoked.
- `is_assistant_home()` reads it, not the ranking. So `mint_assistant_cards`, and every later caller, get the recorded computer.
- A Tauri command `assistant_home_status` returns `{device, name, state, lastSeen, via, chat, here}`, with `state` and `lastSeen` from `device_presence` (`crates/core/src/service/liveness.rs`). The chat and Settings show it (§6).

### Stop here and Stop everywhere

Decision 60's switches stop agents and hold their starts. The assistant isn't an agent: it is off the roster and counted against no limit. So:

- They don't stop the person's own chat. The person is asking.
- They hold what the assistant does by itself: automatic answers to other people become "Ask me" (as assistants-talk §5 already says for Pause all), and the huddle assistant stays out of calls.
- They never move the assistant's computer.

## 4. The conversation as rows

### Kinds

All are private, `Replicated, Refuses`, and declared in `kinds.rs` with an `assistant.` prefix. The prefix is outside `TOLERATED_FAMILIES`, so `Refuses` is allowed. Each is added to `SINCE_GO_REFUSED` in `crates/store/tests/it/golden.rs`.

| Kind | Actor | Written by | Payload |
|---|---|---|---|
| `assistant.chat.asked` | `user` | The device the person typed on, through a host command only | `turn` (a new ULID), `text`, `screen` (that device's snapshot, cut to 4,000 characters as today), `device`, `at`, optional `again` (the turn it repeats) |
| `assistant.chat.said` | `irori` | The assistant's computer | `turn`, `text`, `changed` (the write tools' lines), `posted` (`{id, short, tag}` of a thread it posted), `looked` (the screen label, not its text), `error`, `skipped`, `device`, `at` |
| `assistant.chat.update` | `irori` | Any device's follower | `key` (a hash of the thread id and the fact's key), `thread`, `text`, `act` (as today's `{kind: "answer", card, blocker}`), `at` |
| `assistant.chat.posted` | `user` | The device where the person tapped **Post thread** | `turn`, `posted` |
| `assistant.chat.cleared` | `user` | The device where the person tapped **New chat** | `by`, `at` |
| `assistant.chat.stop` | `user` | The device where the person tapped Stop | `turn`, `by`, `at` |

No tool writes any of these, and no tool may ever write `assistant.chat.asked`: #987's grants are made from it (§5).

### The fold

A new `crates/core/src/service/assistant_chat.rs` folds the rows into one conversation, the same on every device:

- **A turn** is its `asked` row plus at most one `said` row. The first `said` row for a turn wins, by `(hlc, device_id, seq)`, as `notification.posted`'s `older` does. Two `said` rows for one turn should only happen in the moments described in §7. This rule is a safety net, not how one assistant is kept.
- A `said` row that arrives before its `asked` row waits for it. It is never shown alone.
- **Order:** turns go by their `asked` row's HLC, and each reply shows under its question. `update` lines go by their own HLC. Within one device the order is gapless, and across devices `insert_batch` keeps causal order (a reply written after its question was inserted sorts after it).
- **`update` lines:** the first row per `key` wins, as notifications do. Every device's follower may write one, and duplicates fold to one.
- **`posted`:** the first per turn wins, and it fills the turn's `posted` when the `said` row has none.
- **New chat:** the newest `cleared` row, by `(at, device_id)`, hides every turn whose `asked` row is older by HLC, and every `update` line older than it. Its `said` row is hidden with it. Rows are never deleted, so this only hides them (storage.md §4).
- **What is shown:** the turns and lines after the newest clear. The list command pages them, 50 at a time, newest first.
- It loads at start and on resync, in the same block as notifications (`load_notifications`, `crates/core/src/service.rs`), and applies arriving rows in the kind loop.
- **Events:** a new `CoreEvent::AssistantChat` and topic, pumped to the window as `assistant:changed` (`crates/window/src/events.rs`), as `notifications:changed` is.

### Scrubbed before signing

A row is permanent on every device, so a secret that gets into one stays there for good (v0.4 §5.1). Before appending, the writer runs the Core's scrubber (the `scrubber` field on `Services`, which capture uses and which holds the exact secret values `add_secret` gives it) on every text field: the question on the asking device, the reply and its `changed` lines on the assistant's computer, and follow-up lines on their writer. A match becomes `[redacted:<kind>]`, as capture does. The reply is scrubbed before it is shown on the computer that ran it, so every device shows the same text.

### Size and growth

- A question is at most the person's typed text plus 4,000 characters of screen. A reply is the model's text, cut to `MAX_PERSON_TEXT` (512 KiB), which keeps a row well inside the mesh's 8 MiB message.
- `looked` keeps the screen label only. Today's `ChatEntry.looked` holds the whole screen text the model read, once per reply. That would double the screen in every row, and it isn't needed: the question's row already has it.
- A heavy day of 100 questions is under 1 MB, kept for good on every device, the phone included. Capture already writes a row per agent turn, so this is well within precedent. If storage shows the screen is the bulk, cut it in the row.

### Who can read them

- Only the person's devices hold the private log, end to end over QUIC. No relay or server stores it (public-relays.md).
- No tool reads these kinds, so the person's agents don't see the conversation, as today. The assistant itself gets its history in the prompt, not through a tool.
- The rows can quote what the assistant read: private cards, other devices' agents' summaries. Today that text sits in one device's localStorage. Now it is on every one of the person's devices, which is what sharing the conversation means. It never leaves the person's devices, which keeps decision 53's "Mine" scope.

## 5. Asking from any device

### On the device that asks

- `ask(text)` calls a new host command, `assistant_chat_ask(text, screen) -> turn`, in place of `assistant_ask`. The host scrubs the text and appends `assistant.chat.asked` with this device's id, and the fold shows the question at once.
- The frontend sends no history. The assistant's computer builds it from rows.
- **What the chat shows under the question while it waits:**
  - On the assistant's computer itself, the reply streams as it does today (below).
  - On another device, "Mac is answering…" while Mac is connected (`assistant_home_status`), or the away line (§6). Card 7 streams the text here too.
- **Stop** appends `assistant.chat.stop`. Any device can stop any turn.

### On the assistant's computer

**The worker.** A new hook, `Core::set_chat_turn`, installed on desktops from `app/src/shell/mod.rs` as `set_namer` is. Core keeps the queue, the checks and the rows. The window does the run:

- `ChatTurn` is `Fn(&ChatJob, &dyn Fn(AssistantEvent) -> bool) -> ChatDone`. `ChatJob` holds `turn`, the rendered prompt, the screen and the grants. `ChatDone` holds the text, error, `changed` lines and `posted`.
- The window's closure runs `Job::assistant` on its existing `assistant` `Runner`, with the same flags, tools and secret as today.
- One worker thread takes jobs from a channel and survives a panic, like the namer's.

**When it takes a question.** Questions are queued from the kind loop as their rows arrive, local ones included, not from the 5 s sweep. They are also queued by a catch-up scan when the hook is installed and when this device becomes the assistant's computer. Each question is checked in this order, and the first failing check drops it:

| Check | If it fails |
|---|---|
| This device is the assistant's computer (`is_assistant_home`) | Nothing. Another computer answers. |
| The turn has no `said` row | Nothing |
| The turn has no `stop` row | Write a `said` row with `error: "Stopped"` |
| It is after the newest New chat | Nothing |
| It is one of the newest five unanswered questions | Write a `said` row with `skipped: true` (§6) |

**One at a time, in order.** The worker runs one turn at a time, oldest question first. So a question sent from the phone while the Mac's own question is running waits for it, and its turn sees that answer. This also keeps `Grants`' "one set per Core" rule true, since no two chat runs overlap. Grants still `begin` and `end` per run. The window's `MAX_RUNS` of 2 stays for the other kinds of run.

**The prompt.** It is built in Core from rows, by a port of today's `historyFor` and `turnText` (`assistant.ts`) and `render_prompt` (`assistant.rs`):

- The turns after the newest New chat. Runs of `update` lines are grouped into one assistant turn, as `historyFor` does. A posted draft gets its "(This draft was posted as thread …)" note.
- The last 12 turns go in, as `HISTORY_TURNS` says today.
- A question asked more than two minutes before its turn starts is marked: "(Asked at 14:02 on the phone; it is now 17:40. Your computer was away.)" So an old question gets today's answer and says so.

**Grants.** The question's row is the person's typed message. `grants_for(text, earlier)` takes its `text`, and `earlier` is the `text` of the person's earlier `asked` rows after the newest New chat, at most 50, as today. The parser is unchanged: it still reads only the person's words, and a grant is still spent once.

**The screen.** `set_assistant_screen(row.screen)` runs just before the turn starts, under the worker, so the `screen` tool sees the snapshot of the device that asked. `screen_label` gives the `looked` label.

**`timeline` and `why`** read the assistant's computer's capture. That is the one assistant's view, and it is the same whichever device asked. Accepted.

**Posting a thread by itself.** `splitDraft` and `postsByItself` (`assistant.ts`) are ported to Rust in the window. When a finished reply posts by itself, the window's closure calls `Window::start_thread_with(…, assistant: true)` after the run and returns `posted`. It then appears on every device in the `said` row. Only the assistant's computer ever posts a draft by itself, so every device seeing the same reply posts nothing. The frontend's `autoPost` goes away.

**The reply row.** When the run ends, Core scrubs the text and `changed` lines, and appends `assistant.chat.said`. On the assistant's computer, the live events also go to its own window as a `CoreEvent` keyed by `turn`, so that window streams the reply as today. The stream is a view: the row is the reply.

**When the run fails.** `NO_CLAUDE`, `TOO_LONG`, a signed-out Claude Code, or any other error goes in the `said` row's `error`, in today's words plus the computer's name: "Claude Code on Mac couldn't answer: …". Every device shows it.

### Markers

- **`<<thread>>` drafts.** The reply row carries the reply text, markers included, and every device's frontend renders the draft as today. **Post thread** (a check-first draft) works on any device. It starts the thread from that device, then appends `assistant.chat.posted`, and every device hides the button once a `posted` exists. Two taps on two devices within the second it takes rows to cross would start two threads. Accepted: it needs two taps at once.
- **`<<go …>>`** goes only on the device whose id is the question's `device`, and only while its panel is open, as today. Every other device shows the go button, which the person may tap.

### Follow-up lines

- The follower (`assistant-follow.ts`) follows each thread a turn posted, read from the fold's `posted`. It runs on every device, the phone included, so the person can answer a card's question from wherever they are.
- When it has a line to say, it appends `assistant.chat.update` with a key built from the thread id and the fact's key (`cant:`, `perm:`, `ready:`, `ask:` and the card or blocker id). The first row per key wins, so every device saying it at once still shows one line.
- What it has already said comes from the fold's keys, not from `irori.assistant.follow`. That key goes away (§10).
- #1000's cut is unchanged: only lines that wait on the person are written.

### The dot

- A reply lights the dot only on the device that asked, when its panel is closed.
- A follow-up line lights it on every device, since it waits on the person wherever they are.
- Each device keeps its own "seen up to" in localStorage. It is a per-device convenience, not part of the conversation.

## 6. When the computer is away

The chat header says where the assistant runs: "on Mac", with the device dot from Settings › Devices.

| State (from `assistant_home_status`) | What the chat shows | Composer |
|---|---|---|
| Connected | Nothing extra. Under a question from another device: "Mac is answering…" | On |
| Recent or offline | Above the composer: "Your assistant runs on Mac, which is offline (seen 2 hours ago). Questions wait for it." With **Use PC instead** when another computer that can run the assistant is connected now | On: the question waits |
| No assistant's computer: no desktop has Claude Code | "Your assistant needs a computer with Claude Code. None of your devices has it yet." | Off |
| The assistant's computer runs an older Irori (no `assistantChat`) | "Mac needs the latest Irori to answer from your other devices." | Off on other devices |

- **No second assistant starts.** The question row sits on the asking device, and on any device it syncs to, until the assistant's computer gets it. The phone pushes its newest rows when it is put away (`sync_out`), so a question asked just before locking the phone still leaves.
- **When the computer is back,** the catch-up scan queues every unanswered question after the newest New chat. The newest five are answered in order, each marked with how long ago it was asked (§5). Older ones get a `said` row with `skipped: true`, which shows "Mac didn't get to this: it was away. **Ask again**". Ask again appends a new question with the same text and `again` set.
  - Why five: the questions are the person's own, so they should be answered, not dropped for being late, and the age note keeps a late answer honest. The cap keeps a day of questions from a phone from turning into a burst of turns.
- **Use PC instead** writes a `how: "person"` row naming the PC. It is the person's choice, made with one tap, never by itself. If the Mac was only unreachable, it sees the newer row when it is back and stops answering (§7).
- **A signed-out Claude Code** on the assistant's computer shows in each turn's error: "Claude Code on Mac couldn't answer: it isn't signed in. Run `claude` in a terminal on Mac to sign in." The computer doesn't move (D3).
- **On the phone** the chat now works. There is no `NO_CLAUDE` on a phone any more: its questions go to the assistant's computer like anyone else's.

Settings › Agents (`frontend/src/components/settings/agents-pane.tsx`), beside the assistant's model, gains **Assistant's computer**: a picker of the person's computers that say `assistantTurn` and `assistantChat`, each with its dot and "seen …". Picking one writes a `how: "person"` row. With no row yet it shows the computer that will claim itself. It only changes the choice. The assistant works without anyone touching it (decision 52).

## 7. Two desktops at once

Every rule above reads replicated rows, not each device's view of who is online. That leaves a few short moments when two desktops can both run a turn:

| When | What happens | How it ends |
|---|---|---|
| **First claim during a split.** Two desktops that can't see each other both find no claim and claim themselves. | Each answers the questions that reach it. | When they meet, the first claim wins on both. A question both answered has two `said` rows, and the first wins on every device. Accepted: it needs a split during the one-time first claim. |
| **The person moves it while the old computer is split off.** The phone writes "PC", and the Mac doesn't hear. | The Mac gets no new questions while it is split off, because it is the rows that reach it. | When it is back, the Mac reads the newer row before the queued questions (same sync) and stops. A question it had already started finishes, and if the PC answered it too, the first `said` row wins. |
| **A newly paired desktop claims before it syncs.** | It sees no questions yet: they come with the same sync. | Its claim is later than the existing one, so it loses as soon as it syncs. |
| **An older build** on one of the person's devices. | It keeps its own local chat, as today, until it updates. It never claims and never answers others' questions. | Updating it. Accepted. |
| **Asymmetric links**: the Mac hears the PC, but not the other way. | Nothing. Neither link decides anything. | n/a |

So "split-brain" is down to the one-time first claim and the person's own move during a split, and the first-`said`-wins fold makes even those show one answer.

The worker also checks `is_assistant_home` again just before it starts each turn, so a computer that has just learned it lost stops before its next turn, not after the queue empties.

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
  - Each question writes one user message: the screen and grants for that turn are set first, as in §5. Any rows that came in since its last message (follow-up lines, questions answered while it was down) go in that message as "since we last spoke" lines.
  - It is seeded again from the rows (§5's prompt) when the process dies, after New chat, when the assistant's computer changes, or past about 60k tokens of context, as call-agents §7 does.
  - It stays one turn at a time, so the grants rule holds.
  - Card 8 measures it with tools on the chat's model against card 3's turns. The held session stays only if it is faster and answers no worse.

## 9. Answering other people, and the huddle

### Assistants talk (docs/design/assistants-talk.md)

Its cards 4 to 8 (#1060 to #1064) aren't built yet. They change as follows, and come after card 1 here:

- **The gate for an automatic answer** (§5's "This device is the assistant's computer") is `is_assistant_home`. After card 1 here, that reads the recorded computer, so a short link drop no longer moves who answers. The repeated answer id stays as the safety net for §7's moments.
- **Let it answer and Answer this go to the assistant's computer.** A tap on any device appends a new private row, `assistant.answer.requested {post, space, how: "let" | "draft", by, at}`. The assistant's computer runs the turn: the tool-less turn for `let`, the owner's-OK turn for `draft`. A phone gets both buttons instead of "Answer from your computer", because it no longer runs anything itself. The first request per `(post, how)` is served, and later ones do nothing.
- **The owner's-OK draft** (assistants-talk §7) is written as an `assistant.chat.said` row with a `draft: {post, space}` field. Every device shows it, with **Send to Eve's assistant** and **Don't send**. Send works from any device and posts as the assistant's card, as designed there. The answer id still refuses a second Send. The draft is private text and stays in private rows. It is never sent to the model as a person's turn.
- **`held` lines** (assistants-talk §8) become `assistant.chat.update` rows with a key built from the question's post id, so every device shows them, not only the assistant's computer.
- **Stop here** on the assistant's computer turns automatic answers into "Ask me", as §5 there says for Pause all.

### The huddle (docs/design/call-agents.md §7)

- **Which device hosts it** is the recorded assistant's computer, nothing else.
- Its rule "An `on` whose device has been away in presence for over 60 s counts as off … and the next device takes over" is dropped. When the assistant's computer is away, the roster shows "not connected: Mac is offline", the same plain line as the chat.
- `call.assist` stays, so the call knows whether the assistant is connected.
- Stop here or Stop everywhere on the assistant's computer keeps the assistant out of calls (§3).

### What stays where it is

Irori's own side turns aren't the assistant's conversation: thread naming, card weighing, draft rewriting and the merge desk's reviewer pick (merge-desk §12.5, card #1081). Each is a tool-less, single-shot turn with no history, run by whichever device the work lands on. They keep running where they run today.

## 10. Moving today's history

Today each device has its own history in localStorage. Putting them all into one conversation would interleave unrelated chats in an order nobody had them in (the entries have no times), and feed the mix to the model. So:

- **The assistant's computer brings its own history in, once.** On the first load where this device is the assistant's computer and the shared conversation has no question yet, the frontend reads `irori.assistant.history` through `parseHistory` and sends it to a new host command, `assistant_chat_import(entries)`. The host writes the entries in order as `asked` and `said` rows with `imported: true`, scrubbed like any row. Then it sets a local flag, `irori.assistant.imported`, so it never runs twice. It goes on the computer that is the assistant now, because that is the conversation the person has been having with it.
- **Every other device keeps its old history, read-only**, above a divider: "Earlier on this phone, before your chats were shared". It isn't sent to the model, isn't replicated, has no buttons that act, and goes when the person taps New chat.
- **`irori.assistant.follow` is removed.** The fold's `update` keys take its place. A thread that still waits on the person may get its line said once more after the update. Accepted: it still waits on them.
- **If another device became the assistant's computer first**, the shared conversation already has questions, so nothing is imported, and this device's history stays read-only like any other's.

## 11. Versions

Nothing needs a gate:

- Every new kind is private. Older builds store and pass on kinds they don't know (no-server.md: "The private log stores unknown kinds and folds them after an update").
- `assistantChat` is an optional field on `spawn.host`. Older readers ignore it.
- An older build keeps its own local chat until it updates (§7). It never claims, so nobody's questions wait on it.

## 12. Build plan

Eight cards, each with Files, Steps and Done when. Each is normal weight unless marked, and links here. Every card: tests from the repo's patterns, `sh scripts/check.sh`, and scratch builds per CLAUDE.md (a copied `IRORI_HOME`, an empty `CLAUDE_CONFIG_DIR`, `IRORI_DEV_PORT_OFFSET`). The order: 1 and 2 side by side; 3 after both; 4 after 3; then 5, 6 and 7 after 4, in any order; 8 after 4. Assistants-talk's cards #1060 to #1064 come after card 1, and its #1062 and #1064 after card 4 (§9).

**1. The recorded assistant's computer.**
Files: `crates/store/src/kinds.rs` and `crates/store/tests/it/golden.rs` (`assistant.home`), new `crates/core/src/service/assistant_home.rs` (the row, the fold, `assistant_home()`), `crates/core/src/service/spawn_capable.rs` (`is_assistant_home` reads the fold, and `assistant_home_of` also needs `assistantChat` for the claim), `crates/core/src/service/spawn_remote.rs` (`assistantChat` on `SpawnHost`, in `HostShown` and `publish_host`'s tuple; fix the comments that say "signed in" to say "the `claude` command is found"), `crates/core/src/service/spawn.rs` (the claim in `sweep_spawns`, before `mint_assistant_cards`), `crates/core/src/service.rs` (refold on `assistant.home` and `spawn.host` rows and on revocation; a `pub` accessor), `crates/window/src/commands.rs` (`assistant_home_status`, `assistant_home_set`).
Steps: the kind and payload; the fold with its three rules; the claim; `is_assistant_home` on the fold; the status and set commands; tests.
Done when tests show:
- Two Cores that claim during a split agree on the first claim after they meet.
- A person's row beats an earlier automatic claim, and the newest person's row wins.
- With the assistant's computer's Core stopped, another Core that can run the assistant never becomes it, never claims, and `is_assistant_home` stays false there.
- A revoked computer's row stops counting, and another computer claims itself.
- A computer whose own `spawn.host` drops `assistantTurn` stops counting. One that drops nothing keeps it, however long it is offline.
- A device without `assistantChat` never claims.
- `mint_assistant_cards` mints on the recorded computer only (extend `crates/core/tests/it/assistant_card.rs`).

**2. The conversation rows and their fold.**
Files: `crates/store/src/kinds.rs` and `golden.rs` (the six `assistant.chat.*` kinds), new `crates/core/src/service/assistant_chat.rs` (payloads, fold, scrub helper, list), `crates/core/src/service.rs` (load at start and on resync, the kind-loop branch), `crates/core/src/events.rs` (`CoreEvent::AssistantChat` and its topic), `crates/window/src/events.rs` (the `assistant:changed` pump), `crates/window/src/commands.rs` (`assistant_chat_list(before, limit)`), `crates/window/src/storage.rs` (`log_part` names the `assistant.` prefix), `frontend/src/gen/bindings.ts` and `frontend/src/lib/host.ts`.
Steps: the kinds; the fold (turns, first `said` wins, `said` waits for `asked`, `update` first per key, `posted`, New chat); load, apply and the event; the list command; the scrub helper every writer uses; tests.
Done when tests show:
- Rows written on two Cores fold to the same list, in the same order, on both.
- A `said` row that arrives before its `asked` row shows once the question arrives, and not before.
- Two `said` rows for one turn show one, the same on both Cores.
- Two `update` rows with one key show one.
- New chat hides older turns, their replies and older lines, and a question asked after it shows.
- A reply containing an Anthropic key is stored as `[redacted:anthropic]`.
- A row with actor other than its kind's (for example `asked` with actor `irori`) is skipped.

**3. The assistant's computer answers questions from rows.**
Files: `crates/core/src/service/assistant_chat.rs` (the queue, the checks of §5, the catch-up scan, the prompt from rows ported from `historyFor`, `turnText` and `render_prompt`, grants from rows, the `said` row, stop), `crates/core/src/service.rs` (`set_chat_turn`, `ChatJob`, `ChatDone`, the live `CoreEvent` by turn), `crates/window/src/assistant.rs` (the closure: set the screen, `begin` and `end` grants, run `Job::assistant`, port `splitDraft` and `postsByItself`, post with `Window::start_thread_with(…, assistant: true)`), `app/src/shell/mod.rs` (install it on desktops), `crates/window/src/commands.rs` (`assistant_chat_ask(text, screen)`, `assistant_chat_stop(turn)`).
Steps: the ask and stop commands; the hook and a fake turn; the queue and checks; the prompt; grants and screen; the real closure; posting by itself; tests.
Done when, with a fake turn returning fixed text, tests show:
- A question appended on one Core is answered once, by the recorded computer only, and the reply is under it on both Cores.
- Two questions from two Cores at once run one after the other, oldest first, and the second turn's prompt holds the first's answer.
- A question that arrived before the hook was installed is answered by the catch-up scan.
- With six unanswered questions, the newest five are answered in order and the oldest gets `skipped`. A question asked an hour before its turn has the age note in its prompt.
- A Stop row before the turn starts gives "Stopped". One during the run ends it.
- `grants_for` gets the question's text and the earlier `asked` texts after New chat, and a grant isn't spent by another run.
- The `screen` tool sees the asking device's snapshot.
- A reply whose draft posts by itself starts exactly one thread, from the recorded computer, and its `said` row carries `posted`.
- A Core that learns it isn't the assistant's computer stops before its next queued turn.
A real run on a scratch build answers a question asked from a second scratch instance.

**4. The chat reads and writes rows.**
Files: `frontend/src/lib/assistant.ts` (entries from `assistant_chat_list` and `assistant:changed`; `ask` through `assistant_chat_ask`; stop; New chat appends `cleared`; Post thread appends `posted`; `<<go>>` only on the asking device; the dot rules; drop `autoPost`, `saveHistory` and the `history` argument), `frontend/src/lib/assistant-follow.ts` (follow `posted` from the fold; write `update` rows; read what it has said from the fold), `frontend/src/components/assistant/assistant-chat.tsx` ("Mac is answering…"; the streamed reply on the assistant's computer), `crates/window/src/commands.rs` (`assistant_chat_posted`, `assistant_chat_clear`, `assistant_chat_update`).
Steps: read the fold; ask, stop and clear; drafts and Post thread; go on the asking device only; the follower on rows; the dot; tests.
Done when:
- Tests show: a reply's `<<go>>` acts only where it was asked; Post thread on one device hides the button on the other; two followers saying one line give one line; a reply lights the dot only on the asking device, and a follow-up line on both.
- A two-instance scratch run shows the same conversation on both. A question typed on the instance that isn't the assistant's computer is answered by the one that is, and both show it.
- Screenshots at desktop and phone width.

**5. Away, and choosing the computer.**
Files: `frontend/src/components/assistant/assistant-chat.tsx` (the header's "on Mac" and dot; the lines of §6; **Use PC instead**; the composer rules), `frontend/src/lib/assistant.ts` (`assistant_home_status`, read again on `devices:changed` and every 5 s while the panel is open, as `useDevices` does), `frontend/src/components/settings/agents-pane.tsx` (**Assistant's computer**), `crates/window/src/assistant.rs` (the computer's name in turn errors, the signed-out words), `crates/core/src/service/assistant_chat.rs` (`skipped` and Ask again).
Steps: the header; each line of §6's table; Use PC instead; Settings; Ask again; the phone; tests.
Done when:
- Tests show each state of §6's table renders its words and composer rule. Use PC instead writes a person's row. Ask again appends a question with `again`.
- A two-instance scratch run with the assistant's computer stopped shows the away line on the other, a question waiting, and the answer arriving after it starts again, with the age note.
- Screenshots of the away line at desktop and phone width, and of Settings.

**6. Moving today's history.**
Files: `frontend/src/lib/assistant.ts` (the one-time import on the assistant's computer; the read-only "Earlier on this …" block elsewhere; removing `irori.assistant.follow`), `crates/window/src/commands.rs` (`assistant_chat_import(entries)`), `crates/core/src/service/assistant_chat.rs` (writing imported rows in order, scrubbed, with `imported: true`), `frontend/src/components/assistant/assistant-chat.tsx` (the divider and block).
Steps: the import command; the import's conditions; the read-only block; removing the follow key; tests.
Done when tests show: the assistant's computer imports its history once, in order, and a reload doesn't import again; a device that isn't it imports nothing and shows its history read-only, not in any prompt; nothing is imported when the conversation already has a question; New chat removes the read-only block.

**7. The reply streams to other devices.**
Files: `crates/mesh/src/files.rs` (an `assistant/<turn>` arm in `serve_stream`, served only to the person's certified devices), a new `crates/mesh/src/assistant_stream.rs` (client and server, framed like inline-terminals §6.4: `u8 type ‖ u32 len ‖ body`), `crates/core/src/service/assistant_chat.rs` (a broadcast of a running turn's events), `crates/window/src/assistant.rs` and `assistant-chat.tsx` (open the stream on the asking device when the assistant's computer is connected; show deltas and tool lines).
Steps: the server arm; the client; wiring the live events; the fallback; tests.
Done when tests show: a second Core connected to the assistant's computer gets the deltas of a running turn in order; a dropped stream falls back to waiting for the `said` row with no error shown; the row is the reply either way, and the streamed text never differs from it after the end. A two-instance scratch run shows the reply streaming on the device that asked.

**8. A held session on the assistant's computer.** *(heavy: a measurement and a judgment call)*
Files: `crates/window/src/assistant.rs` (a held run beside `Job`: `--input-format stream-json`, stdin kept open, the chat's tools and secret; one message per question; reseed), `crates/core/src/service/assistant_chat.rs` (the "since we last spoke" lines; when to reseed).
Steps: measure first, with tools on the chat's model, a held session against card 3's turns over 20 questions; build it only if it is faster and answers no worse; per-message screen and grants; reseed on process death, New chat, a change of computer and about 60k tokens; tests.
Done when: the measurement is written on the card and in this doc's §8; if kept, tests show a second question answered by the same process, grants that `begin` and `end` per message, and a reseed after New chat that holds no turn from before it; a scratch run answers ten questions from two devices in one process. If it isn't kept, this doc says why, and the card closes with that.

## 13. Not in v1

- **Moving the assistant by itself** when its computer has been away for a long time. That is a second assistant starting on a guess, which is what D3 rules out. If the person wants it, a later tune could offer the move as a one-tap notice after some hours, never do it.
- **Several conversations** (one per space or topic). There is one conversation, as today.
- **Reading the conversation from tools.** No tool reads `assistant.chat.*`, so the person's agents don't see it. Adding one is a separate privacy choice.
- **Freeing the space** of old conversation rows. The log can't be trimmed (storage.md §4). New chat only hides.

## 14. Found along the way

- `spawn_remote.rs` comments say `assistantTurn` means Claude Code is signed in. It means the `claude` command was found. Card 1 fixes the comments.
- assistants-talk.md §3 says `spawn.answers` is a kind. It isn't: it is an in-memory map of brief notes in `spawn.rs`. Fixed in that doc with this change.
- assistants-talk.md §2 says the chat runs on Haiku. Since #1014 it uses the model and effort set in Settings › Agents, default Sonnet low. Fixed in that doc with this change.
- `AGENTS_CHANGED` (`spawn_capable.rs`) is published when the computers that can run agents change, but no window code listens for it. Not this card's.
