// Browser-preview switches for the sample mode (bun run preview:look, or
// bun run dev, in a plain browser; frontend/PREVIEW.md). Like ?empty and
// ?open=, they only exist outside the app: inside Wails they read nothing.
//
//   ?settings       opens the settings popover
//   ?devices        opens the Devices dialog
//   ?update=ready   shows the update card (also downloading, installing)

import { inWails } from "./irori"

// previewFlag is a query parameter's value ("" when it has none), or null
// when it is absent or the page runs inside the app.
export function previewFlag(name: string): string | null {
  if (inWails()) return null
  return new URLSearchParams(window.location.search).get(name)
}
