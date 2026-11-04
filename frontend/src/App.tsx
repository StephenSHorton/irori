import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"

import { AppHeader } from "@/components/app-header"
import { AppSidebar } from "@/components/app-sidebar"
import { CardPage } from "@/components/cards/card-detail"
import { HeldDialog, type HeldTab } from "@/components/cards/held-dialog"
import { NewCardDialog } from "@/components/cards/new-card-dialog"
import { PhoneTabs, phoneTab } from "@/components/phone-tabs"
import { useNarrow } from "@/lib/media"
import { cn } from "@/lib/utils"
import { WindDownDialog } from "@/components/wind-down-dialog"
import { MessageDialog } from "@/components/cards/message-dialog"
import { TalkPage, needsYou } from "@/components/talk/talk-page"
import { talkApi, unreadDms, useConversations } from "@/lib/conversations"
import { AgentCards } from "@/components/cards/agent-hover"
import { AskBox } from "@/components/cards/ask-box"
import { NeedsYou } from "@/components/cards/needs-you"
import { nobodyTook, needsOf } from "@/lib/needs"
import { cardRef } from "@/components/cards/bits"
import { idleLine, whyWaiting } from "@/lib/agent-counts"
import { mainRedText, mainReds, type MainRed } from "@/lib/main-red"
import { asFilter, EmptyCards, matches, type Filter } from "@/components/cards/card-grid"
import {
  BoardView,
  inScope,
  ScopeOwn,
  homeFilterKey,
  rememberedFilter,
  savedHomeFilter,
} from "@/components/cards/board-view"
import {
  cardFromHash,
  cardHash,
  openTarget,
  useCard,
  useCards,
  useDoneCards,
  useIdleReport,
  useThreads,
  withLive,
} from "@/lib/cards"
import { useBoards } from "@/lib/boards"
import { FocusComment, LiveRoster } from "@/lib/thread"
import { useNow, useRoster } from "@/lib/irori"
import { previewFlag } from "@/lib/preview"
import { onOpen } from "@/lib/settings"
import { nameLookup, SystemKey, useGone, useHeld, useInbox } from "@/lib/talk"
import { deskOn, deskQueue, inputsOf, useDesks } from "@/lib/desk"
import { DeskPage } from "@/components/desk/desk-page"
import { openSettings } from "@/components/settings/settings-menu"
import { SpawnNotice } from "@/components/home/spawn-notice"
import { UpdateBanner } from "@/updater"
import { useBoardView } from "@/components/canvas/switch"
import { BoardHeaderBar, BoardPage } from "@/components/boards/board-page"
import { BoardsPage } from "@/components/boards/boards-page"
import { NotesPage } from "@/components/notes/notes-page"
import { useNotesWaiting } from "@/lib/notes"
import { ProfilePage } from "@/components/profile/profile-page"
import { useMe } from "@/lib/profile"
import { AllSpaces, Personal, cardInSpace, threadsSpace, useSpace } from "@/lib/spaces"
import { NotificationsPage } from "@/components/notifications-page"
import { useNotifications, useUnreadNotifications } from "@/lib/notifications"
import { go, Home, sameSection, sectionFromHash, type Section } from "@/lib/nav"
import { CommandPalette, PaletteJump, paletteOpenKey } from "@/components/command-palette"
import { isMac, useCommands } from "@/lib/commands"
import { spawnedKeys } from "@/lib/roster-groups"

// PageState is the history entry of a card's page. fromGrid: the grid is
// the entry behind it, so Back is the browser's back.
type PageState = { card: string; fromGrid?: boolean }

// The app puts the grid's scroll back itself; the browser's own restore on
// back would fight it.
if ("scrollRestoration" in window.history)
  window.history.scrollRestoration = "manual"

// A page opened by a reload or a link straight to it gets the grid as the
// entry behind it, so Back, Esc and the browser's back all land on the grid
// the same way (and a second browser back leaves, as from the grid).
{
  const id = cardFromHash(window.location.hash)
  const st = window.history.state as PageState | null
  if (id && !st?.fromGrid) {
    const hash = window.location.hash
    window.history.replaceState(
      null,
      "",
      window.location.pathname + window.location.search
    )
    window.history.pushState(
      { card: id, fromGrid: true } satisfies PageState,
      "",
      hash
    )
  }
}

// App is the whole window (docs/design/v0.4-devices.md §9): cards of shared
// work, and a card's own page when one is opened. Messages, capture and
// the roster keep running underneath for agents and routing; the person
// reaches them through cards and notifications.
//
// The open card lives in the address ("#card/<id>"), so the browser's (or
// the mouse's) back, Esc and the page's Back return to the grid, with its
// filter and scroll where they were.
export function App() {
  const roster = useRoster()
  const desks = useDesks()
  // Threads show "<agent> is working…" from the live roster (card #299);
  // an agent's hover card shows its "Offer it cards" (card #453).
  const liveRoster = useMemo(
    () =>
      new Map(
        roster.agents.map((a) => [
          a.key,
          {
            status: a.status,
            updatedAt: a.updatedAt,
            lastTool: a.lastTool,
            offersCards: a.offersCards,
            spawned: a.spawned,
          },
        ])
      ),
    [roster.agents]
  )
  const { gone } = useGone()
  const { cards, loading, error } = useCards()
  // Home's idle line (card #341): a work shortage or a dispatch gap.
  const idle = useIdleReport(roster.agents)
  const { boards } = useBoards()
  // The space the sidebar's switcher picked (card #393): Home, Talk and
  // Boards show it. All spaces, the default, is everything.
  const [space, setSpace] = useSpace(boards)
  const shared = space !== AllSpaces && space !== Personal
  const spaceCards = useMemo(() => cards.filter((c) => cardInSpace(c, space)), [cards, space])
  // Agents Irori started for a card are never free for other work (§6).
  const spawned = useMemo(() => spawnedKeys(cards), [cards])
  // Home's own scope: your cards, or a shared space's.
  const homeScope = shared ? space : ScopeOwn
  const me = useMe()
  const now = useNow()
  // ?filter=done in the browser preview starts on that filter; otherwise
  // Home starts on its remembered board pick (card #199).
  const [filter, setFilter] = useState<Filter>(() => {
    const flag = previewFlag("filter")
    if (flag !== null) return asFilter(flag)
    try {
      return savedHomeFilter(localStorage.getItem(homeFilterKey))
    } catch {
      return "all"
    }
  })
  // pickHomeFilter is a click on Home's tabs or board row: shown now, and
  // All, Private or a board remembered for the next start (card #199).
  const pickHomeFilter = (f: Filter) => {
    setFilter(f)
    if (!rememberedFilter(f)) return
    try {
      localStorage.setItem(homeFilterKey, f)
    } catch {
      // A blocked storage only loses the memory.
    }
  }
  // Where the window is (card #96): Home (your own cards), Talk (card
  // #390), Notes (card #413), Boards, or one board's page; in the address
  // so back and forward move between them. ?boards in the preview starts
  // on Boards, ?talk on Talk, ?thread=<id> on Talk with that thread open,
  // and ?notes on Notes (?notes=<board tag> on that board's tab).
  const [section, setSection] = useState<Section>(() => {
    const thread = previewFlag("thread")
    if (thread) return { kind: "talk", id: thread }
    if (previewFlag("talk") !== null) return { kind: "talk", id: "" }
    const notesTab = previewFlag("notes")
    if (notesTab !== null) return { kind: "notes", tab: notesTab }
    return previewFlag("boards") !== null ? { kind: "boards" } : sectionFromHash(window.location.hash)
  })
  const notesWaiting = useNotesWaiting()
  // A board page's own tab (All, Needs a decision, Closed).
  // A board page starts on All; the preview's ?filter=owner:<id> starts it
  // on one member's cards (card #268).
  const [boardFilter, setBoardFilter] = useState<Filter>(() =>
    previewFlag("filter")?.startsWith("owner:")
      ? asFilter(previewFlag("filter"))
      : "all"
  )
  // Closed pages each page's own cards (card #161): Home's, or the board's.
  const doneScope = section.kind === "board" ? section.id : homeScope
  const done = useDoneCards(
    section.kind === "board" ? boardFilter === "done" : filter === "done",
    doneScope
  )
  // List or Canvas (card #93): the grid, or the board as a flow chart.
  const [view, setView] = useBoardView()
  // query is the search box's words (card #171): it narrows Home's and a
  // board page's cards, List, Canvas and Closed alike.
  const [query, setQuery] = useState("")
  const [open, setOpen] = useState(() => cardFromHash(window.location.hash))
  // The comment a notification opened its card at (card #177).
  const [focus, setFocus] = useState<{ card: string; comment: string } | null>(
    null
  )
  // last is the card last opened, marked in the grid on the way back. In
  // the browser preview ?from=<id> sets it, to see the grid as Back leaves
  // it.
  const [last, setLast] = useState(() => open || previewFlag("from") || "")
  const { detail, error: detailError } = useCard(open)
  // gridScroll is where the grid was scrolled when a card opened.
  const gridScroll = useRef(0)
  // cardsNow and filterNow are the latest, for a notification's Open.
  const cardsNow = useRef(cards)
  const filterNow = useRef(filter)
  useEffect(() => {
    cardsNow.current = cards
    filterNow.current = filter
  }, [cards, filter])
  // message is the delivery a notification's Open named, shown in a dialog.
  const [message, setMessage] = useState("")
  // The notification history (card #377): the sidebar's unread count and
  // the Notifications page.
  const unreadNotifications = useUnreadNotifications()
  // The Held list: open, its tab, and the one sender it shows ("" = all).
  const [heldOpen, setHeldOpen] = useState(false)
  const [heldTab, setHeldTab] = useState<HeldTab>("waiting")
  const [heldFrom, setHeldFrom] = useState("")
  // New card (card #116): its dialog is open.
  const [newCard, setNewCard] = useState(() => previewFlag("newcard") !== null)
  // Wind down all (card #220): the header's button and the tray's item.
  const [windDown, setWindDown] = useState(false)
  const [newBoard, setNewBoard] = useState(false)
  // Threads (card #116) live on the Talk page (card #390): a new thread
  // from Home's ask box opens there, as does a thread's #ref.
  const { threads: threadList } = useThreads(threadsSpace(space))
  // Channels and DMs (Talk 1, #389): Talk's list, and its count.
  const { conversations } = useConversations()
  const startedThread = (id: string) => go({ kind: "talk", id })
  const held = useHeld(heldOpen && heldTab === "history")
  // Home's Needs you (card #261): the agents' messages to the person.
  const inbox = useInbox()
  // deliveriesNow is the latest inbox, for a notification's Open.
  const deliveriesNow = useRef(inbox.deliveries)
  useEffect(() => {
    deliveriesNow.current = inbox.deliveries
  }, [inbox.deliveries])
  const name = nameLookup(
    roster.agents.map((a): [string, string] => [a.key, a.name]),
    [...held.held, ...held.history, ...inbox.deliveries].map((d): [string, string] => [
      d.message.from.key,
      d.message.from.name,
    ])
  )
  const openHeld = useCallback((from: string) => {
    setHeldFrom(from)
    setHeldTab("waiting")
    setHeldOpen(true)
  }, [])

  // openCard shows card id's page: a new history entry from the grid, the
  // same entry when another card's page is already showing.
  const openCard = useCallback((id: string) => {
    if (!id) return
    if (cardFromHash(window.location.hash)) {
      const was = window.history.state as PageState | null
      window.history.replaceState(
        { card: id, fromGrid: was?.fromGrid } satisfies PageState,
        "",
        cardHash(id)
      )
    } else {
      gridScroll.current = window.scrollY
      window.history.pushState(
        { card: id, fromGrid: true } satisfies PageState,
        "",
        cardHash(id)
      )
    }
    setOpen(id)
    setLast(id)
  }, [])

  // back returns to the grid: the browser's back when the grid is the
  // entry behind (popstate does the rest; a reload or deep link is given
  // one at start), else the same entry without the card.
  const back = useCallback(() => {
    const st = window.history.state as PageState | null
    if (st?.fromGrid) {
      window.history.back()
      return
    }
    window.history.replaceState(
      null,
      "",
      window.location.pathname + window.location.search
    )
    setOpen("")
    setSection(Home)
  }, [])

  useEffect(() => {
    const onPop = () => {
      const id = cardFromHash(window.location.hash)
      setOpen(id)
      if (id) setLast(id)
      // A card's page keeps the section behind it; anything else names one.
      else {
        setSection(sectionFromHash(window.location.hash))
        setBoardFilter("all")
      }
    }
    window.addEventListener("popstate", onPop)
    return () => window.removeEventListener("popstate", onPop)
  }, [])

  // A card's page starts at the top; the grid comes back where it was,
  // with the card it left from focused.
  const shownOpen = useRef(open)
  useLayoutEffect(() => {
    if (shownOpen.current === open) return
    const wasGrid = !shownOpen.current
    shownOpen.current = open
    if (open) {
      if (wasGrid) window.scrollTo(0, 0)
      return
    }
    window.scrollTo(0, gridScroll.current)
    // The card it left from, else (a filter hides it) the filter in use.
    ;(
      document.querySelector<HTMLElement>(
        `[data-card="${CSS.escape(last)}"]`
      ) ??
      document.querySelector<HTMLElement>(
        '[aria-label^="Filter cards"] [aria-pressed="true"]'
      )
    )?.focus({ preventScroll: true })
  }, [open, last])

  // ?from= in the preview: the grid as Back leaves it, the card focused.
  useEffect(() => {
    if (open || !last || loading) return
    if (previewFlag("from") !== last) return
    document
      .querySelector<HTMLElement>(`[data-card="${CSS.escape(last)}"]`)
      ?.focus({ preventScroll: true })
    // Only on the first load.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading])

  // Esc on a card's page is Back, unless a dialog or menu takes it, a
  // terminal has the keyboard, or it is in a field (there it leaves the field first, so a draft isn't lost
  // to a stray key). Capture: it runs before a dialog closes itself.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || e.isComposing) return
      if (
        document.querySelector(
          '[role="dialog"],[role="alertdialog"],[role="menu"],[role="listbox"]'
        )
      )
        return
      const t = e.target instanceof HTMLElement ? e.target : null
      // A terminal's Esc is the program's (vim, less, PSReadLine): it
      // stays in the terminal (card #369).
      if (t?.closest(".rsz-view")) return
      if (t?.closest("input,textarea,select,[contenteditable]")) {
        t.blur()
        return
      }
      back()
    }
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  }, [open, back])

  // A notification's Open: a task ref opens that card's page; a held
  // message opens the Held list; an agent's message to the person opens
  // its DM in Talk (card #390; GitHub #8), else, when its sender isn't
  // known here, the message in a dialog.
  useEffect(
    () =>
      onOpen((target) => {
        const ref = target.ref ?? ""
        if (target.tab === "wind-down") {
          setWindDown(true)
          return
        }
        // A Talk notification (card #395): that channel or DM.
        if (target.tab === "talk" && ref) {
          go({ kind: "talk", id: ref })
          return
        }
        if (target.tab === "held") {
          openHeld(ref)
          return
        }
        if (target.tab === "inbox" && ref) {
          const from = deliveriesNow.current.find((d) => d.id === ref)?.message.from.key ?? ""
          if (!from || from.startsWith("user:") || from === SystemKey) {
            setMessage(ref)
            return
          }
          void talkApi
            .dm({ kind: "agent", key: from })
            .then((id) => go({ kind: "talk", id }))
            .catch(() => setMessage(ref))
          return
        }
        const to = openTarget(ref)
        if (!to) return
        const id = to.card
        setFocus(to.comment ? to : null)
        // A card the filter hides: show them all, so Back finds it.
        const c = cardsNow.current.find((x) => x.id === id)
        if (c && !matches(c, filterNow.current)) setFilter("all")
        openCard(id)
      }),
    [openCard, openHeld]
  )

  // Home is your own cards; a board's live on its page (card #96), and
  // its decisions show on Home as one line per board.
  const boardDecide = spaceCards.filter((c) => c.scope === "board" && c.openBlockers > 0)
  // Needs you leads Home (card #261): its cards and held messages. An
  // agent's message to you is a DM in Talk (card #390). Held ones are
  // yours, not a shared space's (card #393).
  const needs = useMemo(
    () => needsOf(spaceCards.filter((c) => inScope(c, homeScope)), shared ? [] : held.held),
    [spaceCards, homeScope, shared, held.held]
  )
  const needCards = useMemo(
    () => new Set(needs.filter((n) => n.kind === "check" || n.kind === "decision").map((n) => n.id)),
    [needs]
  )
  // The cards Home files under Stuck because nobody took them (card
  // #487): the idle line counts them, so it never says "no open work"
  // over them.
  const untaken = useMemo(
    () =>
      spaceCards
        .filter((c) => inScope(c, homeScope) && nobodyTook(withLive(c, liveRoster), now.getTime()))
        .map((c) => ({ id: c.id, short: cardRef(c) })),
    [spaceCards, homeScope, liveRoster, now]
  )
  // cardLabel names the card a held message is about: "#28", else "a card"
  // (one not on this board, or not loaded).
  const cardLabel = (ref: string) => {
    const id = ref.slice("task:".length)
    const c =
      cards.find((x) => x.id === id) ?? done.cards.find((x) => x.id === id)
    return c?.short ? `${c.tag ?? ""}#${c.short}` : "a card"
  }

  // What an agent's hover card lists (card #104): the live cards. The
  // provider sits unindented around the page to keep its diff small.
  // openThread is a #ref to a thread (card #144): Talk with that thread
  // open (card #390). One past Talk's list (an old resolved thread) opens
  // as a card's page instead.
  const openThread = useCallback(
    (id: string) => {
      if (!threadList.some((t) => t.id === id)) {
        openCard(id)
        return
      }
      const to: Section = { kind: "talk", id }
      if (open || !sameSection(section, to)) go(to)
    },
    [threadList, openCard, open, section]
  )
  // The command palette (card #213): Cmd/Ctrl+K or P from anywhere, over a
  // card's page and other dialogs too; Cmd+P no longer prints. Capture, so
  // it comes before a field or a dialog takes the key.
  const [palette, setPalette] = useState(false)
  useEffect(() => {
    const mac = isMac()
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || !paletteOpenKey(e, mac)) return
      e.preventDefault()
      e.stopPropagation()
      setPalette((o) => !o)
    }
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  }, [])
  // A jump from the palette closes the Held list and a message left open
  // under it (#218).
  useEffect(() => {
    const close = () => {
      setHeldOpen(false)
      setMessage("")
    }
    window.addEventListener(PaletteJump, close)
    return () => window.removeEventListener(PaletteJump, close)
  }, [])
  // goTo moves to a section from anywhere (a card's page included); going
  // where the window already is adds no history entry.
  const goTo = useCallback(
    (s: Section) => {
      if (!open && sameSection(section, s)) return
      go(s)
    },
    [open, section]
  )
  const onPaletteBoard = useCallback(
    (id: string) => goTo({ kind: "board", id }),
    [goTo]
  )
  // An agent in the palette: Home's cards narrowed to that agent's.
  const onPaletteAgent = useCallback(
    (name: string) => {
      setQuery(name)
      goTo(Home)
    },
    [goTo]
  )
  const onList = !open && (section.kind === "home" || section.kind === "board")
  useCommands([
    { id: "nav.home", title: "Home", group: "Go to", order: 1, keywords: ["my cards", "start"], run: () => goTo(Home) },
    { id: "nav.talk", title: "Talk", group: "Go to", order: 2, keywords: ["threads", "asks", "conversations", "messages"], run: () => goTo({ kind: "talk", id: "" }) },
    { id: "nav.notes", title: "Notes", group: "Go to", order: 2, keywords: ["rules", "gotchas", "decisions", "knowledge", "memory"], run: () => goTo({ kind: "notes", tab: "" }) },
    { id: "nav.boards", title: "Boards", group: "Go to", order: 2, keywords: ["shared", "projects"], run: () => goTo({ kind: "boards" }) },
    { id: "nav.notifications", title: "Notifications", group: "Go to", order: 3, keywords: ["mentions", "history", "alerts"], run: () => goTo({ kind: "notifications" }) },
    { id: "nav.held", title: "Held messages", group: "Go to", order: 3, keywords: ["inbox", "waiting", "history"], run: () => openHeld("") },
    {
      id: "card.new",
      title: section.kind === "board" && !open ? "New card on this board" : "New card",
      order: 1,
      keywords: ["create", "add", "task"],
      run: () => setNewCard(true),
    },
    {
      id: "board.new",
      title: "New board",
      order: 2,
      keywords: ["create", "add", "join", "invite"],
      run: () => {
        goTo({ kind: "boards" })
        setNewBoard(true)
      },
    },
    { id: "view.list", title: "Switch to List", keywords: ["grid", "view"], when: () => onList && view === "canvas", run: () => setView("list") },
    { id: "view.canvas", title: "Switch to Canvas", keywords: ["flow", "chart", "view"], when: () => onList && view === "list", run: () => setView("canvas") },
    { id: "card.back", title: "Back to the cards", shortcut: "Esc", keywords: ["close", "leave"], when: () => !!open, run: back },
    // The header's Wind down (card #220), from the palette (card #228); like
    // the header's button, not on the sample roster.
    {
      id: "agents.wind-down",
      title: "Wind down all agents",
      order: 3,
      keywords: ["stop", "pause", "save", "end", "shut down", "quit"],
      when: () => roster.source !== "sample",
      run: () => setWindDown(true),
    },
  ])

  const agentCards = useMemo(
    () => ({ cards, onOpen: openCard, threads: threadList, onOpenThread: openThread }),
    [cards, openCard, threadList, openThread]
  )

  // The nav drawer on a narrow window (card #227): it closes when you go
  // somewhere, as a phone's drawer does.
  const [drawer, setDrawer] = useState(false)
  const where = open ? `card:${open}` : `${section.kind}:${section.kind === "board" ? section.id : ""}`
  const [drawerAt, setDrawerAt] = useState(where)
  if (drawerAt !== where) {
    setDrawerAt(where)
    setDrawer(false)
  }

  // A phone (card #395, mockup talk-5): Home, Talk and Boards are tabs at
  // the bottom, and an open conversation is the whole screen, its own
  // header carrying Back.
  const narrow = useNarrow()
  const talkWaiting = threadList.filter(needsYou).length + unreadDms(conversations)
  const fullScreen = narrow && !open && section.kind === "talk" && !!section.id

  return (
    <AgentCards.Provider value={agentCards}>
    <LiveRoster.Provider value={liveRoster}>
    {/* No background of its own: the body paints the backdrop the glass
        shows (design/irori-ds.css, #266). */}
    <div className="flex h-svh overflow-hidden">
      {/* The sidebar is the nav (card #135); the header is the page's. */}
      <AppSidebar
        roster={roster}
        held={held.held.length}
        talk={talkWaiting}
        desks={{
          on: desks.desks.filter(deskOn).length,
          needYou: desks.desks
            .filter(deskOn)
            .reduce((n, d) => n + deskQueue(inputsOf(d)).filter((l) => l.state === "outside").length, 0),
        }}
        onHeld={() => openHeld("")}
        unread={unreadNotifications}
        section={open ? null : section}
        notesWaiting={notesWaiting}
        boardsNeedYou={boardDecide.length}
        boards={boards}
        cards={spaceCards}
        openCard={open}
        onOpenCard={openCard}
        drawer={drawer}
        onDrawer={setDrawer}
        space={space}
        onSpace={setSpace}
        me={me}
      />
      <div className="flex min-w-0 flex-1 flex-col">
      {fullScreen ? (
        <div aria-hidden className="shrink-0 pt-[var(--safe-top)]" />
      ) : (
      <AppHeader
        onMenu={() => setDrawer(true)}
        roster={roster}
        title={
          open
            ? "Card"
            : section.kind === "boards"
              ? "Boards"
              : section.kind === "talk"
                ? "Talk"
                : section.kind === "notes"
                  ? "Notes"
                : section.kind === "profile"
                  ? "Profile"
                : section.kind === "desk"
                  ? "Merge desks"
                : section.kind === "notifications"
                  ? "Notifications"
                  : "Home"
        }
        bar={
          !open && section.kind === "board" ? (
            <BoardHeaderBar key={section.id} boardId={section.id} />
          ) : undefined
        }
        onNewCard={() => setNewCard(true)}
        onNewBoard={() => setNewBoard(true)}
        onWindDown={() => setWindDown(true)}
        section={open ? null : section}
      />
      )}
      <main
        className={cn(
          "mx-auto flex min-h-0 w-full max-w-[1920px] flex-1 flex-col gap-3.5 sm:px-6",
          fullScreen ? "px-0 pt-0" : "px-4 pt-[18px]",
          !narrow && "pb-[var(--safe-bottom)]"
        )}
      >
        <UpdateBanner />
        {error && (
          <p className="glass-panel border-destructive/50! px-3 py-2 text-sm text-destructive" role="alert">
            Could not load the cards: {error}
          </p>
        )}
        {open ? (
          <Scroll>
          <FocusComment.Provider
            value={focus && focus.card === open ? focus.comment : ""}
          >
          <CardPage
            key={open}
            summary={
              cards.find((c) => c.id === open) ??
              done.cards.find((c) => c.id === open)
            }
            detail={detail}
            error={detailError}
            gone={gone}
            now={now}
            onBack={back}
            onOpen={openCard}
          />
          </FocusComment.Provider>
          </Scroll>
        ) : section.kind === "talk" ? (
          <TalkPage
            threads={threadList}
            conversations={conversations}
            agents={roster.agents.map((a) => ({ key: a.key, name: a.name, status: a.status }))}
            open={section.id}
            onOpen={(id) => go({ kind: "talk", id })}
            now={now}
            why={whyWaiting(roster.agents.filter((a) => !spawned.has(a.key)))}
            board={threadsSpace(space)}
          />
        ) : section.kind === "notes" ? (
          <Scroll>
            <NotesPage
              tab={section.tab}
              boards={boards}
              roster={roster}
              onTab={(tab) => go({ kind: "notes", tab })}
              onOpen={openCard}
              now={now}
            />
          </Scroll>
        ) : section.kind === "profile" ? (
          <Scroll>
            <ProfilePage
              boards={boards}
              agents={roster.agents.length}
              space={space}
              onSpace={setSpace}
            />
          </Scroll>
        ) : section.kind === "desk" ? (
          <DeskPage
            desks={desks.desks}
            open={section.repo}
            onOpen={(repo) => go({ kind: "desk", repo })}
            onOpenCard={openCard}
            onSettings={() => openSettings("desks")}
            nameOf={name}
            now={now}
          />
        ) : section.kind === "boards" ? (
          <Scroll>
            <BoardsPage
              space={space}
              cards={cards}
              now={now}
              newBoard={newBoard}
              onNewBoard={setNewBoard}
            />
          </Scroll>
        ) : section.kind === "notifications" ? (
          <Scroll>
            <NotificationsLive
              unreadTotal={unreadNotifications}
              now={now}
              onOpenCard={openCard}
              onOpenMessage={setMessage}
            />
          </Scroll>
        ) : section.kind === "board" ? (
          <BoardPage
            key={section.id}
            boardId={section.id}
            cards={cards}
            done={done}
            filter={boardFilter}
            onFilter={setBoardFilter}
            view={view}
            onView={setView}
            query={query}
            onQuery={setQuery}
            selected={last}
            onOpen={openCard}
            now={now}
          />
        ) : loading && cards.length === 0 ? (
          <p className="px-1 text-sm text-muted-foreground">Loading cards…</p>
        ) : (
          <BoardView
            key={homeScope}
            scope={homeScope}
            cards={spaceCards}
            done={done}
            filter={filter}
            onFilter={pickHomeFilter}
            view={view}
            onView={setView}
            query={query}
            onQuery={setQuery}
            selected={last}
            onOpen={openCard}
            now={now}
            header={
              <>
                <MainRedLines reds={mainReds(cards)} onOpen={openCard} />
                <SpawnNotice />
                <AskBox onThread={startedThread} board={threadsSpace(space)} />
                <IdleLine text={idleLine(idle, untaken)} />
              </>
            }
            lead={
              <NeedsYou
                needs={needs}
                now={now}
                name={name}
                onOpenCard={openCard}
              />
            }
            rows
            listed={needCards}
            notice={<BoardsNeedYou cards={boardDecide.filter((c) => !c.ours)} boards={boards} />}
            empty={<EmptyCards />}
          />
        )}
      </main>
      {narrow && (
        <PhoneTabs at={phoneTab(section)} talk={talkWaiting} onGo={goTo} />
      )}
      </div>
      {windDown && (
        <WindDownDialog
          agents={{
            here: roster.agents.filter((a) => a.deviceId === roster.device?.id).length,
            all: roster.agents.length,
          }}
          onClose={() => setWindDown(false)}
        />
      )}
      <NewCardDialog
        open={newCard}
        onClose={() => setNewCard(false)}
        onMade={openCard}
        spawned={spawned}
        board={
          section.kind === "board" && !open
            ? boards.find((b) => b.boardId === section.id)
            : // Home in a shared space makes its cards there (card #393).
              !open && shared && section.kind === "home"
              ? boards.find((b) => b.boardId === space)
              : undefined
        }
      />
      {heldOpen && (
        <HeldDialog
          state={held}
          tab={heldTab}
          onTab={setHeldTab}
          from={heldFrom}
          onFrom={setHeldFrom}
          name={name}
          cardLabel={cardLabel}
          onCard={(id) => {
            setHeldOpen(false)
            openCard(id)
          }}
          now={now}
          onClose={() => setHeldOpen(false)}
        />
      )}
      {palette && (
        <CommandPalette
          open
          onOpenChange={setPalette}
          cards={cards}
          boards={boards}
          agents={roster.agents}
          onCard={openCard}
          onBoard={onPaletteBoard}
          onAgent={onPaletteAgent}
          onThread={openThread}
        />
      )}
      {message && (
        <MessageDialog
          key={message}
          id={message}
          name={name}
          now={now}
          onClose={() => setMessage("")}
        />
      )}
    </div>
    </LiveRoster.Provider>
    </AgentCards.Provider>
  )
}

// BoardsNeedYou keeps a board's decisions from hiding off Home: one line
// naming each board with decisions Home doesn't list as cards (card #237:
// cards you or your agents created or touched are on Home already).
function BoardsNeedYou({
  cards,
  boards,
}: {
  cards: { board?: string | null }[]
  boards: { boardId: string; name: string }[]
}) {
  const by = new Map<string, number>()
  for (const c of cards) if (c.board) by.set(c.board, (by.get(c.board) ?? 0) + 1)
  if (!by.size) return null
  return (
    <p className="flex flex-wrap items-center gap-x-3 gap-y-1 px-1 text-[12.5px] text-muted-foreground">
      <span>Waiting on you on your boards:</span>
      {[...by].map(([id, n]) => (
        <button
          key={id}
          type="button"
          className="text-destructive underline-offset-2 hover:underline"
          onClick={() => go({ kind: "board", id })}
        >
          {boards.find((b) => b.boardId === id)?.name || "a board"} ({n})
        </button>
      ))}
    </p>
  )
}

// MainRedLines lead Home while a test fails on clean main (card #436): one
// card owns each red test and every card that hit it waits on that card,
// so the person sees the bottleneck before anything else.
function MainRedLines({ reds, onOpen }: { reds: MainRed[]; onOpen: (id: string) => void }) {
  if (reds.length === 0) return null
  return (
    <div className="flex flex-col gap-1" role="status">
      {reds.map((r) => (
        <button
          key={r.id}
          type="button"
          onClick={() => onOpen(r.id)}
          className="glass-panel border-destructive/50! px-3 py-2 text-left text-sm text-destructive hover:underline underline-offset-2"
        >
          {mainRedText(r)}
        </button>
      ))}
    </div>
  )
}

// IdleLine says whether idle agents mean there is no open work or that
// cards aren't reaching them (card #341), so a shortage isn't mistaken for
// a dispatch failure.
function IdleLine({ text }: { text: string }) {
  if (!text) return null
  return <p className="px-1 text-[12.5px] text-muted-foreground">{text}</p>
}

export default App

// Scroll is a page that isn't the Board (a card, Boards, a shared board):
// it scrolls by itself under the header (card #170).
function Scroll({ children }: { children: ReactNode }) {
  return (
    <div className="-mx-1 grid min-h-0 flex-1 content-start gap-3.5 overflow-y-auto px-1 pb-10">
      {children}
    </div>
  )
}

// NotificationsLive is the Notifications page with its list, read only
// while the page is open (0.11.1: the sidebar's badge reads a count).
function NotificationsLive(
  props: Omit<Parameters<typeof NotificationsPage>[0], "list" | "loading" | "error">
) {
  const { list, loading, error } = useNotifications()
  return <NotificationsPage {...props} list={list} loading={loading} error={error} />
}
