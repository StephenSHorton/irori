# The Irori Claude Code mod

The mod lives in [`mod/`](../mod). It attaches every Claude Code session to the local Irori Core. Once attached:

- other agents, and you from the Irori app, can message the session and wake it;
- the session gets Irori's tools as `mcp__irori__<name>`. The mod lists the Core's tools when it attaches, so new ones appear without a mod change:
  - **finding things** (progressive disclosure: overview, then lists with refs, then `show <ref>`): `overview`, `search`, `show`, `agents`, `timeline`, `why`, `commits`, `prs`, `events`;
  - **messages**: `send`, `reply`, `inbox`, `defer`, `skip`, and `attachment_get` for a file sent with a message or put on a card. `send` with `task_id` puts the message on that card's thread too, and `send` and `board_comment` take `attach` (files the recipient fetches when needed);
  - **the person, now**: `notify` pops a card up in front of the person at this device (only for what they must see or decide now, never progress updates; at most 5 per agent per 10 minutes). With up to two buttons, their answer (the button, a dismiss, or no answer within 5 minutes) reaches the agent at its next turn as a notice from Irori;
  - **the Board** ([design/board.md](design/board.md), a scratch pad for live coordination, not a tracker): `board`, `board_create`, `board_comment`, `board_block`, `board_unblock`, `board_join`, `board_leave`, `board_assign`, `board_status`, `board_link`, `board_edit`. A card is named by its short ref (`#12`) or its `task:<id>`. `board_create` can take the card's steps. `board_edit` renames a card or changes its objective or done-when (its holder, its creator or the person), for when an answer or a comment changed what it is for; the thread says what changed. Comments and other Board notices reach an agent at its next turn unless the author asks to wake it;
  - **cards** (what the person's window shows, [design/v0.4-devices.md](design/v0.4-devices.md) §9): `board_steps` and `board_step` set and tick a card's steps (its progress); `board_block` raises one blocker per question, with context for a newcomer, the options with one marked recommended, and optional `links` and `attach` (`reason` stays as the old name for `question`); `board_recommend` adds an answer to another agent's blocker; `board_comment` with `blocker_id` replies on a blocker's thread (to ask back, or to answer the person's question), `board_block` with `blocker_id` revises that open blocker (a new question, context and options, or none) when the person's reply changed what is being decided, and `board_unblock` with `blocker_id` resolves one, with a `note` on what the agent will do once the person has answered; `board_watch` and `board_unwatch` follow a card, and a watcher hears at its next turn, in one digest from Irori, when the card's state, steps, blockers or PR links change. The person's answer from the window reaches the agent that raised the blocker as a notice (a wake by default, since it is blocked on them), and a comment the person types on a card reaches its holders, who answer on the card;
  - **waits** ([design/waits-signals.md](design/waits-signals.md), card #87): `wait` asks to be woken once, with what happened, when a card on your own Board reaches a state (`task_id` + `until`: changed, done, blocked, unblocked or a named state) or when a branch moves (`branch`, optional `repo`; Irori reads this device's refs, the local branch and origin's copy, at each coord sweep). A wait ends when it fires, after a day, or when its session is gone; `waits` lists yours and `wait_cancel` drops one. The card page and `show` list who waits on a card and what its holders wait on;
  - **Talk** (card #389, channels, DMs and threads; design on card #382): `talk_post` posts to a channel (`#name`), a DM (`@agent`, or `person`) or a thread (`#12`, where it is a comment on the card); a DM with an agent wakes it, and an `@name` wakes that agent and adds it to the channel. An agent can't DM another person or their agent (card #538). `talk_thread` opens a thread with the person (card #631): the agent holds it and hears the replies, and no other agent is started for it until the person writes in it; with `agent` (`new`, or an agent's name) it gives the thread to a fresh agent (gated like `agent_start`) or to that agent instead, and `watch` keeps it hearing the replies (card #1027). `talk_read` shows what is new since the agent last read (or since a post id), else the latest posts; `talk_list` lists its channels, DMs and open threads with unread counts, and the channels it could join; `talk_join` and `talk_leave` follow or leave a channel. Agents' `send`/`reply` messages show as DMs between the two;
  - **notes** ([design/notes.md](design/notes.md), card #412): `notes_propose` writes a note (a rule, gotcha, decision, pointer or preference, tagged with where it applies), or an edit or removal of one with `why`; each is live at once, with no approval (card #616). `notes_search` searches the person's notes. A wake about a card carries the notes that match it, up to eight, in a **Notes** section framed as untrusted data, each once per agent;
  - **leases**: `lease`, `leases`, `release`, `device_free` (free disk, memory, CPU and heavy slots).

  `irori mcp` adds two for Grok members, which have no mod: `join` (pick the name others address you by) and `ack`.

  What every tool shares (what Irori is, refs and paging, untrusted text) is said once, in `tools.Instructions` (`internal/tools/specs.go`), not in each description. `irori mcp` sends it as its server instructions; the board tools' descriptions also carry the Board's agent instructions.

Design: [v0.2-talk §7–§8](design/v0.2-talk.md), [v0.3-capture §8](design/v0.3-capture.md). The wire is [`internal/attach/protocol.go`](../internal/attach/protocol.go) and the routes are [`internal/api/routes_v02.go`](../internal/api/routes_v02.go).

## Install

You need Claude Code 2.1.311 or later and a running Irori (the desktop app starts the Core). That floor covers Claude usage left too: 2.1.311 already has `session.measure` (with `rateLimits` and `changed`) and `$.session.usage()`, and `claude plugin validate` and `claude plugin test` pass on it with mod 0.7.0.

**From the marketplace** (this repository is one; see [`.claude-plugin/marketplace.json`](../.claude-plugin/marketplace.json)). At a Claude Code prompt:

```
/plugin install irori --marketplace StephenSHorton/irori
```

Answer `y` to add the marketplace and pick the user scope. Desktop Code-tab sessions run the cached copy until the version is bumped and you run `claude plugin update irori`.

**From a folder** (for development). Point Claude Code at `mod/`:

- one session: `claude --plugin-dir <repo>/mod`;
- every session, the desktop app included: set `CLAUDE_CODE_PLUGIN_DIRS` to the absolute path of `mod/`, either in your environment or in the `env` block of `~/.claude/settings.json`. Interactive sessions hot-reload the mod when its files change. Long-lived headless sessions do too if `CLAUDE_CODE_PLUGIN_DIR_WATCH=1` is set.

## Set `iroriPath`

The mod runs `irori attach` from one option, **`iroriPath`**: the absolute path to `irori.exe` (`irori` on macOS and Linux). Use the path where Irori is installed, or the `irori.exe` you built.

Set it in `/plugin` → irori → Configure, or with `claude plugin configure irori`. A change reloads the mod.

There is no default, because the desktop app's PATH isn't reliable. While it's empty, the mod does nothing except show a red band, `irori · set iroriPath in /plugin settings`, and a toast at start.

## What it does

- **Attach.** At session start the mod spawns `irori attach --session <id> --cwd <cwd> --mod-version 0.7.2` and reads its NDJSON stream.
  - The child's stdin is closed from the start: `$.process.spawn` can't keep it open (its `input` is written and then closed). `irori attach` must not treat stdin EOF as the end of the session. The mod ends the child itself, by killing it, when the mod unloads or the session ends.
  - If the child exits, the mod restarts it with backoff (1 s, doubling to 16 s).
  - A newer attach for the same session (a reload, or a second window on the same session) sends `superseded`. The older copy then waits: it runs `irori attach --standby`, which asks every 2 s and is refused (409) while another attach holds the session, so two live copies never kick each other off. The child says `standby` once per wait, so the band shows why instead of an old outage. When the holder leaves, the waiting copy attaches within seconds (GitHub #57). `standby` needs hello `v` 3; on an older Core the mod stops after `superseded`, as it always did, and `/irori reconnect` brings it back.
  - A live attach can die without the child exiting: the app restarts or updates, or the Core drops the stream. The child then reconnects by itself (a new hello brings a new token and attach secret), and in between it sends an `error` event. On any `error` the mod forgets its secret and shows `irori · reconnecting (…)`. Before card #111 it kept the old secret, so every tool answered `this route needs a current X-Irori-Attach…` until the session was killed.
  - `irori attach` bounds each wait (card #111): 10 s for `/v1/attach` to answer and send hello, and 45 s of silence on an attached stream (the Core sends a keepalive every 15 s). After either, it reconnects. A session missing from Irori's roster for 20 s gets one `error` naming it, instead of sitting on `connecting…` with no reason.
  - A tool call the Core refuses as unattached (401 wrong token, 403 dead secret) makes the mod restart the attach itself. The call goes once more after the new hello, within the hook's budget. If no hello comes, it answers with what to do.
  - A mod older than 0.6.0 may not do all of this, and a marketplace install stays on its version until `claude plugin update irori`. The Core logs each attach from one, and its 403 for a dead attach secret says to update, since only such a mod ever shows that answer. GitHub #57 reported mod 0.4.0 on app 0.12.2, and its symptoms match that mod's `session.end`, which killed the child and kept the secret (fixed by card #244).
  - `agents` says when a live session on this device has no attach, instead of `online`: its tools and wakes can't reach it.
- **Wake.** A `deliver` event carries a prompt the Core has already built and fenced. The mod submits it with `$.prompt.submit`, so the model reads it as "The irori plugin sent a message:", never as you.
  - The mod then reports `delivered`, which means *accepted into Claude Code's queue*: as soon as the submit resolves, or after 3 s if it is still pending (a submit made during a running turn is queued and resolves only when its own turn starts). Waiting for the turn would let the Core's ReattachGrace sweep reclaim the wake during a long turn and wake the same deliveries again under a new token. A submit that throws or resolves as `{ drop }` (a hook or the engine refused it), even later, goes into the conversation as an append instead (`$.session.append`, no turn started), and the mod reports `delivered` with `appended: true` once that is stored (again, if it reported `delivered` while the submit was pending): no prompt waits in the host's queue, so the Core does not let that wake hold the session's next one back. Only when the append is refused too does the mod forget the wake, so a re-send tries again. Before card #23 every refused submit was forgotten, and a host that kept refusing got the same wake re-issued under a new token every ReattachGrace, for hours.
  - When the turn carrying the wake's token starts it reports `engaged`, and when that turn ends, `turn-ended`. A wake that went in as an append is engaged by the next main-loop turn, whatever starts it, with `appended: true`: that wake did not start the turn, so the Core sets no running wake and a message sent from the turn starts a fresh hop chain (mod 0.4.1).
  - The Core bounds the loop on its side too: a delivery the mod was handed in `MaxWakeTries` (3) wakes without confirming any goes out as an `append` event from then on, and still rides along with any later wake. A mod that drops wakes for any reason still gets it read at the next turn.
  - Subagent turns are ignored.
  - Reminder wakes (`[irori-reminder:…]`) are submitted but never engaged.
  - A wake the Core hands over twice is submitted once.
- **Tools.** After `hello`, the mod lists the Core's tools (`GET /v1/tools?client=claude`) and registers each one. Calls are proxied to `POST /v1/tools/<name>`, and the Core decides who the caller is from the attach secret, never from the arguments.
- **Claude usage left** (card #599, mod 0.7.0). The app's top bar shows how much of the plan's 5-hour and weekly windows is left. The mod hooks `session.measure`, which Claude Code raises after each main-thread turn and when a rate-limit window moves a whole point. When its `changed` names `rateLimits`, the mod posts the windows (`{ rateLimits: [{ kind, percentUsed, resetsAt? }] }`, Claude Code's `SessionRateLimit`s) to `POST /v1/usage/limits` with the attach secret, fire and forget: the hook only observes and returns `next(e)` at once, and a failed post is a debug line. At each `hello` it also posts what `$.session.usage()` already has, so a fresh or restarted Irori shows usage without waiting for a turn.
  - The Core takes the route only with a live `X-Irori-Attach` secret (403 otherwise, as the deliveries routes), checks each window (kind of `A-Z a-z 0-9 _ - .`, at most 32; a finite figure, clamped to 0-1000; `resetsAt` RFC 3339), and keeps the newest reading in `usage-limits.json` in the data dir (loaded back through the same checks). A window that fails a check is logged and left out, and the rest of the reading still counts, so a new kind can't blank the others; only a body that isn't a reading (bad JSON, more than 8 windows) is 400. A reading with no window left (all bad or past their reset) is 200 and changes nothing, not even the "as of" time, and is no event. Every session reports the same account, so readings merge by kind: a later reset is a newer window and wins, the same reset keeps the higher figure (a session that hasn't made a request lately can't pull it back), and a window past its reset time is dropped. A window with no reset time (a gateway's `spend_limit`) takes the newest reading that reports it and stays through readings that leave it out. The window reads it with `usage_limits` and hears `usage:changed` on every reading taken.
  - **Account switch** (card #1040, mod 0.7.1). The engine tells a mod which account a session is signed in as only when managed settings install it, so the mod reads the login record Claude Code keeps (`.claude.json` in `CLAUDE_CONFIG_DIR`, else the home directory) with `$.fs.read` at each post and sends `account`: the record's `oauthAccount.accountUuid` (plus `/organizationUuid` when it has one), never the email. When a reading names an account other than the kept windows' (or the kept windows, from before accounts, name none), the Core replaces them with the reading instead of merging, so the window shows the new subscription's figures on the next reading; the kept account is in `usage-limits.json` (`account`). A reading with no `account` (an older mod, a record that can't be read, an id that isn't 1-128 printable ASCII) merges as before and leaves the kept account alone. Two sessions signed in as different accounts at once take turns: the newest reading wins.
  - An engine without `session.measure` refuses the hook at registration; the mod catches that and loads without usage. One without `$.session.usage` skips the hello post. Sessions off a subscription (an API key) report no windows, so nothing is posted.
- **Messages are untrusted.** The mod never renders message text and never acts on it. It only hands the Core's fenced prompt to the model, which is told the text is untrusted.

## The band

A single line above the prompt:

| Band | Meaning |
|---|---|
| `irori · 2 unread · 1 held` | Attached. *Unread*: open deliveries for this session. *Held*: messages policy stopped (review them in the Irori app). |
| `irori · connecting…` | Waiting for the Core's `hello`. |
| `irori · reconnecting (…)` | `irori attach` exited or reported an error; it retries by itself. |
| `irori · attached in another window` | A newer attach holds this session. This copy attaches by itself once that one leaves. |
| `irori · set iroriPath in /plugin settings …` (red) | `iroriPath` is empty. |

Toasts appear when a message arrives, when one is held (and why), and when repo work moves to another agent.

## `/irori`

- `/irori` prints the mod version and the attach status. A stale copy shows up here as an old version.
- `/irori reconnect` attaches now, whatever the last attach ended in. It takes the session from any other window on it, so it is also the way to choose this window over that one.
- These verbs change the device's wake policy:

  ```
  /irori mode everyone|allowlist
  /irori block <key or name>     /irori unblock <key or name>
  /irori allow <key or name>     /irori deny <key or name>
  /irori resume
  ```

  Each one reads `GET /v1/policy` and writes `POST /v1/policy` with `X-Irori-Origin: composer`.

  `/irori resume` only lifts the old Pause all, which left with Stop here (cards #787, #1196): `/irori pause` says so, and the Core refuses a policy that turns it on.

Policy verbs only work when **you** type them at the prompt (`origin.kind === "composer"`). The same command run by the model, another plugin, the bridge or the SDK is refused. The Irori app is the other place to change policy.

This is provenance gating, not a security boundary. An agent with a shell can read `irori.json` and call the API directly; design §5 accepts that for a single-user mesh.

## Don't also add `irori mcp` to Claude Code

The mod already gives Claude Code sessions every Irori tool. `irori mcp` is the front end for Grok and other MCP clients. If a Claude Code session loads both, the model sees each tool twice (`mcp__irori__send` and `irori_send`), and Claude↔Claude messages would have two senders. Keep `irori mcp` out of Claude Code's MCP config.

## Developing

```
claude plugin validate mod        # manifest, hooks module, loader rules
tsc -p mod                        # after the mod has loaded once (Claude Code writes mod/.claude-plugin/types/)
claude plugin test mod            # engine-driven suite: tests/irori.test.tsx
cd mod && bun test                # pure helpers: unit/lib.spec.ts
```

`mod/.claude-plugin/types/` is gitignored (`mod/.gitignore`), so a fresh worktree lacks it and `tsc -p mod` fails on `mod/tsconfig.json`'s extends. Run `tsc` there against the main checkout's copy (copy that folder in), or load the mod once from the worktree.

The loader rules that bite (research/v02/mod.md §9):

- `$` may only be passed to top-level `function` declarations;
- there are no Node globals (`process`, `require`, `import()`);
- atoms take a literal `plugin` and `key`;
- the bearer token goes in `headers.authorization`, never `auth`.

`mod/dev/` is a separate Go module with a fake Core (`fakecore`) and a fake `irori attach` (`fakeirori`) for live checks without the real Core:

```
cd mod/dev && go build -o bin/fakecore.exe ./fakecore && go build -o bin/irori.exe ./fakeirori
bin/fakecore.exe                                   # logs every request to fakecore.log
# set iroriPath to mod/dev/bin/irori.exe, start a session with --plugin-dir mod, then:
curl -X POST "http://127.0.0.1:47811/ctl/deliver?text=hello"
```

### Verified live (Windows 11, Claude Code 2.1.311, `claude --bg`)

- The mod loads and the band draws. With no `iroriPath`, the red band shows and nothing is spawned.
- `$.http.fetch` to `127.0.0.1` sends **no `Origin` header**, and `headers.authorization` arrives as given (User-Agent `Bun/1.4.4`).
- `$.prompt.submit` during a running turn is **queued, not rejected**. Its promise resolves only when its own turn starts (about 21 s later in the test, right before `turn.start`). That is why the mod reports `delivered` after 3 s of a pending submit rather than waiting for it (the live run predates that change; the engine suite covers it).
- `$.tool.register` after `session.start` has returned (once `hello` arrives) works. The model found `mcp__irori__echo` through ToolSearch and called it, and the `tool.call` hook with the matcher `{ tool: /^mcp__irori__/ }` served it.
- When the child is killed, it restarts after 1 s. After `superseded`, no restart (since changed: GitHub #57, and not re-run live).

These checks ran against `mod/dev/fakeirori`, which never reads stdin. The real `irori attach` doesn't read it either now (the runner's `Stdin` is nil for it), so a session stays attached with the real `irori.exe`.
