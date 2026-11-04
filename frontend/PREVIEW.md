# Browser preview

See the window in a browser on made-up data, without starting a second Irori.
Nothing here talks to the Go side, your settings, your devices or the
installed app.

```sh
cd frontend
bun install --frozen-lockfile   # once
bun run preview:look            # http://127.0.0.1:9246/
```

It binds 127.0.0.1 only (no firewall prompt) and uses port 9246, so it never
collides with `wails3 dev` on 9245. Stop it with Ctrl+C.

## How it works

Components reach the Go side only through `src/lib/*.ts`. Outside a Wails
webview (`inWails()` false) each of those runs on its built-in sample instead
of the bindings: `sample-cards.ts`, `sample.ts` (roster), `sample-talk.ts`
(messages), and the samples in `devices.ts` and `settings.ts`. Buttons work on
that sample in memory. Inside the app none of it runs, and `bun run build` is
unchanged.

## Open a state from the URL

| URL | Shows |
| --- | --- |
| `/` | the open cards; agents are busy on sample-card-2 and sample-card-4 (spinners) |
| `/?open=board:task:sample-card-4` | a card's page: busy agent, spinning step, links, a file |
| `/?talk` | Talk (card #390): the Threads list beside the new-thread box; `#talk` in the address does the same |
| `/?thread=sample-thread-1` | Talk with a thread open: sample-heron's answer, the reply box, Resolve (cards #116, #390); its #26 (a resolved thread) and #5 (a done card) are links (card #144) |
| `/#talk/sample-ch-suzuri` | a Talk channel (card #390): posts by day, a mention, the pointer a thread started from a message leaves, members with live status; hover a message for Start a thread and Make a card |
| `/#talk` at phone width (under 768 px) | the phone (card #395, mockup talk-5): Home, Talk and Boards as a bottom tab bar, Talk's count on its tab; a conversation opens full-screen with Back, its members behind the channel's name; long-press (or right-click) a message for Reply in thread and Make a card |
| `/#talk/sample-dm-heron` | a DM with an agent (unread until opened); `/#talk/sample-dm-otter-badger` two agents' DM, read-only |
| `/#talk/sample-dm-gone-3` | an archived DM open (card #473): gone agents' DMs, read or not, sit in the Archive at the bottom, which folded lists no rows, only its count, how many are new and Mark all read |
| `/#talk/sample-dm-spencer` | a DM with another member's agent on a shared board (card #478): "BillingEng1 · Spencer", "Spencer's agent · idle", never its `board:<board>/<card>` key; with your DMs, not the Archive |
| `/#talk/sample-dm-spencer-2` | another member's agent whose state this device doesn't know (card #478): "an agent of Spencer's", only "Spencer's agent" under it, no live dot, not gone |
| `/#talk/sample-dm-owl-3` | Agents talking (card #486): sample-owl's message to 16 agents is one row, "sample-owl → 15 agents", unfolded to its recipients because one of them is open; tansy answered, so its DM is its own row. Every row says its last line, with the time beside the name; the section lists the newest 8, up to 50 with Show more |
| `/?open=board:task:sample-card-7` | a parent card's page: its subtasks, one blocked, and the steps done before the split |
| `/?open=board:task:sample-card-9` | a subtask's page, with "Part of #7" under the title |
| `/?open=board:task:sample-card-1` | a card's page with blockers, a PR and a long thread |
| `/?open=board:task:sample-card-2` | a blocker with recommended answers |
| `/?open=board:task:sample-card-5` | a done card with a reviewed PR |
| `/?open=board:task:sample-card-12` | Ready for you: an agent says it's done, its proof (screenshot, then how it was checked) at the top; Done closes it, Not done sends your words back (cards #112, #140) |
| `/?voice=ready&open=board:task:sample-card-1` | the mic button on the comment and answer boxes (card #240); `voice=needs-model` asks for the model download on a press, `voice=denied` shows the mic crossed out |
| `/?open=board:task:sample-card-idle` | Idle, no reason given (card #305): held, its holder idle half an hour, nothing named it waits on; on Home its row is amber "Idle" |
| `/?open=board:task:sample-card-job` | a holder between turns with a background job running (card #431): the page and Home say "running a background job", not "not working on it now" |
| `/?open=board:task:sample-card-named` | the same wait named as a blocker on another agent: Home reads "Waiting on agent sample-kestrel" (card #305) |
| `/?loop=1` | a wait loop (card #481): Home's Needs you has one Stuck row, "#70, #72 and #71 wait on each other", with Open #70/#72/#71; `&open=board:task:sample-card-loop-71` shows a wait as a grey "Waiting on #70 Talk 4: Spaces" line with Stop waiting and no answer box; `sample-card-loop-72` shows it beside a red question |
| `/?gone=1` | items whose agent is gone on cards nobody holds (card #485): `&open=board:task:sample-card-gone-73` shows an amber "Waiting on kestrel (gone)" wait with Hand to an agent beside Stop waiting; `sample-card-gone-74` shows piranha's done check with the amber "Waiting on piranha (gone)" line and Hand to an agent |
| `/?red=1` | main is red (card #436): Home leads with "Main is red: attach_exe fails on clean main; 2 cards wait on #60", and the line opens #60 |
| `/?spawn=first` | spawned agents' first-run notice (card #448): Home leads with "Irori now starts an agent for each card…", Keep it on / Turn it off / Settings › Agents |
| `/?spawn=states` | spawned agents' card lines (card #448): #65 "Starting an agent on this PC", #66 "Waiting: this PC is at its cap", #67 "Can't start: no checkout…"; #64's page shows End for the agent Irori started, and its hover "Irori started it for #64" |
| `/?spawn=states` on Home | spawned agents in Needs you (card #456, sa-7): #58 "Permission", sample-thyme stopped on a prompt, with Attach, Logs and Add to allow-list; #62 "Crashed", its agents crashed twice, with Start another agent |
| `/?spawn=states&open=board:task:sample-card-spawn-on` | Who's on it for an agent Irori started (card #456, sa-2): Attach, Logs, Stop, End, no Revive |
| `/?newcard` | New card (card #456, sa-5): "A fresh agent" by default on your own board, with the model and effort it starts with; `?newcard&spawn=off` shows "Any free agent" instead |
| `/?settings=agents` | Settings › Agents: the spawning switch, model and effort (#448), and the permissions (#449) |
| `/?idle=gap` | Home's idle line as a dispatch gap: idle agents and two cards nobody was handed, with why (card #341); `/` alone shows the shortage line, "N agents idle, no open work" |
| `/#notifications` | The Notifications page (card #377): unread items, a done check and a decision with their live buttons, an agent's ask; `/?notifications=all#notifications` shows All, with a resolved blocker's outcome and read items; `?notifications=none` an empty history |
| `/?filter=review` | the In review chip's cards: #16 waits on its PR's review, its agent free (card #191) |
| `/?open=inbox:sample-delivery-1` | a message Irori held (Release, Dismiss) |
| `/?open=inbox:sample-delivery-6` | a message an agent sent you (Reply) |
| `/?open=held` | the Held list: three held (one by Pause all), and History with a dismissed one to release |
| `/?open=held:sample-device/grok-sample7` | the Held list showing one sender's, as a held notification opens it |
| `/?devices` | Settings at Devices and connections (this PC, a MacBook, an offline node) |
| `/?sidebar=folded` | the sidebar folded to icons (card #135); its top button opens it again |
| `/?boards` | the Boards dialog: two boards of your own (MLG, Website refresh), New board |
| `/?filter=board:sample-board-mlg` | the grid on the MLG board's chip: mlg#1 and mlg#2 |
| `/?open=board:task:sample-board-1` | a card on a shared board: `mlg#1`, "Visible to", Withdraw |
| `/#board/sample-board-mlg` | MLG, shared with Blake (card #268): each card's owner chip (Stephen, Blake, Former member) and the Mine / Blake chips |
| `/?filter=owner:sample-person-0#board/sample-board-mlg` | MLG on Blake's cards only |
| `/#card/sample-board-3` | Blake's card that his agent kite works: the owner chip in the header, kite in Who's on it |
| `/?share=sample-card-10#card/sample-card-10` | Share to board's preview, with a path in the goal taken out |
| `/?settings` | Settings (card #132): Notifications; `?settings=devices`, `=people`, `=app` open those sections |
| `/?update=ready` | the update card (`downloading`, `installing` too) |
| `/?android&update=available` | the phone's update bar (card #270): Update downloads, checks SHA256SUMS and opens Android's install screen; `update=ready` says Install, `update=error` Try again |
| `/?notes` | Notes (card #413): Mine, with New from agents at the top (two proposals and an agent's proposed edit, each with Keep / Accept, Edit, Remove), the kind chips, and the live notes with their tags, source chip, author and who kept them; one says Still true? |
| `/?notes=mlg` | Notes on the MLG board's tab: kite's proposal waits on anyone's Accept |
| `/?open=board:task:sample-card-4` | (Notes) under Who's on it: the notes sample-heron got on wake and the tag that matched each |
| `/?open=board:task:sample-card-12` | Keep as note under the done proof; comments and blocker answers have it too, on hover |
| `/?empty` | no cards yet |
| `/?theme=hearth` | a theme without saving it (`blue`, the default, `hearth`, `midnight`, `forest`, `sakura`, `mono`; card #275); add it to any row above |
| `/?mode=light` | light (or `dark`, `system`) without saving it (dark is the default); add it to any row above |
| `/?glass=milk` | a glass style without saving it (`frost`, the default, `clear`, `milk`, `rim`, `ember`, `liquid`) |
| `/?settings=appearance` | Settings > Appearance: themes, your colours, light/dark/system, glass |
| `/?from=sample-card-4` | the grid as a card's Back leaves it (that card marked and focused) |
| `/?open=board:task:sample-ask-2` | an ask (the person's prompt to the room) an agent took: status line, Close, the thread |
| `/?open=board:task:sample-ask-1` | an ask the agents heard and nobody has taken yet |
| `/?filter=done` | the Done tab: closed cards, the latest first (a closed ask shows what it became) |
| `/?slow#card/sample-card-1` | a card's page while the card loads |
| `/#card/sample-card-gone` | a card's page for a card that isn't there |

`?devices`, `?boards`, `?share`, `?settings`, `?update`, `?from` and `?filter` live in `src/lib/preview.ts`; `?slow` in `src/lib/cards.ts`. An open
card's page is `#card/<id>` in the address (`/#card/sample-card-4` opens it
too); its Back, Esc and the browser's back return to the grid.

## Screenshots

With the preview running, `sh scripts/preview-shots.sh` (Git Bash) saves
PNGs of every row above to `frontend/preview-shots/` (gitignored) using the
installed Chrome or Edge, headless, with a throwaway profile. On macOS set
`BROWSER_EXE` to Chrome's binary; `PREVIEW_MODE=light` shoots the light
palette and `PREVIEW_URL` points it at another port.
