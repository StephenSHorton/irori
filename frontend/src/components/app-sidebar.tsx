// AppSidebar is the window's nav (card #135): Home, Boards and Held, then
// the boards and cards you pinned, and at the bottom this device, how the
// agents stand, and Settings. Its top button folds it to icons; the window
// remembers that on this device.

import { useState, type ReactNode } from "react"
import {
  HomeIcon,
  InboxIcon,
  LayoutGridIcon,
  PanelLeftIcon,
  type LucideIcon,
} from "lucide-react"

import { SettingsMenu } from "@/components/settings/settings-menu"
import { Badge } from "@/components/ui/badge"
import { agentCounts } from "@/lib/agent-counts"
import { useCommands } from "@/lib/commands"
import type { BoardSummary } from "@/lib/boards"
import type { CardSummary } from "@/lib/cards"
import { plainLine, type Roster } from "@/lib/irori"
import { go, type Section } from "@/lib/nav"
import { previewFlag } from "@/lib/preview"
import { cn } from "@/lib/utils"

const foldedKey = "irori.sidebarFolded"

function readFolded(): boolean {
  // ?sidebar=folded starts it folded in the browser preview.
  if (previewFlag("sidebar") === "folded") return true
  try {
    return localStorage.getItem(foldedKey) === "1"
  } catch {
    return false
  }
}

export function AppSidebar({
  roster,
  held,
  onHeld,
  section,
  boardsNeedYou,
  boards,
  cards,
  openCard,
  onOpenCard,
}: {
  roster: Roster
  held: number
  onHeld: () => void
  // Where the window is; null on a card's page.
  section: Section | null
  // Cards on boards waiting on the person, the red count by Boards.
  boardsNeedYou: number
  boards: BoardSummary[]
  // Every live card: the pinned ones are listed.
  cards: CardSummary[]
  // The card whose page is open, if any.
  openCard: string
  onOpenCard: (id: string) => void
}) {
  const [folded, setFolded] = useState(readFolded)
  const fold = (f: boolean) => {
    setFolded(f)
    try {
      localStorage.setItem(foldedKey, f ? "1" : "0")
    } catch {
      // A private window keeps it for this run only.
    }
  }
  useCommands([
    {
      id: "sidebar.fold",
      title: folded ? "Open the sidebar" : "Fold the sidebar",
      keywords: ["nav", "collapse", "expand"],
      run: () => fold(!folded),
    },
  ])
  const { device, version, agents } = roster
  const n = agentCounts(agents)
  const pinnedBoards = boards
    .filter((b) => b.pinned)
    .sort((a, b) => (a.pinRank ?? 0) - (b.pinRank ?? 0))
  // Pinned cards in their order; a subtask shows under Pinned only on its
  // own, its parent's pin already lists the family on the board.
  const pinnedCards = cards
    .filter((c) => c.pinned)
    .sort((a, b) => (a.pinRank ?? 0) - (b.pinRank ?? 0))
  const here = (s: Section) =>
    !openCard &&
    !!section &&
    section.kind === s.kind &&
    (s.kind !== "board" || (section.kind === "board" && section.id === s.id))

  return (
    <aside
      aria-label="Navigation"
      className={cn(
        "flex h-svh shrink-0 flex-col border-r border-border bg-sidebar/60 transition-[width]",
        folded ? "w-14" : "w-60"
      )}
    >
      <div
        className={cn(
          "flex h-14 shrink-0 items-center gap-2",
          folded ? "justify-center px-2" : "px-4"
        )}
      >
        {!folded && (
          <span className="flex items-baseline gap-2">
            <span className="text-lg font-semibold tracking-tight text-ink">
              Irori
            </span>
            <span
              lang="ja"
              className="text-sm font-normal text-muted-foreground"
            >
              囲炉裏
            </span>
          </span>
        )}
        <button
          type="button"
          onClick={() => fold(!folded)}
          aria-label={folded ? "Open the sidebar" : "Fold the sidebar"}
          aria-expanded={!folded}
          title={folded ? "Open the sidebar" : "Fold the sidebar to icons"}
          className={cn(
            "rounded p-1.5 text-muted-foreground hover:bg-secondary hover:text-ink",
            !folded && "ml-auto"
          )}
        >
          <PanelLeftIcon className="size-4" aria-hidden />
        </button>
      </div>

      <nav
        aria-label="Sections"
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-3"
      >
        <NavItem
          icon={HomeIcon}
          label="Home"
          folded={folded}
          current={here({ kind: "home" })}
          onClick={() => go({ kind: "home" })}
        />
        <NavItem
          icon={LayoutGridIcon}
          label="Boards"
          folded={folded}
          current={here({ kind: "boards" })}
          count={boardsNeedYou}
          countTone="destructive"
          countTitle="Cards on your boards waiting on you"
          onClick={() => go({ kind: "boards" })}
        />
        <NavItem
          icon={InboxIcon}
          label="Held"
          folded={folded}
          count={held}
          countTone="warning"
          countTitle="Messages Irori's wake policy held"
          onClick={onHeld}
        />

        {pinnedBoards.length > 0 && (
          <Group label="Pinned boards" folded={folded}>
            {pinnedBoards.map((b) => (
              <PinRow
                key={b.boardId}
                mark={plainLine(b.tag).slice(0, 2)}
                label={plainLine(b.name) || "(unnamed)"}
                title={`${plainLine(b.name)} (${plainLine(b.tag)}#)`}
                folded={folded}
                current={here({ kind: "board", id: b.boardId })}
                onClick={() => go({ kind: "board", id: b.boardId })}
              />
            ))}
          </Group>
        )}
        {pinnedCards.length > 0 && (
          <Group label="Pinned cards" folded={folded}>
            {pinnedCards.map((c) => (
              <PinRow
                key={c.id}
                mark={`${c.tag ? `${plainLine(c.tag)}` : ""}#${plainLine(c.short)}`}
                label={plainLine(c.title)}
                title={plainLine(c.title)}
                folded={folded}
                current={openCard === c.id}
                onClick={() => onOpenCard(c.id)}
              />
            ))}
          </Group>
        )}
      </nav>

      <div
        className={cn(
          "flex shrink-0 items-center gap-2 border-t border-border py-3",
          folded ? "flex-col px-2" : "px-4"
        )}
      >
        {!folded && (
          <div className="grid min-w-0 flex-1 gap-0.5 text-xs">
            <span
              className="truncate font-medium text-ink-2"
              title={device?.id}
            >
              {device?.name || "this device"}
            </span>
            <span
              className="truncate text-muted-foreground tabular-nums"
              title={[
                `${n.working} working${n.running ? ` (${n.running} waiting on a command they started)` : ""}`,
                `${n.free} free: idle, so a thread or card can go to them`,
                n.silent && `${n.silent} idle but not answering`,
                n.offline && `${n.offline} on an offline device`,
              ]
                .filter(Boolean)
                .join("\n")}
            >
              {n.total} {n.total === 1 ? "agent" : "agents"}
              {n.working > 0 && (
                <span className="text-primary"> · {n.working} working</span>
              )}
              {" · "}
              {n.free} free
              {n.silent > 0 && (
                <span className="text-warning">
                  {" "}
                  · {n.silent} not answering
                </span>
              )}
              {version && (
                <span className="font-mono">
                  {" · "}
                  {version === "dev" || version === "sample"
                    ? version
                    : `v${version.replace(/^v/, "")}`}
                </span>
              )}
            </span>
          </div>
        )}
        <SettingsMenu version={version} />
      </div>
    </aside>
  )
}

function NavItem({
  icon: Icon,
  label,
  folded,
  current,
  count = 0,
  countTone,
  countTitle,
  onClick,
}: {
  icon: LucideIcon
  label: string
  folded: boolean
  current?: boolean
  count?: number
  countTone?: "destructive" | "warning"
  countTitle?: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={current ? "page" : undefined}
      aria-label={folded ? `${label}${count ? ` (${count})` : ""}` : undefined}
      title={folded ? label : undefined}
      className={cn(
        "relative flex h-9 items-center gap-3 rounded-md text-left text-sm text-ink-2 hover:bg-secondary hover:text-ink",
        folded ? "justify-center px-0" : "px-2.5",
        current && "bg-secondary font-medium text-ink"
      )}
    >
      <Icon className="size-4 shrink-0" aria-hidden />
      {!folded && <span className="flex-1 truncate">{label}</span>}
      {count > 0 &&
        (folded ? (
          <span
            aria-hidden
            className={cn(
              "absolute top-1.5 right-2 size-1.5 rounded-full",
              countTone === "destructive" ? "bg-destructive" : "bg-warning"
            )}
          />
        ) : (
          <Badge
            variant={countTone}
            className="tabular-nums"
            title={countTitle}
          >
            {count}
          </Badge>
        ))}
    </button>
  )
}

function Group({
  label,
  folded,
  children,
}: {
  label: string
  folded: boolean
  children: ReactNode
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className={cn(
        "grid gap-0.5",
        folded ? "mt-2 border-t border-border pt-2" : "mt-4"
      )}
    >
      {!folded && (
        <h2 className="px-2.5 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          {label}
        </h2>
      )}
      {children}
    </div>
  )
}

function PinRow({
  mark,
  label,
  title,
  folded,
  current,
  onClick,
}: {
  mark: string
  label: string
  title: string
  folded: boolean
  current?: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-current={current ? "page" : undefined}
      aria-label={folded ? title : undefined}
      className={cn(
        "flex h-8 items-center gap-2.5 rounded-md text-left text-sm text-ink-2 hover:bg-secondary hover:text-ink",
        folded ? "justify-center px-0" : "px-2.5",
        current && "bg-secondary font-medium text-ink"
      )}
    >
      <span className="flex h-5 min-w-6 shrink-0 items-center justify-center rounded bg-secondary px-1 font-mono text-[10.5px] text-muted-foreground">
        {mark}
      </span>
      {!folded && <span className="truncate">{label}</span>}
    </button>
  )
}
