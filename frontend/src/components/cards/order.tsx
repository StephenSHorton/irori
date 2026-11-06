// Not yet and order (card #570), kept quiet: the list's shape says it. A
// card that follows another sits under it; one that can't start yet folds
// away at the end. All visible text: the phone has no hover.

import { useState, type ReactNode } from "react"
import { ChevronRightIcon } from "lucide-react"
import { cardRef } from "@/components/cards/bits"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { cards, over, type CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import {
  around,
  laterOf,
  whenLater,
  type HasLater,
  type HasOrder,
  type OrderStep,
} from "@/lib/later"
import { errorText } from "@/lib/talk"
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
      <span className="font-mono">{cardRef(s)}</span> {plainLine(s.title)}
    </button>
  )
}

// WhenLine sits under a card's title, as "Part of" does: when it starts,
// and what it holds up.
export function WhenLine({
  card,
  onOpen,
}: {
  card: HasLater & HasOrder
  onOpen?: (id: string) => void
}) {
  const l = laterOf(card)
  if (l) return <HeldLine card={card} line={whenLater(l)} />
  const { before, after } = around(card)
  if (!before && !after) return null
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

// HeldLine is a held card's line with its Start now.
function HeldLine({ card, line }: { card: { id: string }; line: string }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <p className="text-sm text-muted-foreground">
      {plainLine(line)}{" "}
      <button
        type="button"
        className="text-ink underline underline-offset-2 disabled:opacity-60"
        disabled={busy}
        onClick={async () => {
          setBusy(true)
          setError(null)
          try {
            await cards.startNow(card.id)
          } catch (err) {
            setError(errorText(err))
          } finally {
            setBusy(false)
          }
        }}
      >
        Start now
      </button>
      {error && <span className="block text-xs text-destructive">{error}</span>}
    </p>
  )
}

// NotYetButton holds a card back from the agents: one box takes a card, a
// date or a version, and left empty it waits for the person's word.
export function NotYetButton({ card }: { card: CardSummary }) {
  const url = new URLSearchParams(window.location.search)
  const [open, setOpen] = useState(url.get("later") === "hold")
  const [until, setUntil] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  if (laterOf(card) || over(card) || card.ask) return null
  const hold = async () => {
    setBusy(true)
    setError(null)
    try {
      await cards.notYet(card.id, until)
      setUntil("")
      setOpen(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o)
        if (!o) setError(null)
      }}
    >
      <PopoverTrigger
        render={
          <Button variant="ghost" size="sm" className="h-7 text-xs">
            Not yet
          </Button>
        }
      />
      <PopoverContent align="end">
        <form
          className="contents"
          onSubmit={(e) => {
            e.preventDefault()
            void hold()
          }}
        >
          <p className="text-[13px] text-ink">Hold it back until</p>
          <Input
            aria-label="Until"
            placeholder="#83, 1 Dec or v0.13.0"
            className="h-8"
            value={until}
            disabled={busy}
            onChange={(e) => setUntil(e.target.value)}
          />
          <p className="text-xs text-muted-foreground">
            Leave it empty to hold it until you say. No agent gets it before
            then.
          </p>
          {error && <p className="text-xs text-destructive">{error}</p>}
          <Button
            type="submit"
            size="sm"
            className="h-7 justify-self-end text-xs"
            disabled={busy}
          >
            Hold back
          </Button>
        </form>
      </PopoverContent>
    </Popover>
  )
}
