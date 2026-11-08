// CommandPalette (card #213): Cmd/Ctrl+K or Cmd/Ctrl+P opens it from
// anywhere, a card's page and other dialogs included (Cmd+P no longer
// prints). One search covers cards (a ref like #212 or wind#51, or title
// words), boards, agents, and the commands features register in
// lib/commands (the app's sections are "Go to" commands). Results are
// grouped; an empty query shows what was picked last. Arrows and Enter
// pick, Esc closes.

import { useEffect, useMemo, useRef, useState } from "react"
import { Dialog as DialogPrimitive } from "@base-ui/react/dialog"
import {
  BotIcon,
  CornerDownLeftIcon,
  LayoutGridIcon,
  SearchIcon,
  SquareIcon,
  TerminalIcon,
  type LucideIcon,
} from "lucide-react"

import { Kbd } from "@/components/ui/kbd"
import type { BoardSummary } from "@/lib/boards"
import { cardRef, type CardSummary } from "@/lib/cards"
import { activeCommands, shortcutKeys, type Command } from "@/lib/commands"
import { rank, queryParts } from "@/lib/fuzzy"
import type { AgentCard } from "@/lib/models"
import { cn } from "@/lib/utils"

type Kind = "command" | "card" | "board" | "agent"

// Item is one row: what it shows and what Enter does.
export type Item = {
  kind: Kind
  // kind-prefixed, stable across opens: recents keep it.
  key: string
  title: string
  // A muted line after the title (a card's ref, a board's tag).
  hint?: string
  group: string
  shortcut?: string
  // A command's order in the empty palette, and whether it waits for a query.
  order?: number
  quiet?: boolean
  run: () => void
}

const recentKey = "irori.palette.recent"
const maxRecent = 6
// How many rows each group shows for a query.
const perGroup = 8

function readRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(recentKey) ?? "[]")
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : []
  } catch {
    return []
  }
}

function keepRecent(key: string) {
  const next = [key, ...readRecent().filter((k) => k !== key)].slice(
    0,
    maxRecent
  )
  try {
    localStorage.setItem(recentKey, JSON.stringify(next))
  } catch {
    // A private window keeps nothing.
  }
}

// PaletteJump is the event every pick in the palette sends before it runs,
// so a dialog left open under the palette (Settings, Held, a message)
// closes and what the pick opens shows (#218).
export const PaletteJump = "irori:palette-jump"

// isRef says whether a query is one card ref: #12, wind#51.
export function isRef(q: string): boolean {
  return /^[a-z0-9_-]*#\d+[a-z]*$/i.test(q.trim())
}

function cardRefText(c: CardSummary): string {
  return c.tag ? `${c.tag}#${c.short}` : `#${c.short}`
}

// paletteOpenKey says whether a keydown is the palette's: Cmd+K or Cmd+P on
// a Mac, Ctrl+K or Ctrl+P elsewhere, with no Shift or Alt.
export function paletteOpenKey(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
  mac: boolean
): boolean {
  const mod = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey
  if (!mod || e.shiftKey || e.altKey) return false
  const k = e.key.toLowerCase()
  return k === "k" || k === "p"
}

// buildItems is every row a query can find, commands first.
export function buildItems({
  commands,
  cards,
  boards,
  agents,
  onCard,
  onBoard,
  onAgent,
}: {
  commands: Command[]
  cards: CardSummary[]
  boards: BoardSummary[]
  agents: AgentCard[]
  onCard: (id: string) => void
  onBoard: (id: string) => void
  onAgent: (name: string) => void
}): (Item & { others: string[] })[] {
  const out: (Item & { others: string[] })[] = []
  for (const c of commands)
    out.push({
      kind: "command",
      key: `command:${c.id}`,
      title: c.title,
      group: c.group ?? "Commands",
      shortcut: c.shortcut,
      order: c.order,
      quiet: c.quiet,
      run: c.run,
      others: c.keywords ?? [],
    })
  for (const c of cards) {
    const ref = cardRefText(c)
    out.push({
      kind: "card",
      key: `card:${c.id}`,
      title: c.title,
      hint: ref,
      group: "Cards",
      run: () => onCard(c.id),
      others: [
        `#${c.short}`,
        ref,
        c.shortWas ? `#${c.shortWas}` : "",
        c.scopeLabel ?? "",
      ],
    })
  }
  for (const b of boards)
    out.push({
      kind: "board",
      key: `board:${b.boardId}`,
      title: b.name,
      hint: b.tag ? `${b.tag}#` : undefined,
      group: "Spaces",
      run: () => onBoard(b.boardId),
      others: [b.tag, "space", "board"],
    })
  // Agents Irori started for a card (card #456, spawned agents §10) are
  // listed under their card: its ref and title as the hint, and Enter
  // opens the card. Terminal agents keep "cards with this agent".
  const spawned = spawnedCards(cards)
  const seen = new Set<string>()
  for (const a of agents) {
    const c = spawned.get(a.key)
    if (c && a.status !== "gone") {
      const ref = cardRefText(c)
      out.push({
        kind: "agent",
        key: `agent:${a.key}`,
        title: a.name || a.key,
        hint: `on ${ref} ${c.title}`,
        group: "Agents on cards",
        run: () => onCard(c.id),
        others: [ref, `#${c.short}`, c.title, "agent"],
      })
      continue
    }
    if (!a.name || seen.has(a.name)) continue
    seen.add(a.name)
    out.push({
      kind: "agent",
      key: `agent:${a.name}`,
      title: a.name,
      hint: "cards with this agent",
      group: "Agents",
      run: () => onAgent(a.name),
      others: [a.title ?? "", a.aiTitle ?? "", "agent"],
    })
  }
  return out
}

// groupOrder is the order groups show in; any other group comes after
// Commands.
const groupOrder = [
  "Recent",
  "Go to",
  "Cards",
  "Spaces",
  "Commands",
  "Agents",
  "Agents on cards",
]

// spawnedCards maps each agent Irori started for a card to that card: a
// working row marked spawned. The first card wins for a stale second row.
export function spawnedCards(cards: CardSummary[]): Map<string, CardSummary> {
  const m = new Map<string, CardSummary>()
  for (const c of cards)
    for (const w of c.working) if (w.spawned && !m.has(w.key)) m.set(w.key, c)
  return m
}

function groupIndex(g: string): number {
  const i = groupOrder.indexOf(g)
  return i < 0 ? groupOrder.indexOf("Commands") + 0.5 : i
}

// results is the rows a query shows, grouped: the group with the best match
// first. Empty: recents, then the "Go to" commands and the rest of the
// commands. A card ref puts its card first.
export function results(
  items: (Item & { others: string[] })[],
  query: string,
  recent: string[]
): Item[] {
  if (!queryParts(query).length) {
    const byKey = new Map(items.map((i) => [i.key, i]))
    const rec = recent
      .map((k) => byKey.get(k))
      .filter((i): i is Item & { others: string[] } => !!i)
      .map((i) => ({ ...i, group: "Recent" }))
    const recKeys = new Set(rec.map((i) => i.key))
    const cmds = items
      .filter((i) => i.kind === "command" && !i.quiet && !recKeys.has(i.key))
      .map((r, i) => ({ r, i }))
      .sort((a, b) => (a.r.order ?? 100) - (b.r.order ?? 100) || a.i - b.i)
      .map(({ r }) => r)
    return sortGroups([...rec, ...cmds])
  }
  const ref = isRef(query) ? query.trim().toLowerCase() : ""
  const ranked = rank(items, query, (i) => ({
    title: i.title,
    others: i.others,
  }))
  const exact = ref
    ? ranked.filter(
        (r) =>
          r.item.kind === "card" &&
          r.item.others.some((o) => o.toLowerCase() === ref)
      )
    : []
  const rest = ranked.filter((r) => !exact.includes(r))
  // Best first within a group, and the group with the best match first.
  const groups = new Map<string, Item[]>()
  for (const r of rest) {
    const g = groups.get(r.item.group) ?? []
    if (g.length < perGroup) g.push(r.item)
    groups.set(r.item.group, g)
  }
  return [...exact.map((r) => r.item), ...[...groups.values()].flat()]
}

// sortGroups keeps rows in their order within a group and the groups in
// groupOrder.
function sortGroups(rows: Item[]): Item[] {
  return rows
    .map((r, i) => ({ r, i }))
    .sort((a, b) => groupIndex(a.r.group) - groupIndex(b.r.group) || a.i - b.i)
    .map(({ r }) => r)
}

const kindIcon: Record<Kind, LucideIcon> = {
  command: TerminalIcon,
  card: SquareIcon,
  board: LayoutGridIcon,
  agent: BotIcon,
}

export function CommandPalette({
  open,
  onOpenChange,
  cards,
  boards,
  agents,
  onCard,
  onBoard,
  onAgent,
  onThread,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  cards: CardSummary[]
  boards: BoardSummary[]
  agents: AgentCard[]
  onCard: (id: string) => void
  onBoard: (id: string) => void
  onAgent: (name: string) => void
  onThread: (id: string) => void
}) {
  // Mounted fresh on every open: the query is empty, recents are read now,
  // and what applies is what applies as it opens (a card's page, a board).
  const [query, setQuery] = useState("")
  const [active, setActive] = useState(0)
  // A ref no live card has (a done card, another board's old card, a
  // thread): looked up across every board, shown as its own row.
  const [found, setFound] = useState<{ q: string; item: Item } | null>(null)
  const list = useRef<HTMLDivElement>(null)
  const [recent] = useState(readRecent)
  const [commands] = useState(() => activeCommands())

  const items = useMemo(
    () =>
      buildItems({ commands, cards, boards, agents, onCard, onBoard, onAgent }),
    [commands, cards, boards, agents, onCard, onBoard, onAgent]
  )
  const shown = useMemo(() => {
    const rows = results(items, query, recent)
    if (
      found &&
      found.q === query.trim() &&
      !rows.some((r) => r.key === found.item.key)
    )
      return [found.item, ...rows]
    return rows
  }, [items, query, recent, found])

  // Look a ref up when no loaded card answers it.
  useEffect(() => {
    const q = query.trim()
    if (!isRef(q)) return
    const lower = q.toLowerCase()
    const live = cards.some(
      (c) => `#${c.short}` === lower || cardRefText(c).toLowerCase() === lower
    )
    if (live) return
    let stale = false
    void cardRef(q)
      .then((t) => {
        if (stale || !t) return
        setFound({
          q,
          item: {
            kind: "card",
            key: `card:${t.id}`,
            title: t.title,
            hint: `${q}${t.thread ? " · thread" : t.done ? " · done" : ""}`,
            group: "Cards",
            run: () => (t.thread ? onThread(t.id) : onCard(t.id)),
          },
        })
      })
      .catch(() => {})
    return () => {
      stale = true
    }
  }, [query, cards, onCard, onThread])

  const pick = (i: Item | undefined) => {
    if (!i) return
    keepRecent(i.key)
    onOpenChange(false)
    window.dispatchEvent(new Event(PaletteJump))
    // After the dialog lets go of focus, so the action's own focus sticks.
    window.setTimeout(() => i.run(), 0)
  }

  // Keep the active row in view.
  useEffect(() => {
    list.current
      ?.querySelector<HTMLElement>(`[data-index="${active}"]`)
      ?.scrollIntoView({ block: "nearest" })
  }, [active])

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.nativeEvent.isComposing) return
    const n = shown.length
    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
      e.preventDefault()
      setActive((a) => (n ? (a + 1) % n : 0))
    } else if (
      e.key === "ArrowUp" ||
      (e.ctrlKey && e.key === "p" && !e.metaKey)
    ) {
      e.preventDefault()
      setActive((a) => (n ? (a - 1 + n) % n : 0))
    } else if (e.key === "Home" && !e.shiftKey) {
      e.preventDefault()
      setActive(0)
    } else if (e.key === "End" && !e.shiftKey) {
      e.preventDefault()
      setActive(Math.max(0, n - 1))
    } else if (e.key === "Enter") {
      e.preventDefault()
      pick(shown[active])
    }
  }

  const activeId = shown[active] ? `palette-${active}` : undefined
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Backdrop className="fixed inset-0 isolate z-[60] bg-black/20 backdrop-blur-sm duration-100 dark:bg-black/50 data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0" />
        <DialogPrimitive.Popup
          data-slot="command-palette"
          aria-label="Command palette"
          className="fixed top-[14vh] left-1/2 z-[60] flex max-h-[min(560px,72vh)] w-[min(640px,calc(100%-2rem))] -translate-x-1/2 flex-col overflow-hidden text-sm text-popover-foreground glass-panel outline-none data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0"
        >
          <DialogPrimitive.Title className="sr-only">
            Command palette
          </DialogPrimitive.Title>
          <div className="flex items-center gap-2 border-b border-border px-3">
            <SearchIcon
              className="size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <input
              autoFocus
              value={query}
              onChange={(e) => {
                setQuery(e.target.value)
                setActive(0)
              }}
              onKeyDown={onKeyDown}
              placeholder="Jump to a card, space or agent, or run a command…"
              aria-label="Search cards, spaces, agents and commands"
              role="combobox"
              aria-expanded
              aria-controls="palette-list"
              aria-activedescendant={activeId}
              aria-autocomplete="list"
              spellCheck={false}
              autoComplete="off"
              className="h-12 min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-muted-foreground"
            />
            <Kbd className="text-[10px]">Esc</Kbd>
          </div>
          <div
            ref={list}
            id="palette-list"
            role="listbox"
            aria-label="Results"
            className="min-h-0 flex-1 overflow-y-auto p-1.5"
          >
            {shown.length === 0 && (
              <p className="px-3 py-6 text-center text-muted-foreground">
                Nothing matches “{query.trim()}”.
              </p>
            )}
            {shown.map((it, i) => {
              const head = i === 0 || shown[i - 1].group !== it.group
              const Icon = kindIcon[it.kind]
              return (
                <div key={`${it.group}:${it.key}`} role="presentation">
                  {head && (
                    <div
                      role="presentation"
                      className="px-2.5 pt-2.5 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase"
                    >
                      {it.group}
                    </div>
                  )}
                  <div
                    id={`palette-${i}`}
                    data-index={i}
                    role="option"
                    aria-selected={i === active}
                    onMouseMove={() => i !== active && setActive(i)}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => pick(it)}
                    className={cn(
                      "flex cursor-default items-center gap-2.5 px-2.5 py-1.5 text-[13.5px]",
                      i === active
                        ? "bg-accent text-accent-foreground"
                        : "text-foreground"
                    )}
                  >
                    <Icon
                      className="size-3.5 shrink-0 text-muted-foreground"
                      aria-hidden
                    />
                    <span className="min-w-0 truncate">{it.title}</span>
                    {it.hint && (
                      <span className="shrink-0 text-xs text-muted-foreground">
                        {it.hint}
                      </span>
                    )}
                    <span className="ml-auto flex shrink-0 items-center gap-1">
                      {it.shortcut &&
                        shortcutKeys(it.shortcut).map((k) => (
                          <Kbd
                            key={k}
                            className="h-[18px] min-w-[18px] text-[10px]"
                          >
                            {k}
                          </Kbd>
                        ))}
                      {i === active && (
                        <CornerDownLeftIcon
                          className="size-3.5 text-muted-foreground"
                          aria-hidden
                        />
                      )}
                    </span>
                  </div>
                </div>
              )
            })}
          </div>
        </DialogPrimitive.Popup>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
