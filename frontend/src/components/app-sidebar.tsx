// AppSidebar is the window's nav (card #135): the space switcher (card
// #393), Home, Talk (card #390), Boards, Notifications (card #377) and
// Held, then
// the boards and cards you pinned, and at the bottom this device, how the
// agents stand, you (your profile, card #393) and Settings. Its top button folds it to icons; the window
// remembers that on this device. On a phone or a narrow window (card #227)
// it is a drawer over the page instead: the header's menu button opens it,
// and a tap outside, Escape or going somewhere closes it.
// On the desktop its right edge drags (card #309): the width is remembered on
// this device, dragging well past the narrow end folds it, dragging a folded
// one out opens it again, and a double-click puts back the default width.

import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react"
import {
  BellIcon,
  GitMergeIcon,
  HomeIcon,
  InboxIcon,
  MessagesSquareIcon,
  NotebookPenIcon,
  PanelLeftIcon,
  XIcon,
  type LucideIcon,
} from "lucide-react"

import { Face } from "@/components/profile/face"
import { SettingsMenu } from "@/components/settings/settings-menu"
import { CallBar, useCallPart } from "@/components/talk/huddle"
import { leaveHuddle, setDeafened, setMuted } from "@/lib/call"
import { SpaceSwitcher } from "@/components/space-switcher"
import { Badge } from "@/components/ui/badge"
import { agentCounts } from "@/lib/agent-counts"
import { agentStatus, throughLine } from "@/lib/agent-status"
import { rosterGroups } from "@/lib/roster-groups"
import { AgentStrip, OnCards } from "@/components/cards/agent-strip"
import { CheckSlots } from "@/components/cards/check-slots"
import { useCommands } from "@/lib/commands"
import type { BoardSummary } from "@/lib/boards"
import type { CardSummary } from "@/lib/cards"
import { plainLine, type Roster } from "@/lib/irori"
import { go, type Section } from "@/lib/nav"
import { boardsInSpace } from "@/lib/spaces"
import { useNarrow } from "@/lib/media"
import { previewFlag } from "@/lib/preview"
import {
  clampWidth,
  defaultWidth,
  dragTo,
  foldedWidth,
  maxWidth,
  minWidth,
  readWidth,
  saveWidth,
  type Drag,
} from "@/lib/sidebar-width"
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
  talk,
  desks = 0,
  unread = 0,
  section,
  boards,
  cards,
  openCard,
  onOpenCard,
  drawer = false,
  onDrawer,
  space,
  onSpace,
  onNewSpace,
  me,
}: {
  roster: Roster
  held: number
  onHeld: () => void
  // Threads answered and waiting on the person (card #390), Talk's count.
  talk: number
  // Merge desks with an open PR of ours (card #417, #625); the Desks
  // item shows while one has.
  desks?: number
  // Notifications the person hasn't read (card #377).
  unread?: number
  // Where the window is; null on a card's page.
  section: Section | null
  boards: BoardSummary[]
  // Every live card: the pinned ones are listed.
  cards: CardSummary[]
  // The card whose page is open, if any.
  openCard: string
  onOpenCard: (id: string) => void
  // Narrow windows: the drawer is open, and closing it.
  drawer?: boolean
  onDrawer?: (open: boolean) => void
  // The space Home and Talk show (card #393), and picking one.
  space: string
  onSpace: (id: string) => void
  // New or join a space, from the picker (card #689).
  onNewSpace: () => void
  // You, as your profile has you: the row that opens it.
  me: { name: string; color: string; avatar: string }
}) {
  const narrow = useNarrow()
  // The huddle this device is in (#654); a phone shows it above its tabs.
  const huddle = useCallPart((s) => s.mine)
  const [foldedPref, setFolded] = useState(readFolded)
  const [width, setWidth] = useState(readWidth)
  // A drag of the right edge in progress; the width transition is off
  // while it runs so the edge stays under the pointer.
  const [dragging, setDragging] = useState(false)
  const drag = useRef<{ left: number; at: Drag } | null>(null)
  // A drawer is never folded: it is the full nav, over the page.
  const folded = foldedPref && !narrow
  useEffect(() => {
    if (!narrow || !drawer) return
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDrawer?.(false)
    }
    window.addEventListener("keydown", key)
    return () => window.removeEventListener("keydown", key)
  }, [narrow, drawer, onDrawer])
  const fold = (f: boolean) => {
    setFolded(f)
    try {
      localStorage.setItem(foldedKey, f ? "1" : "0")
    } catch {
      // A private window keeps it for this run only.
    }
  }
  // The drag follows the pointer on the window rather than through pointer
  // capture, which WKWebView drops after the first move.
  const startDrag = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || drag.current) return
    const aside = e.currentTarget.parentElement
    if (!aside) return
    e.preventDefault()
    const d = {
      left: aside.getBoundingClientRect().left,
      at: { folded, width, catching: false, start: width } as Drag,
    }
    drag.current = d
    setDragging(true)
    // The page under the pointer would show its own cursor and select text.
    document.body.style.cursor = "col-resize"
    document.body.style.userSelect = "none"
    const move = (ev: PointerEvent) => {
      const next = dragTo(ev.clientX - d.left, d.at)
      if (next.folded !== d.at.folded) fold(next.folded)
      if (next.width !== d.at.width) setWidth(next.width)
      d.at = next
    }
    const end = () => {
      window.removeEventListener("pointermove", move)
      window.removeEventListener("pointerup", end)
      window.removeEventListener("pointercancel", end)
      window.removeEventListener("blur", end)
      drag.current = null
      setDragging(false)
      document.body.style.cursor = ""
      document.body.style.userSelect = ""
      saveWidth(d.at.width)
    }
    window.addEventListener("pointermove", move)
    window.addEventListener("pointerup", end)
    window.addEventListener("pointercancel", end)
    window.addEventListener("blur", end)
  }
  const resetWidth = () => {
    setWidth(defaultWidth)
    saveWidth(null)
    fold(false)
  }
  // The arrow keys move the edge too, a step of 16px (Shift: 64).
  const keyEdge = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 64 : 16
    let next: number
    if (e.key === "ArrowLeft") {
      if (folded) return
      if (width <= minWidth) {
        e.preventDefault()
        fold(true)
        return
      }
      next = width - step
    } else if (e.key === "ArrowRight") {
      if (folded) {
        e.preventDefault()
        fold(false)
        return
      }
      next = width + step
    } else if (e.key === "Home") next = minWidth
    else if (e.key === "End") next = maxWidth
    else return
    e.preventDefault()
    next = clampWidth(next)
    setWidth(next)
    saveWidth(next)
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
  // Yours (terminal agents) and On cards (the ones Irori started), card
  // #456. Card #512: no "free" count (a thread gets its own agent); a
  // blocked agent counts as blocked, not working or on cards.
  const groups = rosterGroups(agents, cards)
  const n = agentCounts(groups.yours, groups.blocked)
  const blockedOnCards = groups.onCards
    .flatMap((g) => g.agents)
    .filter((a) => groups.blocked.has(a.key)).length
  const onCardsNow = groups.spawned - blockedOnCards
  const blockedNow = n.blocked + blockedOnCards
  const onCardLine = (
    cardId: string,
    a: {
      key: string
      status: string
      deviceState?: string | null
      deviceVia?: string | null
      statusSince?: string
    }
  ) => {
    const w = cards
      .find((c) => c.id === cardId)
      ?.working.find((r) => r.key === a.key)
    // The same words as everywhere else (card #489).
    const doing = agentStatus(a, a.statusSince).text
    const via = a.deviceVia && throughLine(a.deviceVia)
    return [w?.where, doing, via].filter(Boolean).join(" · ")
  }
  const pinnedBoards = boardsInSpace(boards, space)
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

  // Shut, the drawer stays mounted off-screen: Settings lives in it.
  return (
    <>
      {narrow && drawer && (
        <div
          aria-hidden
          className="fixed inset-0 z-40 bg-black/70"
          onClick={() => onDrawer?.(false)}
        />
      )}
      <aside
        aria-label="Navigation"
        className={cn(
          "relative flex h-svh shrink-0 flex-col border-r border-border",
          !dragging && "transition-[width]",
          narrow
            ? cn(
                // Opaque (the theme's --sidebar is translucent glass, which let the page
                // show through on a phone) and clear of Android's status and
                // gesture bars (edge to edge, card #227).
                "fixed inset-y-0 left-0 z-50 w-[min(18rem,85vw)] bg-background pt-[var(--safe-top)] pb-[var(--safe-bottom)] shadow-xl transition-transform",
                !drawer && "invisible -translate-x-full"
              )
            : "bg-sidebar/60"
        )}
        style={narrow ? undefined : { width: folded ? foldedWidth : width }}
      >
        {!narrow && (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize the sidebar"
            aria-valuemin={minWidth}
            aria-valuemax={maxWidth}
            aria-valuenow={folded ? foldedWidth : width}
            tabIndex={0}
            title="Drag to resize; past the narrow end it folds. Double-click: default width"
            onPointerDown={startDrag}
            onDoubleClick={resetWidth}
            onKeyDown={keyEdge}
            className={cn(
              // Straddles the border: 8px to grab, a line when it is live.
              "group absolute inset-y-0 -right-1 z-10 w-2 cursor-col-resize touch-none outline-none",
              "after:absolute after:inset-y-0 after:left-1/2 after:w-0.5 after:-translate-x-1/2 after:bg-primary after:opacity-0 after:transition-opacity",
              "hover:after:opacity-60 focus-visible:after:opacity-100",
              dragging && "after:opacity-100"
            )}
          />
        )}
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
          {narrow ? (
            <button
              type="button"
              onClick={() => onDrawer?.(false)}
              aria-label="Close the menu"
              className="ml-auto flex size-11 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-ink"
            >
              <XIcon className="size-5" aria-hidden />
            </button>
          ) : (
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
          )}
        </div>

        <div className="shrink-0 px-2 pb-2">
          <SpaceSwitcher
            boards={boards}
            space={space}
            onSpace={onSpace}
            onNewSpace={onNewSpace}
            folded={folded}
          />
        </div>

        <nav
          aria-label="Sections"
          className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-x-hidden overflow-y-auto px-2 pb-3 mac:overscroll-none"
        >
          <NavItem
            icon={HomeIcon}
            label="Home"
            folded={folded}
            current={here({ kind: "home" })}
            onClick={() => go({ kind: "home" })}
          />
          <NavItem
            icon={MessagesSquareIcon}
            label="Talk"
            folded={folded}
            current={here({ kind: "talk", id: "" })}
            count={talk}
            countTone="default"
            countTitle="Threads an agent answered and DMs you haven't read"
            onClick={() => go({ kind: "talk", id: "" })}
          />
          {desks > 0 && (
            <NavItem
              icon={GitMergeIcon}
              label="Merge desks"
              folded={folded}
              current={here({ kind: "desk", repo: "" })}
              onClick={() => go({ kind: "desk", repo: "" })}
            />
          )}
          <NavItem
            icon={NotebookPenIcon}
            label="Notes"
            folded={folded}
            current={!openCard && section?.kind === "notes"}
            onClick={() => go({ kind: "notes" })}
          />
          <NavItem
            icon={BellIcon}
            label="Notifications"
            folded={folded}
            current={here({ kind: "notifications" })}
            count={unread}
            countTone="info"
            countTitle="Notifications you haven't read"
            onClick={() => go({ kind: "notifications" })}
          />
          <NavItem
            icon={InboxIcon}
            label="Held"
            folded={folded}
            count={held}
            countTone="warning"
            countTitle="Messages Irori is holding back"
            onClick={onHeld}
          />

          {pinnedBoards.length > 0 && (
            <Group label="Pinned spaces" folded={folded}>
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

        {!folded && (
          <section aria-label="Agents" className="shrink-0 px-2 pb-3">
            <h2 className="px-2.5 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
              Agents
            </h2>
            <div
              className="flex flex-wrap gap-1 px-2.5"
              title={[
                onCardsNow > 0 &&
                  `${onCardsNow} started by Irori for a card, each ends when its card is Done`,
                groups.starting > 0 &&
                  `${groups.starting} cards starting an agent`,
                groups.queued > 0 &&
                  `${groups.queued} cards queued for an agent: a device's free slot or a usage limit's reset`,
                `${n.total} of yours`,
                `${n.working} working${n.running ? ` (${n.running} running a job)` : ""}`,
                blockedNow > 0 &&
                  `${blockedNow} blocked: on a permission prompt, or on a card waiting for you`,
                n.free + n.ownWork > 0 && `${n.free + n.ownWork} idle`,
                n.silent && `${n.silent} idle but not answering`,
                n.offline && `${n.offline} on an offline device`,
              ]
                .filter(Boolean)
                .join("\n")}
            >
              {onCardsNow > 0 && (
                <Badge variant="info" className="normal-case tabular-nums">
                  {onCardsNow} on cards
                </Badge>
              )}
              {groups.starting > 0 && (
                <Badge variant="slate" className="normal-case tabular-nums">
                  {groups.starting} starting
                </Badge>
              )}
              {groups.queued > 0 && (
                <Badge variant="warning" className="normal-case tabular-nums">
                  {groups.queued} queued
                </Badge>
              )}
              {n.working > 0 && (
                <Badge variant="info" className="normal-case tabular-nums">
                  {n.working} working
                </Badge>
              )}
              {blockedNow > 0 && (
                <Badge variant="warning" className="normal-case tabular-nums">
                  {blockedNow} blocked
                </Badge>
              )}
              {n.silent > 0 && (
                <Badge variant="warning" className="normal-case tabular-nums">
                  {n.silent} not answering
                </Badge>
              )}
            </div>
            {groups.onCards.length > 0 ? (
              <>
                <RosterLabel>Yours</RosterLabel>
                <AgentStrip agents={groups.yours} />
                <RosterLabel>On cards</RosterLabel>
                <OnCards
                  groups={groups.onCards}
                  line={onCardLine}
                  onOpen={onOpenCard}
                />
              </>
            ) : (
              <AgentStrip agents={agents} />
            )}
            <CheckSlots />
          </section>
        )}

        {/* The huddle you're in (#654), on every page. */}
        {huddle && !narrow && (
          <CallBar
            mine={huddle}
            folded={folded}
            onOpen={() => go({ kind: "talk", id: huddle.convId })}
            onMute={(on) => void setMuted(on).catch(() => {})}
            onDeafen={(on) => void setDeafened(on).catch(() => {})}
            onLeave={() => void leaveHuddle().catch(() => {})}
          />
        )}
        <div
          className={cn(
            "flex shrink-0 items-center gap-2 border-t border-border py-3",
            folded ? "flex-col px-2" : "px-4"
          )}
        >
          <button
            type="button"
            onClick={() => go({ kind: "profile" })}
            aria-current={here({ kind: "profile" }) ? "page" : undefined}
            aria-label={folded ? "Your profile" : undefined}
            title="Your profile: name, colour and picture, and your spaces"
            className={cn(
              "flex min-w-0 items-center gap-2 rounded-md text-left text-xs hover:bg-secondary",
              folded ? "justify-center p-1" : "-ml-1.5 flex-1 px-1.5 py-1",
              here({ kind: "profile" }) && "bg-secondary"
            )}
          >
            <Face
              name={me.name}
              color={me.color}
              avatar={me.avatar}
              size="md"
            />
            {!folded && (
              <span className="grid min-w-0 flex-1">
                <span className="truncate font-medium text-ink">
                  {me.name || "You"}
                </span>
                <span
                  className="truncate text-muted-foreground"
                  title={device?.id}
                >
                  {device?.name || "this device"}
                </span>
              </span>
            )}
          </button>
          <SettingsMenu version={version} />
        </div>
      </aside>
    </>
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
  countTone?: "destructive" | "warning" | "default" | "info"
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
        "relative flex h-9 items-center gap-3 rounded-md text-left text-sm text-ink-2 hover:bg-secondary hover:text-ink pointer-coarse:h-11",
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
              countTone === "destructive"
                ? "bg-destructive"
                : countTone === "default"
                  ? "bg-primary"
                  : countTone === "info"
                    ? "bg-info"
                    : "bg-warning"
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

function RosterLabel({ children }: { children: ReactNode }) {
  return (
    <h3 className="px-2.5 pt-2 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </h3>
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
        // One column that may shrink below its rows' text (#309).
        "grid grid-cols-[minmax(0,1fr)] gap-0.5",
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
        "flex h-8 w-full min-w-0 items-center gap-2.5 rounded-md text-left text-sm text-ink-2 hover:bg-secondary hover:text-ink pointer-coarse:h-11",
        folded ? "justify-center px-0" : "px-2.5",
        current && "bg-secondary font-medium text-ink"
      )}
    >
      <span
        className={cn(
          "flex h-5 min-w-6 shrink-0 items-center justify-center rounded bg-secondary px-1 font-mono text-[10.5px] text-muted-foreground",
          // A long tag or number shrinks into the icon column when folded.
          folded && "max-w-10 overflow-hidden"
        )}
      >
        <span className={cn(folded && "truncate")}>{mark}</span>
      </span>
      {!folded && <span className="min-w-0 flex-1 truncate">{label}</span>}
    </button>
  )
}
