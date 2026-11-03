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
| `/?open=board:task:sample-card-7` | a parent card's page: its subtasks, one blocked, and the steps done before the split |
| `/?open=board:task:sample-card-9` | a subtask's page, with "Part of #7" under the title |
| `/?open=board:task:sample-card-1` | a card's page with blockers, a PR and a long thread |
| `/?open=board:task:sample-card-2` | a blocker with recommended answers |
| `/?open=board:task:sample-card-5` | a done card with a reviewed PR |
| `/?open=board:task:sample-card-12` | Ready for you: an agent says it's done, its proof (screenshot, then how it was checked) at the top; Done closes it, Not done sends your words back (cards #112, #140) |
| `/?voice=ready&open=board:task:sample-card-1` | the mic button on the comment and answer boxes (card #240); `voice=needs-model` asks for the model download on a press, `voice=denied` shows the mic crossed out |
| `/?open=board:task:sample-card-idle` | Idle, no reason given (card #305): held, its holder idle half an hour, nothing named it waits on; on Home its row is amber "Idle" |
| `/?open=board:task:sample-card-named` | the same wait named as a blocker on another agent: Home reads "Waiting on agent sample-kestrel" (card #305) |
| `/?idle=gap` | Home's idle line as a dispatch gap: idle agents and two cards nobody was handed, with why (card #341); `/` alone shows the shortage line, "N agents idle, no open work" |
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
