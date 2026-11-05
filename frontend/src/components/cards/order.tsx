// Later and order on the Board and a card's page (card #570). All visible
// text, no tooltips: the phone has no hover.

import { useState, type ReactNode } from "react"
import { CheckIcon, ClockIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import type { CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import {
  chainOf,
  chainsOf,
  laterOf,
  type OrderState,
  type OrderStep,
} from "@/lib/later"
import { cn } from "@/lib/utils"

const stepTone: Record<OrderState, string> = {
  done: "border-border text-muted-foreground line-through",
  working: "border-info/60 bg-info/10 text-ink",
  waiting: "border-border bg-muted/30 text-ink",
  later: "border-dashed border-border text-muted-foreground",
}

// OrderPanel is the Board's view of its orders: one line per chain, so the
// sequence reads at a glance and what's next is plain.
export function OrderPanel({
  cards,
  onOpen,
}: {
  cards: readonly CardSummary[]
  onOpen: (id: string) => void
}) {
  const chains = chainsOf(cards)
  if (!chains.length) return null
  return (
    <section className="grid min-w-0 gap-1.5" aria-label="Order">
      <h3 className="flex items-center gap-2 px-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        In order
        <span className="tabular-nums">{chains.length}</span>
      </h3>
      <ul className="glass-panel grid min-w-0 overflow-hidden">
        {chains.map((ch) => (
          <li
            key={ch[0].id}
            className="grid min-w-0 gap-1.5 border-b border-border/60 px-4 py-2.5 last:border-b-0"
          >
            <span className="flex min-w-0 flex-wrap items-center gap-1">
              {ch.map((s, i) => (
                <span key={s.id} className="flex items-center gap-1">
                  {i > 0 && (
                    <span aria-hidden className="text-muted-foreground">
                      →
                    </span>
                  )}
                  <Step step={s} onOpen={onOpen} />
                </span>
              ))}
            </span>
            <NextLine chain={ch} />
          </li>
        ))}
      </ul>
    </section>
  )
}

function Step({
  step: s,
  onOpen,
}: {
  step: OrderStep
  onOpen: (id: string) => void
}) {
  return (
    <button
      type="button"
      onClick={() => onOpen(s.id)}
      aria-label={`#${s.short} ${plainLine(s.title)}, ${s.state}`}
      className={cn(
        "flex items-center gap-1 rounded-md border px-1.5 py-0.5 font-mono text-xs tabular-nums outline-none hover:bg-accent/50 focus-visible:ring-1 focus-visible:ring-ring",
        stepTone[s.state]
      )}
    >
      {s.state === "done" && <CheckIcon aria-hidden className="size-3" />}#
      {s.short}
    </button>
  )
}

function NextLine({ chain }: { chain: OrderStep[] }) {
  const now = chain.find((s) => s.state === "working")
  const next = chain.find((s) => s.state === "waiting")
  return (
    <span className="min-w-0 text-[12.5px] text-muted-foreground">
      {now && (
        <>
          Now <span className="font-mono">#{now.short}</span>{" "}
          {plainLine(now.title)}
        </>
      )}
      {now && next && ". "}
      {next && (
        <>
          Next <span className="font-mono">#{next.short}</span>{" "}
          {plainLine(next.title)}
        </>
      )}
    </span>
  )
}

// OrderList is a card's page: the whole order, this card marked.
export function OrderList({
  card,
  onOpen,
}: {
  card: { id: string }
  onOpen?: (id: string) => void
}) {
  const ch = chainOf(card) ?? []
  return (
    <ol className="grid gap-1">
      {ch.map((s, i) => {
        const here = s.id === card.id
        return (
          <li key={s.id}>
            <button
              type="button"
              disabled={here || !onOpen}
              onClick={() => onOpen?.(s.id)}
              className={cn(
                "grid w-full min-w-0 grid-cols-[1.25rem_auto_minmax(0,1fr)_auto] items-baseline gap-x-2 rounded-lg border px-3 py-1.5 text-left text-[13px]",
                here
                  ? "border-primary/60 bg-primary/10 text-ink"
                  : "border-transparent text-muted-foreground hover:bg-accent/50"
              )}
            >
              <span className="text-xs tabular-nums">{i + 1}</span>
              <span className="font-mono text-xs">#{s.short}</span>
              <span
                className={cn("truncate", s.state === "done" && "line-through")}
              >
                {plainLine(s.title)}
              </span>
              <span className="text-xs">
                {here
                  ? "This card"
                  : s.state === "done"
                    ? "Done"
                    : s.state === "working"
                      ? "Working"
                      : ""}
              </span>
            </button>
          </li>
        )
      })}
    </ol>
  )
}

// LaterItem is a parked card's page: why, what brings it back, and the
// person's word.
export function LaterItem({ card }: { card: { id: string } }) {
  const l = laterOf(card)
  if (!l) return null
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-x-2 gap-y-0.5 rounded-lg border border-border bg-muted/30 px-3 py-2">
      <div className="min-w-0 text-[13px] text-ink">{plainLine(l.why)}</div>
      <Button variant="outline" size="sm" className="row-span-2 h-7 text-xs">
        Bring back now
      </Button>
      <div className="min-w-0 text-xs text-muted-foreground">
        {l.line}. Parked by {l.by}. Nobody is handed it until then.
      </div>
    </div>
  )
}

// LaterButton parks a card from its page: why, and what brings it back.
export function LaterButton({ card }: { card: CardSummary }) {
  const url = new URLSearchParams(window.location.search)
  const [open, setOpen] = useState(url.get("later") === "dialog")
  const [back, setBack] = useState("card")
  if (laterOf(card)) return null
  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        className="h-7 gap-1.5 text-xs"
        title="Not for now: nobody is handed it until it comes back"
        onClick={() => setOpen(true)}
      >
        <ClockIcon aria-hidden />
        Later
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Put #{plainLine(card.short)} off for later</DialogTitle>
          </DialogHeader>
          <div className="grid gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="later-why">Why</Label>
              <Input
                id="later-why"
                placeholder="Not until the Talk work is done"
              />
            </div>
            <div className="grid gap-1.5">
              <Label>It comes back</Label>
              <RadioGroup
                value={back}
                onValueChange={(v) => setBack(String(v))}
              >
                <Choice value="card" text="When a card finishes">
                  <Input
                    aria-label="Which card"
                    placeholder="#83"
                    className="h-7 w-24"
                  />
                </Choice>
                <Choice value="date" text="On a date">
                  <Input
                    aria-label="Which date"
                    type="date"
                    className="h-7 w-36"
                  />
                </Choice>
                <Choice value="version" text="When a device runs a version" />
                <Choice value="word" text="Only when I say" />
              </RadioGroup>
            </div>
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button onClick={() => setOpen(false)}>Put off</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

function Choice({
  value,
  text,
  children,
}: {
  value: string
  text: string
  children?: ReactNode
}) {
  return (
    <label className="flex min-h-7 items-center gap-2 text-[13px]">
      <RadioGroupItem value={value} />
      <span className="min-w-0 flex-1">{text}</span>
      {children}
    </label>
  )
}
