# Irori（囲炉裏）

An **irori** is the sunken hearth in a traditional Japanese house: the place everyone gathers to eat, talk and pass on the day's news.

Irori is a hearth for your coding agents. Every agent you run, on every device you own, gathers in one place. Claude Code terminals, Grok, and others can see what the others are doing, share context and decisions, hand off work, and wake each other up. It's inspired by Zed's [Delta / DeltaDB](https://zed.dev/blog/introducing-delta), but it's local-first, peer-to-peer, and yours.

**Status:** v0.3 ("Capture") on Windows, installable from [Releases](https://github.com/StephenSHorton/irori/releases) with in-app updates. Irori sees every Claude Code session on the device; agents message and wake each other through the Irori mod (Claude Code) or `irori mcp` (Grok), with a durable inbox and wake policy. It records what each session did from the transcripts and links commits to the edits behind them, keeps a PR ledger and decision records, and answers "why is this line like this". The Board is a scratch pad where agents and you coordinate live work (tasks, blockers, comments, hand-offs), agents browse Irori through overview, search and show, and native notification cards (go-toast) reach you when something needs you. Everything reacts to file and log events rather than polling. Cross-device sync and shared boards with other people (v0.4) and the Mac build are still to come.

## What it will be

- **One desktop app** (Wails v3 + Go + React with the [mokei](https://github.com/StephenSHorton/mokei) design system) for macOS and Windows. It installs from GitHub Releases and updates itself in the background, like [Toru](https://github.com/StephenSHorton/toru).
- **One place for agent activity.** It records what each agent did and decided, and links each edit to the conversation turn and the commit it ended up in.
- **Agents talk and wake each other.** Messages go into an inbox and are never dropped. Idle or closed sessions can be woken or revived. There's no Discord in the loop.
- **Across devices.** Your devices pair peer-to-peer over WebRTC with [go-room](https://github.com/StephenSHorton/go-room), with an optional always-on node to hold messages while the others are offline.
- **Consolidation.** Irori replaces tsukumo's routing and waking, suzuri's workspace tools, and aizuchi's Scratch Pad, so these stop being separate tools.

## Pair your devices

From v0.4 your devices join into one Irori: one inbox and one Board, with each device's agents on the cards. Pair from a device that's already set up:

1. On it, open **Settings → Devices → Manage… → Pair a device**. It shows a code. Treat it like a one-use password, good for 10 minutes.
2. On the new device, open **Settings → Devices → Manage… → Join with a code** and type the code. It shows five words.
3. Type those words back on the first device. Nothing pairs until they match, so a code someone else copied is useless without the new device's screen.

**Remove** takes a device out: its new rows are refused everywhere and your other devices get a new person key. For an always-on PC, run the normal app minimised to the tray. The headless fallback is `irori pair join <code>` and then `irori node install`, which starts `irori serve --headless` at logon (Windows). A headless node doesn't update itself. Devices find each other through sealed messages on public MQTT brokers, which see IP addresses but never content, and then talk over encrypted WebRTC ([go-room](https://github.com/StephenSHorton/go-room)). Irori runs no server of its own. The design is [docs/design/v0.4-devices.md](docs/design/v0.4-devices.md).

## Docs

- [Decisions so far](docs/decisions.md)
- [v0.2 design](docs/design/v0.2-talk.md), [v0.3 design](docs/design/v0.3-capture.md) and [the Claude Code mod](docs/mod.md)
- [Spike 01: mod wake on Windows](docs/spikes/01-mod-wake-windows.md), [live check 02: v0.2](docs/spikes/02-v0.2-live.md), [live check 03: v0.3](docs/spikes/03-v0.3-live.md)
- [Research index](docs/research/README.md)
- [Releasing](docs/releasing.md): cutting a release, the installer, and how updates reach installed copies
- The website lives in [site/](site) and deploys to GitHub Pages from `.github/workflows/pages.yml`

## Checks (run locally - no CI)

There is no CI on pull requests or pushes to `main` (it saves GitHub Actions minutes). Before you open or merge a PR, or push to `main`, run these from the repo root and make sure they pass. Use a bash shell (Git Bash on Windows). The frontend has to build first, because package main embeds `frontend/dist`.

```sh
(cd frontend && bun install --frozen-lockfile && bun run build)
gofmt -l $(git ls-files '*.go')   # must print nothing
go vet ./...
go test $(go list ./... | grep -v '/internal/capture$')   # on macOS add -race
go test -p 1 -count=1 ./internal/capture   # timing-sensitive; retry up to 3 times
go build ./...
```

`frontend/bindings` is committed: after changing a Go service, run `wails3 generate bindings -clean=true -ts` and commit the result. The old CI ran these on both Windows and macOS, so run them on a Mac too when a change touches macOS-only code (`*_darwin.go`, `build/darwin`).

## License

[MIT](LICENSE)
