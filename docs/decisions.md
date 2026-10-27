# Decisions

| # | Date | Decision |
|---|---|---|
| 1 | 2026-10-25 | **A new app, built the way Toru is built.** Wails v3 + Go + React/mokei, GitHub Releases, background updates. It is not grown inside suzuri; suzuri stays the terminal. |
| 2 | 2026-10-25 | **An always-on node: yes.** A headless Irori on a Windows desktop holds and forwards messages while other devices are offline. |
| 3 | 2026-10-25 | **Reviving closed sessions: allowed.** When a message arrives for a session that isn't running, Irori can revive it (e.g. `claude --bg --resume`). |
| 4 | 2026-10-25 | **What context to share and sync:** researched in [research/04-context-sharing.md](research/04-context-sharing.md). Recommendation, not yet signed off: replicate structured events plus a redacted, stripped turn index everywhere. Fetch full transcripts, tool output and subagent bodies on demand from the device that recorded them. Sharing or forking full context is an explicit action. Nothing raw is injected into an agent. |
| 5 | 2026-10-25 | **No Discord.** Irori has its own messaging, UI and peer-to-peer transport. |
| 6 | 2026-10-25 | **Name: Irori (囲炉裏).** |
| 7 | 2026-10-26 | **Waking Claude Code through the mod works on Windows.** [Spike 01](spikes/01-mod-wake-windows.md): `$.process.spawn` streams a helper's stdout and `$.prompt.submit` starts a turn in a `--bg` session, which shows it as a plugin message. The mod path is the primary wake; `--resume` stays the fallback for closed sessions. macOS is still to be checked. |
| 8 | 2026-10-26 | **One binary, `irori`:** the desktop app by default, `serve --headless` for the always-on node, `mcp` for agents, `attach` for the mod's stream. Agents find the running instance through `irori.json` in the data dir (`%LOCALAPPDATA%\irori`, `~/.irori`). Wails v3 is pinned at alpha.98 to match Toru; v3.0.0 stable shipped 2026-10-26 and is a separate upgrade. |
