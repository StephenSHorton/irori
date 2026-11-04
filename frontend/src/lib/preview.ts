// Browser-preview switches for the sample mode (bun run preview:look, or
// bun run dev, in a plain browser; frontend/PREVIEW.md). Like ?empty and
// ?open=, they only exist outside the app: inside Wails they read nothing.
//
//   ?settings[=<s>] opens Settings at a section: notifications, appearance,
//                   devices, people, maintenance or app
//   ?devices        opens Settings at Devices and connections
//   ?boards         opens the Boards dialog
//   ?share=<card>   opens Share to board on that card's page
//   ?update=ready   shows the update card (also downloading, installing)
//   ?voice=ready    shows the mic on the text boxes (also needs-model, denied)
//   ?slots          shows two check.sh runs in the sidebar's Check slots
//   ?from=<card>    the grid as that card's Back leaves it (App.tsx)
//   ?filter=done    the grid on that filter tab (App.tsx)
//   ?theme=<id>     a theme without saving it: blue, hearth, midnight,
//                   forest, sakura, mono (main.tsx, lib/appearance.ts)
//   ?mode=light     light (also dark, system) without saving it
//   ?glass=<style>  a glass style without saving it (frost, clear, ...)

import { inWails } from "./irori"

// previewFlag is a query parameter's value ("" when it has none), or null
// when it is absent or the page runs inside the app.
export function previewFlag(name: string): string | null {
  if (inWails()) return null
  return new URLSearchParams(window.location.search).get(name)
}
