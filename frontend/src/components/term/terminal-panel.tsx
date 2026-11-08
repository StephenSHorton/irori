// The terminal panel (card #734): docked at the bottom of the window,
// toggled by Ctrl+` or the palette's Toggle terminal, as in VS Code. On a
// card's page it holds that card's shells; anywhere else the person's own
// shell on this device. Drag its top edge (or focus it and use the arrow
// keys) to resize; the height is remembered. Desktop only: a phone runs
// no shells (inline-terminals.md section 2).

import { type PointerEvent as ReactPointerEvent, useRef } from "react"
import { XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { termsHere } from "@/lib/term"
import {
  MinHeight,
  clampHeight,
  TerminalShortcut,
  setTermPanelHeight,
  setTermPanelOpen,
  useTermPanel,
} from "@/lib/term-panel"
import { shortcutKeys } from "@/lib/commands"

import { DeviceTerminals, TerminalSection } from "./terminal-section"

// How far one arrow key moves the edge.
const Step = 24

export function TerminalPanel({
  cardId,
  cardLabel,
}: {
  // The card whose page is open, or null off a card.
  cardId: string | null
  // How the header names that card ("#734").
  cardLabel?: string
}) {
  const { open, height } = useTermPanel()
  if (!open || !termsHere()) return null
  return (
    <section
      aria-label="Terminal panel"
      // A window made shorter since never loses the page under it.
      className="relative flex max-h-[calc(100svh-160px)] shrink-0 flex-col border-t border-border bg-background"
      style={{ height }}
    >
      <Edge height={height} />
      <header className="flex shrink-0 items-center gap-2 px-4 pt-1.5 pb-1 sm:px-6">
        <h2 className="text-[12px] font-medium tracking-wide text-muted-foreground uppercase">
          Terminal
        </h2>
        <span className="min-w-0 truncate text-[12px] text-muted-foreground">
          {cardId
            ? `this card${cardLabel ? ` · ${cardLabel}` : ""}`
            : "this device"}
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="ml-auto size-6 text-muted-foreground"
          onClick={() => setTermPanelOpen(false)}
          title={`Hide the terminal (${shortcutKeys(TerminalShortcut).join("+")})`}
          aria-label="Hide the terminal"
        >
          <XIcon className="size-3.5" aria-hidden />
        </Button>
      </header>
      <div className="min-h-0 flex-1 px-4 pb-2 sm:px-6">
        {cardId ? (
          <TerminalSection key={cardId} cardId={cardId} fill />
        ) : (
          <DeviceTerminals fill />
        )}
      </div>
    </section>
  )
}

// Edge is the panel's top edge: drag it, or focus it and press the arrow
// keys, to resize. The height is remembered when the drag ends.
function Edge({ height: kept }: { height: number }) {
  const dragging = useRef(false)
  // The height as shown: a remembered one taller than this window allows
  // is drawn at the most it can be, and dragging starts from there.
  const height = clampHeight(kept, window.innerHeight)
  // The drag follows the pointer on the window, not through pointer
  // capture, which WKWebView drops after the first move (card #730).
  const startDrag = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || dragging.current) return
    e.preventDefault()
    dragging.current = true
    const y0 = e.clientY
    let h = height
    // The page under the pointer would show its own cursor and select text.
    document.body.style.cursor = "row-resize"
    document.body.style.userSelect = "none"
    const move = (ev: PointerEvent) => {
      h = height + (y0 - ev.clientY)
      setTermPanelHeight(h)
    }
    const end = () => {
      window.removeEventListener("pointermove", move)
      window.removeEventListener("pointerup", end)
      window.removeEventListener("pointercancel", end)
      window.removeEventListener("blur", end)
      dragging.current = false
      document.body.style.cursor = ""
      document.body.style.userSelect = ""
      setTermPanelHeight(h, true)
    }
    window.addEventListener("pointermove", move)
    window.addEventListener("pointerup", end)
    window.addEventListener("pointercancel", end)
    window.addEventListener("blur", end)
  }
  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize the terminal"
      aria-valuenow={height}
      aria-valuemin={MinHeight}
      tabIndex={0}
      title="Drag to change the panel's height"
      className="absolute inset-x-0 -top-1 z-10 h-2 cursor-row-resize touch-none outline-none hover:bg-primary/30 focus-visible:bg-primary/40"
      onPointerDown={startDrag}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp") setTermPanelHeight(height + Step, true)
        else if (e.key === "ArrowDown") setTermPanelHeight(height - Step, true)
        else return
        e.preventDefault()
      }}
    />
  )
}
