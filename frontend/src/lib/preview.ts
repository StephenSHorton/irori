// Browser-preview switches for the sample mode (bun run preview:look, or
// bun run dev, in a plain browser; frontend/PREVIEW.md). Like ?empty and
// ?open=, they only exist outside the app: inside Wails they read nothing.
//
//   ?settings[=<s>] opens Settings at a section: notifications, appearance,
//                   devices, people, agents, desks or app
//   ?storage[=<kind>] opens Storage (card #728), or one kind's page
//   ?devices        opens Settings at Devices and connections
//   ?pair=join      there, the Join with a code form (card #1261);
//                   looking: a join still looking for the other device,
//                   found only through an address; address: the same with
//                   the address box shown at once, not after 15 s
//   ?boards         opens New or join a space (card #689)
//   ?share=<card>   opens Share to board on that card's page
//   ?transfer=<card> opens Transfer ownership (or Ask to own it) there
//   ?move=<card>    opens Move to another board there
//   ?rename=<card>  opens the title's rename field on that card's or
//                   thread's page (card #779)
//   ?update=ready   shows the update card (also downloading, installing)
//   ?voice=ready    shows the mic on the text boxes (also needs-model, denied);
//                   pressing it hears a sample sentence (lib/voice.ts)
//   ?call=<state>   huddles (#654) on lib/call.ts's stand-in: start (one
//                   on in #netcode, ringing here; Join shows the
//                   transcript notice first, #656), sheet (the open
//                   channel's Huddle start sheet, with the notice, #655),
//                   live (you're in #suzuri's: the Transcript on badge,
//                   Aiko not transcribed, captions written as it goes),
//                   share (and Spencer shares a window, #658) or sharing
//                   (you do); transcript opens #suzuri's earlier huddle's
//                   transcript beside it (#656)
//   ?given=batch   Needs you's block of cards given by people (decision 69):
//                   two givers, one batch of 4 (sample-cards.ts)
//   ?spawn=first   shows the spawned agents' first-run notice on Home
//   ?spawn=limit    Settings › Agents with a usage limit holding the queue
//   ?newcard        opens the New card dialog (?spawn=off: spawning off)
//   ?addboard       opens New board (with its optional description) on a
//                   space's or Personal's boards (card #690)
//   ?description=<board> opens that board's Edit description on its page
//   ?close=<board>  opens that board's Close board dialog on its page
//   ?closed         opens the Closed boards fold under a space's tiles
//   ?slots          shows three held slots in the sidebar's Slots
//   ?from=<card>    the grid as that card's Back leaves it (App.tsx)
//   ?filter=done    the grid on that filter tab (App.tsx)
//   ?theme=<id>     a theme without saving it: blue, hearth, midnight,
//                   forest, sakura, mono (main.tsx, lib/appearance.ts)
//   ?mode=light     light (also dark, system) without saving it
//   ?glass=<style>  a glass style without saving it (frost, clear, ...)
//   ?meters=open    opens the top bar's hardware meters panel (card #599);
//   ?gpu=none       the meters on a machine with no GPU (lib/meters.ts)
//   ?sync=on        shows the header's sync badge (card #1089, lib/sync.ts)
//   ?usage=none     Claude usage with no reading yet; ?usage=stale
//                   one from 40 minutes ago; ?usage=80 both windows 80%
//                   used (lib/usage.ts)
//   ?term           a card page's Terminal section with one blank stand-in
//                   shell and its height handle (lib/term.ts, card #730);
//                   ?term=open also opens the bottom terminal panel
//   ?assistant=open Ask Irori (card #783) open on a sample chat; busy with a
//                   reply being written, new on an empty chat, draft with a
//                   check-first thread and its Post thread button, follow a
//                   thread it posted and the lines on its work (card #785)
//                   (lib/assistant.ts)
//                   go with a reply that takes you somewhere and its button
//                   (card #784, lib/goto.ts)
//   ?assistant-ask=<q>  opens Ask Irori on a new chat and asks q (the real
//                   assistant in a window check); &assistant-post then posts
//                   its drafted thread (lib/assistant.ts autoAsk)
//   ?winddown       a finished Stop everywhere, still on; =running mid-run,
//                   =off started again mid-run, =on on from another
//                   machine, =device this machine's Stop here mid-run
//                   (lib/wind-down.ts, cards #773, #787)
//   ?agentspause=on Stop here on (lib/agents.ts, card #665)

import { inWails } from "./irori"

// previewFlag is a query parameter's value ("" when it has none), or null
// when it is absent or the page runs inside the app. Only a debug build's
// window check opens the app's window with ?dev (app/src/shell/shot.rs,
// card #587), and then the switches apply there too.
export function previewFlag(name: string): string | null {
  const q = new URLSearchParams(window.location.search)
  if (inWails() && !q.has("dev")) return null
  return q.get(name)
}
