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
| `/?open=board:task:sample-card-7` | a parent card's page: its subtasks, one blocked, and the steps done before the split |
| `/?open=board:task:sample-card-9` | a subtask's page, with "Part of #7" under the title |
| `/?open=board:task:sample-card-1` | a card's page with blockers, a PR and a long thread |
| `/?open=board:task:sample-card-2` | a blocker with recommended answers |
| `/?open=board:task:sample-card-5` | a done card with a reviewed PR |
| `/?open=inbox:sample-delivery-1` | a message Irori held (Release, Dismiss) |
| `/?open=inbox:sample-delivery-6` | a message an agent sent you (Reply) |
| `/?open=held` | the Held list: three held (one by Pause all), and History with a dismissed one to release |
| `/?open=held:sample-device/grok-sample7` | the Held list showing one sender's, as a held notification opens it |
| `/?devices` | the Devices dialog (this PC, a MacBook, an offline node) |
| `/?settings` | the settings popover |
| `/?update=ready` | the update card (`downloading`, `installing` too) |
| `/?empty` | no cards yet |
| `/?from=sample-card-4` | the grid as a card's Back leaves it (that card marked and focused) |
| `/?open=board:task:sample-ask-2` | an ask (the person's prompt to the room) an agent took: status line, Close, the thread |
| `/?open=board:task:sample-ask-1` | an ask the agents heard and nobody has taken yet |
| `/?filter=done` | the Done tab: closed cards, the latest first (a closed ask shows what it became) |
| `/?slow#card/sample-card-1` | a card's page while the card loads |
| `/#card/sample-card-gone` | a card's page for a card that isn't there |

`?devices`, `?settings`, `?update`, `?from` and `?filter` live in `src/lib/preview.ts`; `?slow` in `src/lib/cards.ts`. An open
card's page is `#card/<id>` in the address (`/#card/sample-card-4` opens it
too); its Back, Esc and the browser's back return to the grid.

## Screenshots

With the preview running, `sh scripts/preview-shots.sh` (Git Bash) saves
PNGs of every row above to `frontend/preview-shots/` (gitignored) using the
installed Chrome or Edge, headless, with a throwaway profile.
