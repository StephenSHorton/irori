// Not yet and order (card #570), kept quiet: the list's shape says it. A
// card that follows another sits under it; one that can't start yet folds
// away at the end. All visible text: the phone has no hover.

import { useState, type ReactNode } from "react"
import { ChevronRightIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import type { CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { chainOf, laterOf, type OrderStep } from "@/lib/later"
import { cn } from "@/lib/utils"

// NotYetFold is the end of a list: one quiet line until it is opened.
export function NotYetFold({
  count,
  children,
}: {
  count: number
  children: ReactNode
}) {
  const url = new URLSearchParams(window.location.search)
  const [open, setOpen] = useState(url.has("notyet"))
  return (
    <section className="grid min-w-0 gap-1.5">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 justify-self-start px-1 text-xs text-muted-foreground outline-none hover:text-ink focus-visible:ring-1 focus-visible:ring-ring"
      >
        <ChevronRightIcon
          aria-hidden
          className={cn("size-3 transition-transform", open && "rotate-90")}
        />
        {count} not yet
      </button>
      {open && children}
    </section>
  )
}

function CardLink({
  step: s,
  onOpen,
}: {
  step: OrderStep
  onOpen?: (id: string) => void
}) {
  return (
    <button
      type="button"
      className="text-ink underline-offset-2 hover:underline disabled:no-underline"
      disabled={!onOpen}
      onClick={() => onOpen?.(s.id)}
    >
      <span className="font-mono">#{s.short}</span> {plainLine(s.title)}
    </button>
  )
}

// WhenLine sits under a card's title, as "Part of" does: when it starts,
// and what it holds up.
export function WhenLine({
  card,
  onOpen,
}: {
  card: { id: string }
  onOpen?: (id: string) => void
}) {
  const l = laterOf(card)
  if (l)
    return (
      <p className="text-sm text-muted-foreground">
        Not yet: {l.line.charAt(0).toLowerCase() + l.line.slice(1)}.{" "}
        {plainLine(l.why)}.{" "}
        <button
          type="button"
          className="text-ink underline underline-offset-2"
        >
          Start now
        </button>
      </p>
    )
  const ch = chainOf(card)
  if (!ch) return null
  const i = ch.findIndex((s) => s.id === card.id)
  const before = ch[i - 1]
  const after = ch[i + 1]
  return (
    <p className="text-sm text-muted-foreground">
      {before && (
        <>
          After <CardLink step={before} onOpen={onOpen} />
          {after ? ", then " : ""}
        </>
      )}
      {after && (
        <>
          {!before && "Then "}
          <CardLink step={after} onOpen={onOpen} />
        </>
      )}
    </p>
  )
}

// NotYetButton holds a card back from the agents: one box takes a card, a
// date or a version, and left empty it waits for the person's word.
export function NotYetButton({ card }: { card: CardSummary }) {
  const url = new URLSearchParams(window.location.search)
  const [open, setOpen] = useState(url.get("later") === "hold")
  if (laterOf(card)) return null
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button variant="ghost" size="sm" className="h-7 text-xs">
            Not yet
          </Button>
        }
      />
      <PopoverContent align="end">
        <p className="text-[13px] text-ink">Hold it back until</p>
        <Input
          aria-label="Until"
          placeholder="#83, 1 Dec or v0.13.0"
          className="h-8"
        />
        <p className="text-xs text-muted-foreground">
          Leave it empty to hold it until you say. No agent gets it before then.
        </p>
        <Button
          size="sm"
          className="h-7 justify-self-end text-xs"
          onClick={() => setOpen(false)}
        >
          Hold back
        </Button>
      </PopoverContent>
    </Popover>
  )
}
