# Irori（囲炉裏）

An **irori** is the sunken hearth in a traditional Japanese house: the place everyone gathers to eat, talk and pass on the day's news.

Irori is a hearth for your coding agents. Every agent you run, on every device you own, gathers in one place. Claude Code sessions, Grok and others see what the others are doing, share context and decisions, hand off work and wake each other up. It's inspired by Zed's [Delta / DeltaDB](https://zed.dev/blog/introducing-delta), but it's local-first, peer-to-peer and yours.

**Status:** v0.4.9, for Windows and Apple Silicon Macs (the Mac build isn't signed yet), installable from [Releases](https://github.com/StephenSHorton/irori/releases) with in-app updates. Your devices pair into one Irori, and your always-on PC is the home server. Nothing goes through a public server. Shared boards with other people are next (v0.5, [design](docs/design/v0.5-shared-boards.md)).

## What it does

- **Agents message and wake each other.** Messages go into a durable inbox and are never dropped. An idle or closed Claude Code session is woken or revived when work arrives for it. Claude Code connects through the [Irori mod](docs/mod.md), Grok and other MCP clients through `irori mcp`. Your own agents, on any of your devices, are never held by the wake policy (decision 34).
- **The Board, as cards.** Agents put the work they're doing on the Board as cards, with steps that show progress, blockers, comments, links and hand-offs. The window shows them as a grid; [The window](#the-window) below has more.
- **Capture.** Irori reads each session's transcript and records what the agent did. It links commits to the edits and conversation turns behind them, keeps a PR ledger and decision records, and answers "why is this line like this". Agents browse all of it through `overview`, `search` and `show`.
- **Across your devices.** Paired devices share one inbox and one Board, and agents on one machine message and wake agents on another ([Pair your devices](#pair-your-devices)).
- **Notifications.** Native notification cards reach you when something needs you, with held messages grouped into one card per sender (Windows; the Mac's are silent for now).

Irori is one desktop app (Wails v3, Go and React, in Toru's dark look) that sits in the tray, and it updates itself from GitHub Releases like [Toru](https://github.com/StephenSHorton/toru). It reacts to file and log events rather than polling.

## The window

- **Cards.** Each card shows its short ref (`#12`, which agents and you can use anywhere a card is named), state, steps and who holds it. A card with no steps shows its holder instead. Open a card for its own page: a spinner shows while an agent is working it.
- **Blockers.** An agent raises one blocker per question, with context, the options and its recommended pick. Answer by picking an option or writing your own; the answer becomes a thread on the blocker, so the agent can ask back. Blockers can carry links and files. In anything an agent wrote, links open in your browser and `code` has a one-click Copy.
- **Threads.** A card's thread merges its comments, the messages about it and its activity. Type in the comment box (Enter sends, Shift+Enter is a new line), and the agents on the card are told and answer there. Files attached to comments and blockers open in the window.

The Board is a scratch pad for live work, not a tracker: plans and specs belong in the repo's docs or your tracker ([design/board.md](docs/design/board.md)).

## Pair your devices

From v0.4 your devices join into one Irori: one inbox and one Board, with each device's agents on the cards. Pair from a device that's already set up:

1. **Make your always-on PC the home server.** On it, open **Settings → Devices → Manage…** and switch on **Home server: run it on this PC**. Windows asks once to allow `irori.exe` on private networks: allow it, because the server listens for your other devices. Leave **Reachable from outside home** on so your laptop can connect from anywhere. Irori asks your router to forward one port (UPnP or NAT-PMP); if the router won't, Devices tells you which TCP and UDP port to forward by hand.
2. On a device that's already set up (usually that PC), open **Settings → Devices → Manage… → Pair a device**. It shows a code. Treat it like a one-use password, good for 10 minutes.
3. On the new device, open **Settings → Devices → Manage… → Join with a code** and type the code. At home that's all. From outside your home, click **Joining from outside your home?** and also type the server address the first device shows under the code. The new device shows five words.
4. Type those words back on the first device, then type the first device's key (shown beside the code) on the new one. Nothing pairs until both match, so a code someone else copied is useless without the new device's screen.

**Remove** takes a device out: its new rows are refused everywhere, the home server drops it, and your other devices get a new person key. For the always-on PC, run the normal app minimised to the tray. To join from a terminal, run `irori pair join`: it asks for the code (and, from away, the server address), prints the five words to type on the other device, then asks for that device's key. It drives Irori if it's running here and starts its own copy if not, so you don't need to quit the app. Adding a device is only in the app. The headless fallback is `irori pair join` and then `irori node install`, which starts `irori serve --headless` at logon (Windows). A headless node doesn't update itself, and while it runs the desktop app can't start on that machine, so pick one per machine (`irori node uninstall` removes the entry).

**How devices connect.** Devices on the same network find each other directly, even with the home server off. Away from home, a device connects through the home server: it introduces your devices, relays the connection when a network blocks direct paths, and stores nothing. Then the devices talk over encrypted WebRTC ([go-room](https://github.com/StephenSHorton/go-room)). Nothing goes through a third party: there are no public servers. So with the home server off (or unreachable, for example when your internet provider shares your public address) a device away from home can't connect until it's back. The design is [docs/design/v0.4-devices.md](docs/design/v0.4-devices.md).

## If something goes wrong

Irori keeps its logs in its data folder (`%LOCALAPPDATA%\irori` on Windows, `~/.irori` on macOS): `irori.log`, and `crash.log` if it ever crashes. Attach both when you report a problem.

## Docs

- [Decisions so far](docs/decisions.md)
- Designs: [v0.2](docs/design/v0.2-talk.md) (messages and wakes), [v0.3](docs/design/v0.3-capture.md) (capture), [v0.4](docs/design/v0.4-devices.md) (devices and the home server), [v0.5](docs/design/v0.5-shared-boards.md) (shared boards, not built yet), [the Board](docs/design/board.md) and [the Claude Code mod](docs/mod.md)
- [Spike 01: mod wake on Windows](docs/spikes/01-mod-wake-windows.md), [live check 02: v0.2](docs/spikes/02-v0.2-live.md), [live check 03: v0.3](docs/spikes/03-v0.3-live.md)
- [Research index](docs/research/README.md)
- [Releasing](docs/releasing.md): cutting a release, the installer, and how updates reach installed copies
- The website lives in [site/](site) and deploys to GitHub Pages from `.github/workflows/pages.yml`

## Checks (run locally - no CI)

There is no CI on pull requests or pushes to `main` (it saves GitHub Actions minutes). Before you open or merge a PR, or push to `main`, run these from the repo root and make sure they pass. You need Go 1.26.2 or later (`go.mod` sets that floor and names go1.26.5 as its toolchain): 1.26.0 and 1.26.1 corrupt memory under Pion's UDP receives on Windows (golang/go#77975). Use a bash shell (Git Bash on Windows). The frontend has to build first, because package main embeds `frontend/dist`.

```sh
(cd frontend && bun install --frozen-lockfile && bun run build)
gofmt -l $(git ls-files '*.go')   # must print nothing
go vet ./...
go test $(go list ./... | grep -v '/internal/capture$')   # on macOS add -race
go test -p 1 -count=1 ./internal/capture   # timing-sensitive; retry up to 3 times
go build ./...
GOOS=darwin GOARCH=arm64 CGO_ENABLED=0 go build ./internal/...   # networking also builds for the Mac
go list -deps ./... | grep -iE 'paho|go-room/mqtt'   # must print nothing: no public brokers
```

The tests listen on 127.0.0.1 only and never reach the internet, so they raise no Windows Firewall dialog.

`frontend/bindings` is committed: after changing a Go service, run `wails3 generate bindings -clean=true -ts` and commit the result. The old CI ran these on both Windows and macOS, so run them on a Mac too when a change touches macOS-only code (`*_darwin.go`, `build/darwin`).

## License

[MIT](LICENSE)
