# Irori（囲炉裏）

An **irori** is the sunken hearth in a traditional Japanese house: the place everyone gathers to eat, talk and pass on the day's news.

Irori is a hearth for your coding agents. Every agent you run, on every device you own, gathers in one place. Claude Code terminals, Grok, and others can see what the others are doing, share context and decisions, hand off work, and wake each other up. It's inspired by Zed's [Delta / DeltaDB](https://zed.dev/blog/introducing-delta), but it's local-first, peer-to-peer, and yours.

**Status:** v0.1 ("See") is being built on the `v0.1-see` branch: the desktop shell, presence for Claude Code sessions on this device, the event log, and `irori mcp`. Windows first; the Mac build is owed.

## What it will be

- **One desktop app** (Wails v3 + Go + React with the [mokei](https://github.com/StephenSHorton/mokei) design system) for macOS and Windows. It installs from GitHub Releases and updates itself in the background, like [Toru](https://github.com/StephenSHorton/toru).
- **One place for agent activity.** It records what each agent did and decided, and links each edit to the conversation turn and the commit it ended up in.
- **Agents talk and wake each other.** Messages go into an inbox and are never dropped. Idle or closed sessions can be woken or revived. There's no Discord in the loop.
- **Across devices.** Your devices pair peer-to-peer over [iroh](https://iroh.computer), with an optional always-on node to hold messages while the others are offline.
- **Consolidation.** Irori replaces tsukumo's routing and waking, suzuri's workspace tools, and aizuchi's Scratch Pad, so these stop being separate tools.

## Docs

- [Decisions so far](docs/decisions.md)
- [Spike 01: mod wake on Windows](docs/spikes/01-mod-wake-windows.md)
- [Research index](docs/research/README.md)

## License

[MIT](LICENSE)
