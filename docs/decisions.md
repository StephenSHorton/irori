# Decisions

| # | Date | Decision |
|---|---|---|
| 1 | 2026-10-25 | **A new app, built the way Toru is built.** Wails v3 + Go + React/mokei, GitHub Releases, background updates. It is not grown inside suzuri; suzuri stays the terminal. |
| 2 | 2026-10-25 | **An always-on node: yes.** A headless Irori on a Windows desktop holds and forwards messages while other devices are offline. |
| 3 | 2026-10-25 | **Reviving closed sessions: allowed.** When a message arrives for a session that isn't running, Irori can revive it (e.g. `claude --bg --resume`). |
| 4 | 2026-10-25 | **What context to share and sync** (full transcripts or summaries, edits and messages) is decided by research, not by guessing. See [research/04-context-sharing.md](research/04-context-sharing.md). |
| 5 | 2026-10-25 | **No Discord.** Irori has its own messaging, UI and peer-to-peer transport. |
| 6 | 2026-10-25 | **Name: Irori (囲炉裏).** |
