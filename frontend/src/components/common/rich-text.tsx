import { useEffect, useState } from "react"
import { CheckIcon, CopyIcon, ExternalLinkIcon } from "lucide-react"

import { cards, LinkKind, type Link } from "@/lib/cards"
import { copyText } from "@/lib/clipboard"
import { plainLine } from "@/lib/irori"
import { richBlocks, richInline, type Inline } from "@/lib/rich"
import { cn } from "@/lib/utils"

// RichText shows untrusted multi-line text from an agent with its links,
// code and lists usable (lib/rich.ts): links open in the person's browser,
// code has a one-click Copy, line breaks and lists keep their shape. Never
// HTML. Set interactive false where it sits inside a button: links and
// code then only look the part.
export function RichText({
  text,
  className,
  interactive = true,
}: {
  text: string
  className?: string
  interactive?: boolean
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
                <Pieces pieces={l} interactive={interactive} />
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
                  <Pieces pieces={it.inline} interactive={interactive} />
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
}: {
  text: string
  interactive?: boolean
}) {
  return <Pieces pieces={richInline(text)} interactive={interactive} />
}

function Pieces({
  pieces,
  interactive,
}: {
  pieces: Inline[]
  interactive: boolean
}) {
  return pieces.map((p, i) => {
    if (p.kind === "text") return <span key={i}>{p.text}</span>
    if (p.kind === "code")
      return <Code key={i} text={p.text} copy={interactive} />
    return <InlineLink key={i} link={p} interactive={interactive} />
  })
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
  const look = "break-all text-primary decoration-primary/40 underline underline-offset-2"
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
      title={done === false ? "Couldn't copy" : done ? "Copied" : `${label}: ${text}`}
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
export function LinkChip({ link, className }: { link: Link; className?: string }) {
  const ref = plainLine(link.ref)
  if (link.kind === LinkKind.LinkURL) {
    let host = ""
    try {
      host = new URL(ref).hostname.replace(/^www\./, "")
    } catch {
      return null
    }
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
      title={link.kind === LinkKind.LinkCommit ? `Commit ${ref}` : `Repo path ${ref}`}
    >
      <code className="truncate font-mono text-ink">{ref}</code>
      <CopyButton text={ref} />
    </span>
  )
}
