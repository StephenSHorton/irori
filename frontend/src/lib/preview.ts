// Browser-preview switches for the sample mode (bun run preview:look, or
// bun run dev, in a plain browser; frontend/PREVIEW.md). Like ?empty and
// ?open=, they only exist outside the app: inside Wails they read nothing.
//
//   ?settings[=<s>] opens Settings at a section: notifications, appearance,
//                   devices, people, agents, desks,
//                   maintenance or app
//   ?devices        opens Settings at Devices and connections
//   ?boards         opens the Boards dialog
//   ?share=<card>   opens Share to board on that card's page
//   ?transfer=<card> opens Transfer ownership (or Ask to own it) there
//   ?move=<card>    opens Move to another board there
//   ?update=ready   shows the update card (also downloading, installing)
//   ?voice=ready    shows the mic on the text boxes (also needs-model, denied);
//                   pressing it hears a sample sentence (lib/voice.ts)
//   ?call=<state>   huddles (#654) on lib/call.ts's stand-in: start (one
//                   on in #netcode, ringing here), sheet (the open
//                   channel's Huddle start sheet, #655) or live (you're in
//                   #suzuri's); share and transcript are still the #571
//                   mock-up (components/talk/call-mock.tsx)
//   ?spawn=first   shows the spawned agents' first-run notice on Home
//   ?spawn=limit    Settings › Agents with a usage limit holding the queue
//   ?newcard        opens the New card dialog (?spawn=off: spawning off)
//   ?slots          shows two check.sh runs in the sidebar's Check slots
//   ?from=<card>    the grid as that card's Back leaves it (App.tsx)
//   ?filter=done    the grid on that filter tab (App.tsx)
//   ?theme=<id>     a theme without saving it: blue, hearth, midnight,
//                   forest, sakura, mono (main.tsx, lib/appearance.ts)
//   ?mode=light     light (also dark, system) without saving it
//   ?glass=<style>  a glass style without saving it (frost, clear, ...)
//   ?meters=open    opens the top bar's hardware meters panel (card #599);
//   ?gpu=none       the meters on a machine with no GPU (lib/meters.ts)
//   ?usage=none     Claude usage left with no reading yet; ?usage=stale
//                   one from 40 minutes ago (lib/usage.ts)

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
