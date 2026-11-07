// The terminal panel (card #734): docked at the bottom of the window,
// toggled by Ctrl+` or the palette's Toggle terminal, as in VS Code. On a
// card's page it holds that card's shells; anywhere else the person's own
// shell on this device. Drag its top edge (or focus it and use the arrow
// keys) to resize; the height is remembered. Desktop only: a phone runs
// no shells (inline-terminals.md section 2).

import { useRef } from "react"
import { XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { termsHere } from "@/lib/term"
import {
  MinHeight,
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
function Edge({ height }: { height: number }) {
  const drag = useRef<{ y: number; h: number } | null>(null)
  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize the terminal"
      aria-valuenow={height}
      aria-valuemin={MinHeight}
      tabIndex={0}
      className="absolute inset-x-0 -top-1 z-10 h-2 cursor-row-resize outline-none hover:bg-primary/30 focus-visible:bg-primary/40"
      onPointerDown={(e) => {
        if (e.button !== 0) return
        e.preventDefault()
        e.currentTarget.setPointerCapture(e.pointerId)
        drag.current = { y: e.clientY, h: height }
      }}
      onPointerMove={(e) => {
        const d = drag.current
        if (!d) return
        setTermPanelHeight(d.h + (d.y - e.clientY))
      }}
      onPointerUp={(e) => {
        const d = drag.current
        if (!d) return
        drag.current = null
        setTermPanelHeight(d.h + (d.y - e.clientY), true)
      }}
      onPointerCancel={() => {
        if (drag.current) setTermPanelHeight(height, true)
        drag.current = null
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp") setTermPanelHeight(height + Step, true)
        else if (e.key === "ArrowDown") setTermPanelHeight(height - Step, true)
        else return
        e.preventDefault()
      }}
    />
  )
}
