// A card's weight (card #963): light, normal or heavy, with a line on why
// and who set it. Each board's Settings › Agents maps a weight to the model
// and effort its agent starts with; no weight runs as normal. A weight the
// person sets is theirs: agents and the assistant can't change it, until
// "Let agents decide" hands it back.

import { useState } from "react"
import { WeightIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import {
  WEIGHT_LEVELS,
  shortModel,
  useAgents,
  weightBy,
  weightLabel,
  type AgentsSettings,
} from "@/lib/agents"
import { cards, over, type CardSummary, type CardWeight } from "@/lib/cards"
import { previewFlag } from "@/lib/preview"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

const MAX_REASON = 200

const levelName: Record<string, string> = {
  light: "Light",
  normal: "Normal",
  heavy: "Heavy",
}

// weightOf is a card's weight, or none: an older host sends no field.
export function weightOf(card: {
  weight?: CardWeight | null
}): CardWeight | null {
  return card.weight ?? null
}

// WeightTag is the grid tile's mark: just the level, small; what it runs
// on, the reason and who set it on hover.
export function WeightTag({ card }: { card: CardSummary }) {
  const w = weightOf(card)
  if (!w) return null
  return (
    <span
      className="inline-flex items-center gap-0.5 text-[11px] text-muted-foreground"
      title={`${weightLabel(w)}\n${weightBy(w)}`}
    >
      <WeightIcon aria-hidden className="size-3" />
      <span className="sr-only">Weight: </span>
      {w.level}
    </span>
  )
}

// WeightLine sits under the card's title: "heavy · Opus high: planning the
// split", who set it on hover.
export function WeightLine({ card }: { card: CardSummary }) {
  const w = weightOf(card)
  if (!w) return null
  return (
    <p
      className="flex min-w-0 items-start gap-1.5 text-sm text-muted-foreground"
      title={weightBy(w)}
    >
      <WeightIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
      <span className="min-w-0 break-words">
        <span className="sr-only">Weight: </span>
        {weightLabel(w)}
      </span>
    </p>
  )
}

// mapped is what a level starts an agent with on the card's board.
function mapped(s: AgentsSettings | null, level: string): string {
  if (!s) return ""
  if (level === "light") return shortModel(s.lightModel, s.lightEffort)
  if (level === "heavy") return shortModel(s.heavyModel, s.heavyEffort)
  return shortModel(s.model, s.effort)
}

// WeightButton is the person's control: pick Light, Normal or Heavy (with
// an optional line on why), or let the agents decide again.
export function WeightButton({ card }: { card: CardSummary }) {
  const w = weightOf(card)
  // ?weight=open opens it on load, for the preview (as ?later=hold).
  const [open, setOpen] = useState(() => previewFlag("weight") === "open")
  const [level, setLevel] = useState(w?.level ?? "normal")
  const [reason, setReason] = useState(
    w?.setBy === "person" ? (w.reason ?? "") : ""
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { settings } = useAgents(open, card.board ?? "")
  if (over(card) || card.ask) return null
  const save = async (to: string, why: string) => {
    setBusy(true)
    setError(null)
    try {
      await cards.setWeight(card.id, to, why)
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
        if (o) {
          // Start from the weight it has; the reason only if it's yours.
          setLevel(w?.level ?? "normal")
          setReason(w?.setBy === "person" ? (w.reason ?? "") : "")
        } else setError(null)
      }}
    >
      <PopoverTrigger
        render={
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs"
            title="How heavy its work is: picks the model and effort its agent starts with"
          >
            Weight
          </Button>
        }
      />
      <PopoverContent align="end">
        <form
          className="contents"
          onSubmit={(e) => {
            e.preventDefault()
            void save(level, reason)
          }}
        >
          <p className="text-[13px] text-ink">How heavy is it?</p>
          <div
            role="radiogroup"
            aria-label="Weight"
            className="grid grid-cols-3 gap-1"
          >
            {WEIGHT_LEVELS.map((l) => (
              <button
                key={l}
                type="button"
                role="radio"
                aria-checked={level === l}
                disabled={busy}
                onClick={() => setLevel(l)}
                className={cn(
                  "flex min-w-0 flex-col items-center rounded-md border px-1.5 py-1 text-xs outline-none focus-visible:ring-1 focus-visible:ring-ring",
                  level === l
                    ? "border-primary bg-primary/15 text-ink"
                    : "border-border text-muted-foreground hover:text-ink"
                )}
              >
                <span className="font-medium">{levelName[l]}</span>
                <span className="truncate text-[11px] text-muted-foreground">
                  {mapped(settings, l) || " "}
                </span>
              </button>
            ))}
          </div>
          <Input
            aria-label="Why"
            placeholder="Why (optional)"
            className="h-8"
            maxLength={MAX_REASON}
            value={reason}
            disabled={busy}
            onChange={(e) => setReason(e.target.value)}
          />
          <p className="text-xs text-muted-foreground">
            {w ? `${weightBy(w)}. ` : ""}A weight you set is yours: agents
            can&apos;t change it. Let agents decide hands it back; until one
            weighs it, it runs as Normal.
          </p>
          {error && <p className="text-xs text-destructive">{error}</p>}
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            {w && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-7 text-xs"
                disabled={busy}
                onClick={() => void save("", "")}
              >
                Let agents decide
              </Button>
            )}
            <Button
              type="submit"
              size="sm"
              className="h-7 text-xs"
              disabled={busy}
            >
              Set {levelName[level]?.toLowerCase() ?? level}
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  )
}
