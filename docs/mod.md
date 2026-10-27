# The Irori Claude Code mod

The mod lives in [`mod/`](../mod). It attaches every Claude Code session to the local Irori Core. Once attached:

- other agents, and you from the Irori app, can message the session and wake it;
- the session gets Irori's tools as `mcp__irori__<name>`: `send`, `reply`, `inbox`, `defer`, `skip`, `agents`, `events`, tasks and leases.

Design: [v0.2-talk §7–§8](design/v0.2-talk.md). The wire is [`internal/attach/protocol.go`](../internal/attach/protocol.go) and the routes are [`internal/api/routes_v02.go`](../internal/api/routes_v02.go).

## Install

You need Claude Code 2.1.311 or later and a running Irori (the desktop app starts the Core).

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

- **Attach.** At session start the mod spawns `irori attach --session <id> --cwd <cwd> --mod-version 0.2.0` and reads its NDJSON stream.
  - The child's stdin is closed from the start: `$.process.spawn` can't keep it open (its `input` is written and then closed). `irori attach` must not treat stdin EOF as the end of the session. The mod ends the child itself, by killing it, when the mod unloads or the session ends.
  - If the child exits, the mod restarts it with backoff (1 s, doubling to 16 s).
  - A newer attach for the same session (a reload, or a second window on the same session) sends `superseded`. The older copy then stops quietly.
- **Wake.** A `deliver` event carries a prompt the Core has already built and fenced. The mod submits it with `$.prompt.submit`, so the model reads it as "The irori plugin sent a message:", never as you.
  - The mod then reports `delivered`, which means *accepted into Claude Code's queue*: as soon as the submit resolves, or after 3 s if it is still pending (a submit made during a running turn is queued and resolves only when its own turn starts). Waiting for the turn would let the Core's ReattachGrace sweep reclaim the wake during a long turn and wake the same deliveries again under a new token. A submit that fails later still makes the mod forget the wake, so a re-send submits it again.
  - When the turn carrying the wake's token starts it reports `engaged`, and when that turn ends, `turn-ended`.
  - Subagent turns are ignored.
  - Reminder wakes (`[irori-reminder:…]`) are submitted but never engaged.
  - A wake the Core hands over twice is submitted once.
- **Tools.** After `hello`, the mod lists the Core's tools (`GET /v1/tools?client=claude`) and registers each one. Calls are proxied to `POST /v1/tools/<name>`, and the Core decides who the caller is from the attach secret, never from the arguments.
- **Messages are untrusted.** The mod never renders message text and never acts on it. It only hands the Core's fenced prompt to the model, which is told the text is untrusted.

## The band

A single line above the prompt:

| Band | Meaning |
|---|---|
| `irori · 2 unread · 1 held` | Attached. *Unread*: open deliveries for this session. *Held*: messages policy stopped (review them in the Irori app). |
| `irori · connecting…` | Waiting for the Core's `hello`. |
| `irori · reconnecting (…)` | `irori attach` exited or reported an error; it retries by itself. |
| `irori · set iroriPath in /plugin settings …` (red) | `iroriPath` is empty. |

Toasts appear when a message arrives, when one is held (and why), and when repo work moves to another agent.

## `/irori`

- `/irori` prints the mod version and the attach status. A stale copy shows up here as an old version.
- These verbs change the device's wake policy:

  ```
  /irori mode everyone|allowlist
  /irori block <key or name>     /irori unblock <key or name>
  /irori allow <key or name>     /irori deny <key or name>
  /irori pause                   /irori resume
  ```

  Each one reads `GET /v1/policy` and writes `POST /v1/policy` with `X-Irori-Origin: composer`.

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
- When the child is killed, it restarts after 1 s. After `superseded`, no restart.

These checks ran against `mod/dev/fakeirori`, which never reads stdin, not the real `irori attach`. The real runner as of w2/api exits when stdin closes, so with it a session restarts forever and never stays attached. That needs the runner changed (see the stdin note under **Attach**); the attach, restart and `superseded` checks are owed again with the real `irori.exe` after it is.
