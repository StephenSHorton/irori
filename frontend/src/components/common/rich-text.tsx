import { useContext, useEffect, useState, type ReactNode } from "react"
import { CheckIcon, CopyIcon, ExternalLinkIcon } from "lucide-react"

import { AgentCards } from "@/components/cards/agent-hover"
import { Face } from "@/components/cards/faces"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  cards,
  cardStatus,
  deviceVar,
  LinkKind,
  refStatus,
  type CardMention,
  type Link,
  type RefStatus,
} from "@/lib/cards"
import { copyText } from "@/lib/clipboard"
import { plainLine } from "@/lib/irori"
import { useCardRef } from "@/lib/cards"
import {
  cardRefs,
  isGone,
  localRef,
  refText,
  splitMentions,
} from "@/lib/mentions"
import {
  boldRuns,
  richBlocks,
  richInline,
  safeHref,
  type Inline,
} from "@/lib/rich"
import { cn } from "@/lib/utils"

// RichText shows untrusted multi-line text from an agent with its links,
// code and lists usable (lib/rich.ts): links open in the person's browser,
// code has a one-click Copy, line breaks and lists keep their shape. Never
// HTML. Set interactive false where it sits inside a button: links and
// code then only look the part. mentions (card #77) turn each "@name"
// they name into a chip; any other "@word" stays text, and code is never
// split. bold (card #783, Ask Irori's replies) shows **bold** as bold.
export function RichText({
  text,
  className,
  interactive = true,
  mentions,
  bold = false,
}: {
  text: string
  className?: string
  interactive?: boolean
  mentions?: CardMention[]
  bold?: boolean
}) {
  const blocks = richBlocks(text)
  if (!blocks.length) return null
  return (
    <div className={cn("grid gap-1.5", className)}>
      {blocks.map((b, i) =>
        b.kind === "para" ? (
          <p key={i} className="whitespace-pre-wrap">
            {b.lines.map((l, j) => (
              <span key={j}>
                {j > 0 && <br />}
                <Pieces
                  pieces={l}
                  interactive={interactive}
                  mentions={mentions}
                  bold={bold}
                />
              </span>
            ))}
          </p>
        ) : (
          <ul key={i} className="grid gap-0.5">
            {b.items.map((it, j) => (
              <li
                key={j}
                className="grid grid-cols-[auto_minmax(0,1fr)] gap-1.5"
              >
                <span className="text-muted-foreground tabular-nums">
                  {it.marker === "*" || it.marker === "-" ? "•" : it.marker}
                </span>
                <span className="whitespace-pre-wrap">
                  <Pieces
                    pieces={it.inline}
                    interactive={interactive}
                    mentions={mentions}
                    bold={bold}
                  />
                </span>
              </li>
            ))}
          </ul>
        )
      )}
    </div>
  )
}

// RichLine is RichText for one line (a question, an answer, a note).
export function RichLine({
  text,
  interactive = true,
  mentions,
}: {
  text: string
  interactive?: boolean
  mentions?: CardMention[]
}) {
  return (
    <Pieces
      pieces={richInline(text)}
      interactive={interactive}
      mentions={mentions}
    />
  )
}

function Pieces({
  pieces,
  interactive,
  mentions,
  bold,
}: {
  pieces: Inline[]
  interactive: boolean
  mentions?: CardMention[]
  bold?: boolean
}) {
  return pieces.map((p, i) => {
    if (p.kind === "text" && bold && p.text.includes("**"))
      return (
        <span key={i}>
          {boldRuns(p.text).map((r, j) =>
            r.bold ? (
              <strong key={j} className="font-semibold">
                <CardRefs text={r.text} interactive={interactive} />
              </strong>
            ) : (
              <CardRefs key={j} text={r.text} interactive={interactive} />
            )
          )}
        </span>
      )
    if (p.kind === "text") {
      if (!mentions?.length)
        return (
          <span key={i}>
            <CardRefs text={p.text} interactive={interactive} />
          </span>
        )
      return (
        <span key={i}>
          {splitMentions(p.text, mentions).map((s, j) =>
            s.kind === "text" ? (
              <CardRefs key={j} text={s.text} interactive={interactive} />
            ) : (
              <MentionChip key={j} text={s.text} mention={s.mention} />
            )
          )}
        </span>
      )
    }
    if (p.kind === "code")
      return <Code key={i} text={p.text} copy={interactive} />
    return <InlineLink key={i} link={p} interactive={interactive} />
  })
}

// CardRefs is text with each card ref ("#12", "mlg#12", card #114) made a
// chip (card #343): a live card or a thread the window holds at once, a
// done card or an old thread once the backend has looked it up (card
// #144). Each shows its state and moves with the card. A ref nothing
// has, or text outside the app's context, stays text.
function CardRefs({
  text,
  interactive,
}: {
  text: string
  interactive: boolean
}) {
  const ctx = useContext(AgentCards)
  if (!ctx || !text.includes("#")) return text
  const refs = cardRefs(text)
  if (!refs.length) return text
  const out: ReactNode[] = []
  let last = 0
  for (const r of refs) {
    out.push(text.slice(last, r.start))
    const label = text.slice(r.start, r.end)
    const hit = localRef(r, ctx.cards, ctx.threads ?? [])
    if (hit?.kind === "card") {
      const c = ctx.cards.find((x) => x.id === hit.id)
      out.push(
        <RefChip
          key={r.start}
          label={label}
          title={hit.title}
          status={c ? cardStatus(c) : null}
          interactive={interactive}
          onOpen={() => ctx.onOpen(hit.id)}
        />
      )
    } else if (hit) {
      const t = ctx.threads?.find((x) => x.id === hit.id)
      out.push(
        <RefChip
          key={r.start}
          label={label}
          title={hit.title}
          status={refStatus({
            thread: true,
            done: t?.state === "resolved",
            state: "",
            blocked: false,
          })}
          interactive={interactive}
          onOpen={() =>
            ctx.onOpenThread ? ctx.onOpenThread(hit.id) : ctx.onOpen(hit.id)
          }
        />
      )
    } else {
      out.push(
        <LookedUpRef
          key={r.start}
          r={refText(r)}
          label={label}
          interactive={interactive}
        />
      )
    }
    last = r.end
  }
  out.push(text.slice(last))
  return <>{out}</>
}

// LookedUpRef is a ref the window doesn't hold: a done card, a card on
// a board it isn't showing, or a thread past the panel's list. Text until
// (and unless) the backend finds it; fresh on every board change.
function LookedUpRef({
  r,
  label,
  interactive,
}: {
  r: string
  label: string
  interactive: boolean
}) {
  const ctx = useContext(AgentCards)
  const t = useCardRef(r)
  if (!ctx || !t) return label
  return (
    <RefChip
      label={label}
      title={t.title}
      status={refStatus(t)}
      interactive={interactive}
      onOpen={() =>
        t.thread && ctx.onOpenThread ? ctx.onOpenThread(t.id) : ctx.onOpen(t.id)
      }
    />
  )
}

const dotTone: Record<RefStatus["tone"], string> = {
  info: "bg-info",
  destructive: "bg-destructive",
  success: "bg-success",
  warning: "bg-warning",
  slate: "bg-muted-foreground",
}

const closedTone = new Set(["Done", "Failed", "Canceled", "Resolved"])

// RefDot is a card's state as a dot in its status colour (card #343).
export function RefDot({
  status,
  className,
}: {
  status: RefStatus
  className?: string
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "inline-block size-1.5 shrink-0 rounded-full",
        dotTone[status.tone],
        className
      )}
    />
  )
}

// RefChip is a card ref in text (card #343): the ref, a dot in the card's
// status colour, its title cut short and its steps, so a thread reads
// what each #12 is and how far along without opening it. The whole
// title, the state and the steps are in its tooltip. It sits in the line
// as a word does (inline-flex, baseline), and inside a button
// (interactive false) it only looks the part.
function RefChip({
  label,
  title,
  status,
  interactive,
  onOpen,
}: {
  label: string
  title: string
  status: RefStatus | null
  interactive: boolean
  onOpen: () => void
}) {
  const name = plainLine(title)
  const steps = status?.progress
    ? `${status.progress.done}/${status.progress.total}`
    : ""
  const tip = [name, status?.label, steps && `${steps} steps`]
    .filter(Boolean)
    .join(" · ")
  const body = (
    <>
      {status && <RefDot status={status} className="self-center" />}
      <span className="shrink-0 font-medium text-primary">{label}</span>
      {name && (
        <span
          className={cn(
            "max-w-[22ch] min-w-0 truncate",
            // A closed card's title steps back, so done reads at a glance
            // beside the dot, not by colour alone.
            status && closedTone.has(status.label)
              ? "text-muted-foreground"
              : "text-ink/85"
          )}
        >
          {name}
        </span>
      )}
      {steps && (
        <span className="shrink-0 text-[0.85em] text-muted-foreground tabular-nums">
          {steps}
        </span>
      )}
    </>
  )
  const chip =
    "inline-flex max-w-full items-baseline gap-1 rounded-sm bg-primary/10 px-1 align-baseline leading-tight"
  if (!interactive)
    return (
      <span className={chip} title={tip} data-ref-state={status?.label}>
        {body}
      </span>
    )
  return (
    <button
      type="button"
      className={cn(chip, "cursor-pointer hover:bg-primary/20")}
      title={tip}
      aria-label={`${label} ${tip}`}
      data-ref-state={status?.label}
      onClick={(e) => {
        e.stopPropagation()
        onOpen()
      }}
    >
      {body}
    </button>
  )
}

// MentionChip is one "@name" a mention names: in the agent's device
// colour, its face on hover. Gone (stored gone, or gone now) is greyed and
// struck through: it was never delivered.
function MentionChip({
  text,
  mention: m,
}: {
  text: string
  mention: CardMention
}) {
  const gone = m.gone || isGone(m.who)
  const tint = deviceVar(m.who.color)
  const chip = (
    <span
      className={cn(
        "rounded-sm px-0.5 font-medium",
        gone
          ? "text-muted-foreground line-through opacity-70"
          : "bg-primary/10 text-primary"
      )}
      style={
        !gone && tint
          ? {
              color: tint,
              background: `color-mix(in oklch, ${tint} 14%, transparent)`,
            }
          : undefined
      }
      title={gone ? `${plainLine(m.name)} is gone; not delivered` : undefined}
    >
      {text}
    </span>
  )
  if (gone) return chip
  return (
    <Tooltip>
      <TooltipTrigger render={chip} />
      <TooltipContent>
        <Face who={m.who} noDot noHover />
        <span>
          {plainLine(m.who.name || m.name)}
          {m.who.where && ` · ${plainLine(m.who.where)}`}
        </span>
      </TooltipContent>
    </Tooltip>
  )
}

// InlineLink is a link in the text: what the agent wrote, its host in
// full weight so a look-alike path can't pass for another site, and the
// full URL on hover. It opens in the person's browser, never in the app.
function InlineLink({
  link,
  interactive,
}: {
  link: Extract<Inline, { kind: "link" }>
  interactive: boolean
}) {
  const body = (
    <>
      <span className="opacity-75">{link.pre}</span>
      <span className="font-semibold">{link.host}</span>
      <span className="opacity-75">{link.rest}</span>
      {link.shownHost && (
        <span className="ml-1 font-semibold">({link.shownHost})</span>
      )}
      <ExternalLinkIcon
        className="ml-0.5 inline size-3 align-[-1px]"
        aria-hidden
      />
    </>
  )
  const look =
    "break-all text-primary decoration-primary/40 underline underline-offset-2"
  if (!interactive) return <span className={look}>{body}</span>
  return (
    <button
      type="button"
      title={`Open ${link.href} in your browser`}
      onClick={() => void cards.openURL(link.href)}
      className={cn(
        look,
        "inline cursor-pointer bg-transparent p-0 text-left outline-none hover:decoration-primary focus-visible:ring-2 focus-visible:ring-ring/50"
      )}
    >
      {body}
    </button>
  )
}

// Code is `code` from the text, with a Copy beside it.
export function Code({ text, copy = true }: { text: string; copy?: boolean }) {
  return (
    <span className="inline-flex max-w-full items-baseline gap-0.5 align-baseline">
      <code className="bg-muted px-1 py-px font-mono text-[0.92em] break-all whitespace-pre-wrap text-ink">
        {text}
      </code>
      {copy && <CopyButton text={text} />}
    </span>
  )
}

// CopyButton copies text in one click and says so for a moment.
export function CopyButton({
  text,
  label = "Copy",
}: {
  text: string
  label?: string
}) {
  const [done, setDone] = useState<boolean | null>(null)
  useEffect(() => {
    if (done === null) return
    const id = window.setTimeout(() => setDone(null), 1500)
    return () => window.clearTimeout(id)
  }, [done])
  return (
    <button
      type="button"
      title={
        done === false ? "Couldn't copy" : done ? "Copied" : `${label}: ${text}`
      }
      aria-label={done ? "Copied" : `${label} ${text}`}
      onClick={() => void copyText(text).then(setDone)}
      className={cn(
        "inline-grid size-5 shrink-0 cursor-pointer place-items-center self-center text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50",
        done && "text-success",
        done === false && "text-destructive"
      )}
    >
      {done ? (
        <CheckIcon className="size-3" aria-hidden />
      ) : (
        <CopyIcon className="size-3" aria-hidden />
      )}
    </button>
  )
}

// LinkChip is a link the agent put on a blocker or an option: a URL opens
// in the person's browser (its label and host, the full URL on hover); a
// repo path or commit can't be opened from here, so it shows in full with
// a Copy.
export function LinkChip({
  link,
  className,
}: {
  link: Link
  className?: string
}) {
  const ref = plainLine(link.ref)
  // A URL that can't be opened safely (an old row with user@ in it) shows
  // as text with a Copy, like a path, instead of a button that does nothing.
  const href = link.kind === LinkKind.LinkURL ? safeHref(ref) : null
  if (href) {
    const host = new URL(href).hostname.replace(/^www\./, "")
    const label = plainLine(link.label) || host
    return (
      <button
        type="button"
        title={`Open ${ref} in your browser`}
        onClick={() => void cards.openURL(ref)}
        className={cn(
          "inline-flex h-6 max-w-full min-w-0 cursor-pointer items-center gap-1.5 border border-border bg-card px-2 text-xs text-ink outline-none hover:border-primary focus-visible:ring-2 focus-visible:ring-ring/50",
          className
        )}
      >
        <span className="truncate font-medium">{label}</span>
        {label !== host && (
          <span className="truncate text-muted-foreground">{host}</span>
        )}
        <ExternalLinkIcon className="size-3 shrink-0" aria-hidden />
      </button>
    )
  }
  return (
    <span
      className={cn(
        "inline-flex h-6 max-w-full min-w-0 items-center gap-1 border border-border bg-card pr-0.5 pl-2 text-xs",
        className
      )}
      title={
        link.kind === LinkKind.LinkCommit
          ? `Commit ${ref}`
          : link.kind === LinkKind.LinkURL
            ? `Link ${ref} (not opened from here)`
            : `Repo path ${ref}`
      }
    >
      <code className="truncate font-mono text-ink">{ref}</code>
      <CopyButton text={ref} />
    </span>
  )
}
