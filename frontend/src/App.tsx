import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"

import { AppHeader } from "@/components/app-header"
import { CardPage } from "@/components/cards/card-detail"
import { HeldDialog, type HeldTab } from "@/components/cards/held-dialog"
import { MessageDialog } from "@/components/cards/message-dialog"
import { AskBox } from "@/components/cards/ask-box"
import {
  asFilter,
  CardGrid,
  DoneGrid,
  EmptyCards,
  DeviceBar,
  FilterBar,
  matches,
  shownCards,
  type Filter,
} from "@/components/cards/card-grid"
import {
  cardFromHash,
  cardHash,
  deviceChips,
  useCard,
  useCards,
  useDoneCards,
} from "@/lib/cards"
import { useNow, useRoster } from "@/lib/irori"
import { previewFlag } from "@/lib/preview"
import { onOpen } from "@/lib/settings"
import { nameLookup, useGone, useHeld } from "@/lib/talk"
import { UpdateBanner } from "@/updater"

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
  const { gone } = useGone()
  const { cards, loading, error } = useCards()
  const now = useNow()
  // ?filter=done in the browser preview starts on that filter.
  const [filter, setFilter] = useState<Filter>(() =>
    asFilter(previewFlag("filter"))
  )
  const done = useDoneCards(filter === "done")
  const [open, setOpen] = useState(() => cardFromHash(window.location.hash))
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
  // The Held list: open, its tab, and the one sender it shows ("" = all).
  const [heldOpen, setHeldOpen] = useState(false)
  const [heldTab, setHeldTab] = useState<HeldTab>("waiting")
  const [heldFrom, setHeldFrom] = useState("")
  const held = useHeld(heldOpen && heldTab === "history")
  const name = nameLookup(
    roster.agents.map((a): [string, string] => [a.key, a.name]),
    [...held.held, ...held.history].map((d): [string, string] => [
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
  }, [])

  useEffect(() => {
    const onPop = () => {
      const id = cardFromHash(window.location.hash)
      setOpen(id)
      if (id) setLast(id)
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

  // Esc on a card's page is Back, unless a dialog or menu takes it, or it
  // is in a field (there it leaves the field first, so a draft isn't lost
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
      if (t?.closest("input,textarea,select,[contenteditable]")) {
        t.blur()
        return
      }
      back()
    }
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  }, [open, back])

  // A notification's Open: a task ref opens that card's page; a message
  // (held, or sent to the person) opens in a dialog over the cards.
  useEffect(
    () =>
      onOpen((target) => {
        const ref = target.ref ?? ""
        if (target.tab === "held") {
          openHeld(ref)
          return
        }
        if (target.tab === "inbox" && ref) {
          setMessage(ref)
          return
        }
        if (!ref.startsWith("task:")) return
        const id = ref.slice("task:".length)
        // A card the filter hides: show them all, so Back finds it.
        const c = cardsNow.current.find((x) => x.id === id)
        if (c && !matches(c, filterNow.current)) setFilter("all")
        openCard(id)
      }),
    [openCard, openHeld]
  )

  const chips = deviceChips(cards)
  // A device filter whose device has no cards left falls back to All.
  const active: Filter =
    filter.startsWith("device:") && !chips.some((d) => filter === `device:${d.id}`)
      ? "all"
      : filter
  const { shown, context } = shownCards(cards, active)
  const decide = cards.filter((c) => c.openBlockers > 0).length
  // cardLabel names the card a held message is about: "#28", else "a card"
  // (one not on this board, or not loaded).
  const cardLabel = (ref: string) => {
    const id = ref.slice("task:".length)
    const c =
      cards.find((x) => x.id === id) ?? done.cards.find((x) => x.id === id)
    return c?.short ? `#${c.short}` : "a card"
  }

  return (
    <div className="min-h-svh bg-background">
      <AppHeader
        roster={roster}
        held={held.held.length}
        onHeld={() => openHeld("")}
      />
      <main className="mx-auto flex max-w-[1920px] flex-col gap-3.5 px-4 pt-[18px] pb-10 sm:px-6">
        <UpdateBanner />
        {error && (
          <p className="glass-panel border-destructive/50! px-3 py-2 text-sm text-destructive" role="alert">
            Could not load the cards: {error}
          </p>
        )}
        {open ? (
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
        ) : loading && cards.length === 0 ? (
          <p className="px-1 text-sm text-muted-foreground">Loading cards…</p>
        ) : cards.length === 0 && filter !== "done" ? (
          <>
            <AskBox onAsked={setLast} />
            <FilterBar
              filter={filter}
              onFilter={setFilter}
              decide={0}
              only={["all", "done"]}
            />
            <EmptyCards />
          </>
        ) : (
          <>
            <AskBox
              onAsked={(id) => {
                setLast(id)
                setFilter("all")
              }}
            />
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <FilterBar
                filter={active}
                onFilter={setFilter}
                decide={decide}
              />
              <DeviceBar filter={active} onFilter={setFilter} devices={chips} />
            </div>
            {filter === "done" ? (
              <DoneGrid
                done={done}
                selected={last}
                onSelect={openCard}
                now={now}
              />
            ) : shown.length ? (
              <CardGrid cards={shown} context={context} selected={last} onSelect={openCard} />
            ) : (
              <p className="px-1 text-[12.5px] text-muted-foreground">
                Nothing here with this filter.
              </p>
            )}
          </>
        )}
      </main>
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
  )
}

export default App
