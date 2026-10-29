// copyText puts text on the clipboard: Wails' own in the app (the webview
// may refuse navigator.clipboard), the browser's otherwise. It resolves
// false when neither could.

import { Clipboard } from "@wailsio/runtime"

import { inWails } from "./irori"

export async function copyText(text: string): Promise<boolean> {
  if (inWails()) {
    try {
      await Clipboard.SetText(text)
      return true
    } catch {
      // Fall through to the browser's.
    }
  }
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}
