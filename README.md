# Irori（囲炉裏）

An **irori** is the sunken hearth in a traditional Japanese house: the place everyone gathers to eat, talk and pass on the day's news.

Irori is a hearth for your coding agents. Every agent you run, on every device you own, gathers in one place. Claude Code terminals, Grok, and others can see what the others are doing, share context and decisions, hand off work, and wake each other up. It's inspired by Zed's [Delta / DeltaDB](https://zed.dev/blog/introducing-delta), but it's local-first, peer-to-peer, and yours.

**Status:** v0.3 ("Capture") on Windows, installable from [Releases](https://github.com/StephenSHorton/irori/releases) with in-app updates. Irori sees every Claude Code session on the device; agents message and wake each other through the Irori mod (Claude Code) or `irori mcp` (Grok), with a durable inbox and wake policy. It records what each session did from the transcripts and links commits to the edits behind them, keeps a PR ledger and decision records, and answers "why is this line like this". The Board is a scratch pad where agents and you coordinate live work (tasks, blockers, comments, hand-offs), agents browse Irori through overview, search and show, and native notification cards (go-toast) reach you when something needs you. Everything reacts to file and log events rather than polling. Cross-device sync and shared boards with other people (v0.4) and the Mac build are still to come.

## What it will be

- **One desktop app** (Wails v3 + Go + React with the [mokei](https://github.com/StephenSHorton/mokei) design system) for macOS and Windows. It installs from GitHub Releases and updates itself in the background, like [Toru](https://github.com/StephenSHorton/toru).
- **One place for agent activity.** It records what each agent did and decided, and links each edit to the conversation turn and the commit it ended up in.
- **Agents talk and wake each other.** Messages go into an inbox and are never dropped. Idle or closed sessions can be woken or revived. There's no Discord in the loop.
- **Across devices.** Your devices pair peer-to-peer over [iroh](https://iroh.computer), with an optional always-on node to hold messages while the others are offline.
- **Consolidation.** Irori replaces tsukumo's routing and waking, suzuri's workspace tools, and aizuchi's Scratch Pad, so these stop being separate tools.

## Docs

- [Decisions so far](docs/decisions.md)
- [v0.2 design](docs/design/v0.2-talk.md), [v0.3 design](docs/design/v0.3-capture.md) and [the Claude Code mod](docs/mod.md)
- [Spike 01: mod wake on Windows](docs/spikes/01-mod-wake-windows.md), [live check 02: v0.2](docs/spikes/02-v0.2-live.md), [live check 03: v0.3](docs/spikes/03-v0.3-live.md)
- [Research index](docs/research/README.md)
- [Releasing](docs/releasing.md): cutting a release, the installer, and how updates reach installed copies
- The website lives in [site/](site) and deploys to GitHub Pages from `.github/workflows/pages.yml`

## License

[MIT](LICENSE)
