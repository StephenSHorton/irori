import {
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ComponentProps,
  type KeyboardEvent,
} from "react"
import { flushSync } from "react-dom"

import { AgentCards } from "@/components/cards/agent-hover"
import { MicButton } from "@/components/common/mic-button"
import { Face } from "@/components/cards/faces"
import { Popover, PopoverContent } from "@/components/ui/popover"
import { Textarea } from "@/components/ui/textarea"
import { RefDot } from "@/components/common/rich-text"
import { deviceVar, refCandidates, useMentionCandidates } from "@/lib/cards"
import { agentStatus } from "@/lib/agent-status"
import { plainLine } from "@/lib/irori"
import {
  addMention,
  boardRows,
  cardTokenAt,
  composing as imeComposing,
  filterCards,
  insertCardRef,
  type CardCandidate,
  filterCandidates,
  insertMention,
  isFull,
  isGone,
  keepMentions,
  MaxMentions,
  menuKey,
  moveHighlight,
  settle,
  settleRange,
  tokenAt,
  agentOnLabel,
  agentOnRows,
  rowName,
  MentionBoard,
  type Mention,
  type MentionCandidate,
  type Token,
} from "@/lib/mentions"
import { cn } from "@/lib/utils"

type Props = Omit<ComponentProps<typeof Textarea>, "value" | "onChange"> & {
  value: string
  mentions: Mention[]
  onChange: (value: string, mentions: Mention[]) => void
  // The picker's rows; left out, the box reads them itself
  // (useMentionCandidates) once it has focus.
  candidates?: MentionCandidate[]
  // Set: no picker, and the reason is the box's title.
  disabledMentions?: string
  // Voice input (card #240): a mic button right after the box (a sibling,
  // so the site lays it out in its row). Hidden where there's no engine.
  // "main": the page's or dialog's main composer, which Cmd/Ctrl+D starts
  // when no box has focus (card #634).
  voice?: boolean | "main"
  // The card the box is on: its "the agent on #N" row comes first (card
  // #452, sa-4).
  card?: string
}

// MentionTextarea is a Textarea with the @ picker (card #77,
// docs/design/board.md "In the window"): typing "@" opens a list of agents
// at the caret, filtered by what follows. Focus stays in the box. Tab
// enters the list and steps through it (Shift+Tab back, Up and Down too),
// Enter or a click puts "@name " in the text and records the agent's key,
// Esc closes the list and keeps the text. While the list is open, Enter
// never reaches the site's own onKeyDown, so it doesn't send (with no
// rows it does nothing). Deleting the "@name" drops its key; pasting one,
// or typing punctuation straight after an exact name, records it.
export function MentionTextarea({
  value,
  mentions,
  onChange,
  candidates: given,
  disabledMentions,
  voice,
  card,
  onKeyDown,
  onBlur,
  onFocus,
  onSelect,
  title,
  ...rest
}: Props) {
  const el = useRef<HTMLTextAreaElement>(null)
  const [focused, setFocused] = useState(false)
  const [wanted, setWanted] = useState(false)
  const [caret, setCaret] = useState(0)
  // dismissed is the token start Esc (or a click away) closed the list
  // on; it stays closed for that token.
  const [dismissed, setDismissed] = useState<number | null>(null)
  // hi is the highlighted row for one token and query; -1 is none.
  const [hi, setHi] = useState<{ at: string; i: number }>({ at: "", i: -1 })
  const pendingCaret = useRef<number | null>(null)
  const listId = useId()

  const loaded = useMentionCandidates(!given && wanted)
  const board = useContext(MentionBoard)
  const all = given ?? loaded
  // The # picker (card #114): the window's live cards. An "@" token wins.
  const ctx = useContext(AgentCards)
  // Each card an agent is live on also offers "the agent on #N" (card
  // #452): picking it mentions whoever holds that card now.
  const cands = useMemo(() => {
    const own = boardRows(all, board)
    return [
      ...own,
      ...boardRows(agentOnRows(ctx?.cards ?? [], own, card), board),
    ]
  }, [all, board, ctx?.cards, card])
  const names = useMemo(() => cands.map(rowName), [cands])
  const cardCands = useMemo(
    () => refCandidates(ctx?.cards ?? [], ctx?.threads ?? []),
    [ctx?.cards, ctx?.threads]
  )
  const agentToken: Token | null =
    focused && !disabledMentions ? tokenAt(value, caret, names) : null
  const cardToken: Token | null =
    focused && !agentToken && cardCands.length
      ? cardTokenAt(
          value,
          caret,
          cardCands.map((c) => c.title)
        )
      : null
  const token: Token | null = agentToken ?? cardToken
  const forCards = !agentToken && !!cardToken
  // A dismissal lasts while its token does: once the token is gone (sent,
  // deleted, cleared by the site) the next "@" opens the list again.
  if (dismissed !== null && token?.start !== dismissed) setDismissed(null)
  const open = !!token && token.start !== dismissed
  const agentRows = useMemo(
    () =>
      open && !forCards && token ? filterCandidates(cands, token.query) : [],
    [open, forCards, token?.query, cands] // eslint-disable-line react-hooks/exhaustive-deps
  )
  const cardRows = useMemo(
    () =>
      open && forCards && token ? filterCards(cardCands, token.query) : [],
    [open, forCards, token?.query, cardCands] // eslint-disable-line react-hooks/exhaustive-deps
  )
  const rows: unknown[] = forCards ? cardRows : agentRows
  const at = token ? `${token.start}:${token.query}` : ""
  const highlight = hi.at === at ? hi.i : -1
  const shown = rows.length ? (highlight < 0 ? 0 : highlight) : -1
  // At the cap a new agent can't be added; one already named can.
  const kept = keepMentions(mentions, value)
  const capped = isFull(kept)
  const taken = (row: MentionCandidate) =>
    kept.some((m) => m.key === row.who.key)

  // The caret goes after an inserted mention once React has put the text in.
  useLayoutEffect(() => {
    const c = pendingCaret.current
    if (c === null || !el.current) return
    pendingCaret.current = null
    el.current.setSelectionRange(c, c)
  })

  // Esc closes the list and nothing else: not the dialog or card page
  // around the box (their handlers run on window and document).
  useEffect(() => {
    if (!open || !token) return
    const start = token.start
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape" || e.isComposing) return
      e.preventDefault()
      e.stopPropagation()
      setDismissed(start)
    }
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  }, [open, token?.start]) // eslint-disable-line react-hooks/exhaustive-deps

  // The list sits at the caret: a virtual element measured from a mirror
  // of the box.
  const anchor = useMemo(
    () => ({
      getBoundingClientRect: () =>
        el.current ? caretRect(el.current, caret) : new DOMRect(),
      get contextElement() {
        return el.current ?? undefined
      },
    }),
    [caret]
  )

  const sync = () => {
    const t = el.current
    if (t) setCaret(t.selectionStart ?? 0)
  }

  const pickCard = (row: CardCandidate) => {
    if (!token) return
    const r = insertCardRef(value, token, caret, row.ref)
    pendingCaret.current = r.caret
    setCaret(r.caret)
    setDismissed(token.start)
    onChange(r.text, keepMentions(mentions, r.text))
  }

  const pick = (row: MentionCandidate) => {
    if (!token || (capped && !taken(row))) return
    const r = insertMention(value, token, caret, row.who.name)
    const next = addMention(keepMentions(mentions, r.text), {
      key: row.who.key,
      name: row.who.name,
    })
    pendingCaret.current = r.caret
    setCaret(r.caret)
    // A name that another one starts with (oak, oak tree) would leave the
    // list open on the new text; it is done.
    setDismissed(token.start)
    onChange(r.text, next)
  }

  const keyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    const composing = imeComposing(e)
    const k = menuKey(
      { key: e.key, shiftKey: e.shiftKey, isComposing: composing },
      open,
      rows.length
    )
    if (k === "next" || k === "prev") {
      e.preventDefault()
      e.stopPropagation()
      setHi({
        at,
        i: moveHighlight(highlight, rows.length, k === "next" ? 1 : -1),
      })
      return
    }
    if (k === "pick") {
      e.preventDefault()
      e.stopPropagation()
      if (forCards) pickCard(cardRows[shown])
      else pick(agentRows[shown])
      return
    }
    if (k === "hold") {
      // The list is open with nothing to pick: Enter waits for the
      // person to fix the name or press Esc.
      e.preventDefault()
      e.stopPropagation()
      return
    }
    if (k === "close") {
      // The window listener has it; this keeps the site's handler out.
      e.preventDefault()
      e.stopPropagation()
      if (token) setDismissed(token.start)
      return
    }
    // Enter that sends: an exact, unique name at the caret counts as
    // picked. flushSync, so the form's submit sees it.
    if (e.key === "Enter" && !e.shiftKey && !composing && !disabledMentions) {
      const next = settle(value, caret, mentions, cands)
      if (next !== mentions) flushSync(() => onChange(value, next))
    }
    onKeyDown?.(e)
  }

  const change = (v: string, at: number) => {
    let next = mentions
    // A space or closing punctuation typed after an exact, unique name
    // counts as picking it.
    const p = at - 1
    const typed =
      p >= 0 &&
      v.length === value.length + 1 &&
      v.slice(0, p) + v.slice(p + 1) === value
    if (!disabledMentions && typed && endsMention.test(v[p])) {
      next = settle(value, p, next, cands)
      // Settled by punctuation ("@oak,"): the list is done, or Enter
      // would put "@oak " over the comma.
      const t = tokenAt(value, p, names)
      if (next !== mentions && t) setDismissed(t.start)
    }
    // Pasted text (or Ctrl+Z bringing a mention back): each exact,
    // unique name in what came in counts as picked.
    if (!disabledMentions && !typed && v.length > value.length) {
      let from = 0
      while (from < value.length && v[from] === value[from]) from++
      const tail = value.length - from
      let back = 0
      while (
        back < tail &&
        v[v.length - 1 - back] === value[value.length - 1 - back]
      )
        back++
      // From the "@" a paste may finish ("@" typed, then "oak" pasted).
      const start = Math.max(0, v.lastIndexOf("@", from))
      next = settleRange(v, start, v.length - back, next, cands)
    }
    onChange(v, keepMentions(next, v))
  }

  return (
    <>
      <Textarea
        {...rest}
        ref={el}
        value={value}
        title={disabledMentions || title}
        onChange={(e) => {
          const at = e.target.selectionStart ?? 0
          setCaret(at)
          if (e.target.value.includes("@")) setWanted(true)
          change(e.target.value, at)
        }}
        onKeyDown={keyDown}
        onSelect={(e) => {
          sync()
          onSelect?.(e)
        }}
        onFocus={(e) => {
          setFocused(true)
          setWanted(true)
          sync()
          onFocus?.(e)
        }}
        onBlur={(e) => {
          setFocused(false)
          setDismissed(null)
          // Clicking Send blurs first: an exact name still counts.
          if (!disabledMentions) {
            const next = settle(value, caret, mentions, cands)
            if (next !== mentions) onChange(value, next)
          }
          onBlur?.(e)
        }}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={
          open && shown >= 0 ? `${listId}-${shown}` : undefined
        }
        aria-autocomplete={disabledMentions ? undefined : "list"}
      />
      <Popover
        open={open}
        modal={false}
        onOpenChange={(o, d) => {
          if (o || !token) return
          // A click back in the box only moves the caret.
          if (d.reason === "outside-press" && d.event?.target === el.current)
            return
          if (d.reason === "outside-press" || d.reason === "escape-key")
            setDismissed(token.start)
        }}
      >
        <PopoverContent
          anchor={anchor}
          side="bottom"
          align="start"
          sideOffset={4}
          initialFocus={false}
          finalFocus={false}
          className="w-80 gap-0 p-1"
        >
          {forCards ? (
            <div
              role="listbox"
              id={listId}
              aria-label="Cards and threads to mention"
              className="grid max-h-72 overflow-y-auto"
            >
              {cardRows.length === 0 ? (
                <p className="px-2 py-1.5 text-xs text-muted-foreground">
                  {token?.query
                    ? `No card or thread matches ${plainLine(token.query)}`
                    : "No cards or threads"}
                </p>
              ) : (
                cardRows.map((c, i) => (
                  <div
                    key={c.id}
                    id={`${listId}-${i}`}
                    role="option"
                    aria-selected={i === shown}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => pickCard(c)}
                    className={cn(
                      "flex min-w-0 cursor-pointer items-center gap-2 px-2 py-1.5 text-[13px]",
                      i === shown &&
                        (i === highlight
                          ? "bg-accent ring-1 ring-ring ring-inset"
                          : "bg-muted/60"),
                      i !== shown && "hover:bg-muted/40"
                    )}
                  >
                    <RefDot status={c.status} />
                    <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                      {c.ref}
                    </span>
                    <span className="min-w-0 truncate text-ink">{c.title}</span>
                    <span className="ml-auto shrink-0 text-[11px] text-muted-foreground tabular-nums">
                      {c.status.label}
                      {c.status.progress &&
                        ` · ${c.status.progress.done}/${c.status.progress.total}`}
                    </span>
                  </div>
                ))
              )}
            </div>
          ) : (
            <div
              role="listbox"
              id={listId}
              aria-label="Agents to mention"
              className="grid max-h-72 overflow-y-auto"
            >
              {capped && (
                <p className="px-2 py-1.5 text-xs text-muted-foreground">
                  At most {MaxMentions} mentions
                </p>
              )}
              {rows.length === 0 ? (
                <p className="px-2 py-1.5 text-xs text-muted-foreground">
                  {token?.query
                    ? `No agent named ${plainLine(token.query)}`
                    : "No agents to mention"}
                </p>
              ) : (
                agentRows.map((c, i) => (
                  <MentionRow
                    key={`${c.card ?? ""}@${c.who.key}`}
                    id={`${listId}-${i}`}
                    row={c}
                    active={i === shown}
                    explicit={i === highlight}
                    disabled={capped && !taken(c)}
                    onPick={() => pick(c)}
                  />
                ))
              )}
            </div>
          )}
        </PopoverContent>
      </Popover>
      {voice && (
        <MicButton
          target={el}
          main={voice === "main"}
          value={value}
          disabled={!!rest.disabled || !!rest.readOnly}
          onText={(text, at) => {
            // Words, not names: mentions whose "@name" went are dropped.
            pendingCaret.current = at
            setCaret(at)
            onChange(text, keepMentions(mentions, text))
          }}
        />
      )}
    </>
  )
}

// MentionRow is one agent in the picker: its face, name, device in the
// device colour, and the repo (else the cwd's last part), so two agents
// with one name tell apart. A gone one is greyed; it can still be picked
// (stored, not delivered).
function MentionRow({
  id,
  row,
  active,
  explicit,
  disabled,
  onPick,
}: {
  id: string
  row: MentionCandidate
  active: boolean
  explicit: boolean
  // Past the cap: shown, not pickable.
  disabled: boolean
  onPick: () => void
}) {
  const { who } = row
  const gone = isGone(who)
  const tint = deviceVar(who.color)
  const place = row.repo || lastPart(row.cwd)
  return (
    <div
      id={id}
      role="option"
      aria-selected={active}
      aria-disabled={disabled || undefined}
      title={[plainLine(who.name), who.where, row.cwd, gone && "gone"]
        .filter(Boolean)
        .join(" · ")}
      // Keep focus in the box.
      onMouseDown={(e) => e.preventDefault()}
      onClick={onPick}
      className={cn(
        "flex min-w-0 cursor-pointer items-center gap-2 px-2 py-1.5 text-[13px]",
        active &&
          (explicit ? "bg-accent ring-1 ring-ring ring-inset" : "bg-muted/60"),
        !active && "hover:bg-muted/40",
        (gone || disabled) && "opacity-55",
        disabled && "cursor-not-allowed"
      )}
    >
      <Face who={who} className="shrink-0" noHover />
      <span
        className={cn(
          "min-w-0 truncate font-medium text-ink",
          gone && "line-through"
        )}
      >
        {plainLine(row.card ? agentOnLabel(row.card) : who.name)}
      </span>
      {who.where && !row.card && (
        <span
          className="shrink-0 font-mono text-[10.5px] text-muted-foreground"
          style={tint ? { color: tint } : undefined}
        >
          {plainLine(who.where)}
        </span>
      )}
      <span className="ml-auto min-w-0 truncate text-right font-mono text-[10.5px] text-muted-foreground">
        {gone
          ? "gone"
          : row.card
            ? [plainLine(who.name), agentStatus(who).text]
                .filter(Boolean)
                .join(" · ")
            : [agentStatus(who).text, plainLine(place)]
                .filter(Boolean)
                .join(" · ")}
      </span>
    </div>
  )
}

// endsMention is a character that ends a typed name: whitespace or
// closing punctuation (lib/mentions' after).
const endsMention = /[\s.,;:!?)'"]/

function lastPart(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean)
  return parts[parts.length - 1] ?? ""
}

// Mirror styles: what decides where text wraps in the box.
const mirrored = [
  "boxSizing",
  "width",
  "paddingTop",
  "paddingRight",
  "paddingBottom",
  "paddingLeft",
  "borderTopWidth",
  "borderRightWidth",
  "borderBottomWidth",
  "borderLeftWidth",
  "borderStyle",
  "fontFamily",
  "fontSize",
  "fontWeight",
  "fontStyle",
  "fontVariant",
  "letterSpacing",
  "lineHeight",
  "textTransform",
  "textIndent",
  "wordSpacing",
  "tabSize",
] as const

// caretRect is the caret's box on screen: a hidden mirror div with the
// box's font, padding, width and wrapping holds the text up to the caret
// and ends in a marker span.
function caretRect(t: HTMLTextAreaElement, pos: number): DOMRect {
  const cs = getComputedStyle(t)
  const div = document.createElement("div")
  for (const p of mirrored) div.style[p] = cs[p]
  div.style.position = "absolute"
  div.style.visibility = "hidden"
  div.style.top = "0"
  div.style.left = "-9999px"
  div.style.whiteSpace = "pre-wrap"
  div.style.overflowWrap = "break-word"
  div.style.overflow = "hidden"
  div.textContent = t.value.slice(0, pos)
  const mark = document.createElement("span")
  mark.textContent = "​"
  div.appendChild(mark)
  document.body.appendChild(div)
  const box = t.getBoundingClientRect()
  const x =
    box.left + parseFloat(cs.borderLeftWidth) + mark.offsetLeft - t.scrollLeft
  const y =
    box.top + parseFloat(cs.borderTopWidth) + mark.offsetTop - t.scrollTop
  const h = mark.offsetHeight || parseFloat(cs.lineHeight) || 16
  div.remove()
  // Kept inside the box: a scrolled-away caret anchors at its edge.
  const top = Math.min(Math.max(y, box.top), box.bottom - h)
  return new DOMRect(x, top, 0, h)
}
