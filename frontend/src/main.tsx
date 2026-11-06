import { StrictMode, Suspense, lazy } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import App from "./App.tsx"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "./gen/bindings"
import { inHost } from "./lib/irori"
import { applyAppearance, clean, loadAppearance, onAppearanceApplied } from "./lib/appearance"
import { sendCardLook } from "./lib/card-look"
import { previewFlag } from "./lib/preview"
import { followInsets } from "./lib/insets"

followInsets()

// Card #232: index.html shows the opening screen while the Core starts on
// its own thread (a fully synced log takes a while on a phone); every
// command needs the Core, so the app mounts once it is up. Outside the
// window (the browser preview, #260) there is no Core to wait for.
async function mount() {
  try {
    if (inHost()) await commands.coreReady()
  } catch (err) {
    const text = document.getElementById("irori-opening-text")
    if (text)
      text.textContent = `Irori couldn't open: ${err instanceof Error ? err.message : String(err)}`
    return
  }
  // Card #259: the design-system gallery, browser preview only (?gallery).
  if (!inHost() && new URLSearchParams(window.location.search).has("gallery")) {
    const Gallery = lazy(() => import("./design/gallery"))
    createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <TooltipProvider delay={300}>
          <Suspense fallback={null}>
            <Gallery />
          </Suspense>
        </TooltipProvider>
      </StrictMode>
    )
    return
  }
  // Card #275: index.html applied the saved appearance before the first
  // paint; apply it again from this build's code (a custom theme's CSS is
  // made afresh). The browser preview can try one without saving it:
  // ?theme=<id>, ?mode=light|dark|system, ?glass=<style>.
  // Card #527: the notification cards follow the look; every apply (this
  // one, and each change in Settings > Appearance) sends it to the host.
  onAppearanceApplied((a) => void sendCardLook(a))
  const saved = loadAppearance()
  applyAppearance(
    clean({
      ...saved,
      theme: previewFlag("theme") ?? saved.theme,
      mode: (previewFlag("mode") as typeof saved.mode | null) ?? saved.mode,
      glass: previewFlag("glass") ?? saved.glass,
    })
  )
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      {/* The look is set on <html> (lib/appearance.ts); Settings >
          Appearance changes it. */}
      <TooltipProvider delay={300}>
        <App />
      </TooltipProvider>
    </StrictMode>
  )
}

void mount()
