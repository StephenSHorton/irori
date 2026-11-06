// BoardsPage is where boards live (card #96): each board a project (say
// ww-multiplayer) you share with people you invite. A board opens as its
// own page; here you see them all. The header's New board (card #126)
// makes a new one, or joins one with an invite someone sent. Everything
// is private: only invited people see a board.

import { cardTone, type Tone } from "@/components/cards/bits"
import { Face } from "@/components/cards/faces"
import { Button } from "@/components/ui/button"
import { Loading } from "@/components/ui/loading"
import { useBoards, useInvite, type BoardSummary } from "@/lib/boards"
import { AllSpaces, Personal, boardsInSpace, boardsOf } from "@/lib/spaces"
import { TaskState, type CardSummary } from "@/lib/cards"
import { plainLine, relativeTime } from "@/lib/irori"
import { go } from "@/lib/nav"
import { cn } from "@/lib/utils"

import { InvitePanel, NewBoardDialog } from "./parts"
import { PersonalBoards } from "./space-boards"

export function BoardsPage({
  cards,
  cardsLoading = false,
  now,
  newBoard,
  onNewBoard,
  space = AllSpaces,
}: {
  // The sidebar's space (card #393): only its boards show.
  space?: string
  cards: CardSummary[]
  // cardsLoading: the cards' first read hasn't landed (the tiles' bars).
  cardsLoading?: boolean
  now: Date
  // The header's New board dialog is open (card #126).
  newBoard: boolean
  onNewBoard: (open: boolean) => void
}) {
  const { boards: all, error, loaded, reload } = useBoards()
  const boards = boardsInSpace(all, space)
  const first = !loaded || cardsLoading
  const { invite, setInvite } = useInvite(true)
  const inviting = !!invite && invite.state !== ""
  return (
    <div className="grid gap-4">
      {space !== Personal && (
        <p className="max-w-2xl text-[13px] text-muted-foreground">
          A space is a group of people: their boards, Talk and agents. Spaces
          are private: only the people you invite see them.
        </p>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
      {/* An open invite sits over the list, never in its place (card
          #103): your boards stay one click away while it waits. */}
      {inviting && (
        <div className="glass-panel max-w-xl p-4">
          <InvitePanel
            s={invite}
            onClose={() => {
              setInvite(null)
              void reload()
            }}
          />
        </div>
      )}
      {/* All spaces lists Personal's boards too (card #558), as each
          shared space's. */}
      {space === AllSpaces && <PersonalBoards cards={cards} now={now} titled />}
      {space === AllSpaces && (first || boards.length > 0) && (
        <h2 className="px-1 text-base font-semibold">Spaces</h2>
      )}
      {space === Personal ? (
        <PersonalBoards cards={cards} now={now} />
      ) : (
        // Until the first reads of the spaces and the cards land (card
        // #598), the tiles' shape: not "No spaces yet", nor empty bars.
        <Loading loading={first} label="Loading the spaces" className="min-w-0">
          {first ? (
            <BoardTilesShape />
          ) : boards.length > 0 ? (
            <ul
              className="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(280px,1fr))]"
              aria-label="Your spaces"
            >
              {boards.map((b) => (
                <BoardTile key={b.boardId} board={b} cards={cards} now={now} />
              ))}
            </ul>
          ) : (
            <div className="grid justify-items-start gap-2">
              <p className="text-[13px] text-muted-foreground">
                No spaces yet. Make one, then invite people to it; or join one
                with an invite link someone sent you.
              </p>
              <Button size="sm" onClick={() => onNewBoard(true)}>
                + New space
              </Button>
            </div>
          )}
        </Loading>
      )}
      <NewBoardDialog
        open={newBoard}
        onClose={() => onNewBoard(false)}
        taken={all.map((b) => b.tag)}
        onMade={reload}
        // One invite at a time: joining waits until this one ends.
        join={!inviting}
        onJoined={setInvite}
      />
    </div>
  )
}

// BoardTilesShape is the spaces' skeleton: tiles in BoardTile's frame,
// plain markup with no hooks.
function BoardTilesShape() {
  const tiles = [
    ["MLG", "mlg", "Stephen, Blake, Casey", "MLG · Netcode"],
    ["Netcode", "net", "Stephen, Casey", "Netcode"],
    ["Irori", "irori", "Stephen", "Irori · Site"],
  ]
  return (
    <ul className="grid [grid-template-columns:repeat(auto-fill,minmax(280px,1fr))] gap-3">
      {tiles.map(([name, tag, people, lists]) => (
        <li key={tag} className="grid content-start gap-2 p-3 glass-panel">
          <span className="flex items-center gap-2">
            <span className="block size-2 shrink-0 rounded-full" />
            <span className="text-sm font-medium text-ink">{name}</span>
          </span>
          <span className="text-xs text-muted-foreground">{people}</span>
          <span className="text-xs text-ink-2">{lists}</span>
          <span className="grid gap-1">
            <span className="block h-1.5 w-full rounded-full bg-muted" />
            <span className="text-[11px] text-muted-foreground">
              2 working · 1 open · 3 done this week
            </span>
          </span>
          <span className="grid gap-1">
            {["Tune the boost pads", "Lobby sync for the boat race"].map(
              (t) => (
                <span key={t} className="flex items-center gap-2 text-[12.5px]">
                  <span className="block size-5 shrink-0" />
                  <span className="font-mono text-[11px] text-muted-foreground">
                    {tag}#3
                  </span>
                  <span className="text-ink-2">{t}</span>
                </span>
              )
            )}
          </span>
          <span className="text-xs text-muted-foreground">
            Active 4 min ago
          </span>
        </li>
      ))}
    </ul>
  )
}

// The state bar's segments (card #126), in the colours the cards' state
// chips use: blocked or waiting on someone, being worked, open (nobody on
// it), done this week.
const segTone: Record<Tone, string> = {
  destructive: "bg-destructive",
  info: "bg-info",
  slate: "bg-muted-foreground/45",
  success: "bg-success",
}

type Seg = { key: string; label: string; n: number; tone: Tone }

// boardSegs counts a board's live cards by state, as the cards' own chips
// colour them, plus the board's done this week.
export function boardSegs(mine: CardSummary[], doneWeek: number): Seg[] {
  let blocked = 0
  let working = 0
  let open = 0
  for (const c of mine) {
    const t = cardTone(c)
    if (t === "destructive") blocked++
    else if (t === "info") working++
    else if (t === "success") doneWeek++
    else if (c.state !== TaskState.Canceled && c.state !== TaskState.Failed) open++
  }
  return [
    { key: "working", label: "working", n: working, tone: "info" },
    { key: "open", label: "open", n: open, tone: "slate" },
    { key: "blocked", label: "blocked", n: blocked, tone: "destructive" },
    { key: "done", label: "done this week", n: doneWeek, tone: "success" },
  ]
}

// StateBar is the tile's thin bar of card counts by state, with the
// counts spelled out under it.
export function StateBar({ segs }: { segs: Seg[] }) {
  const total = segs.reduce((n, s) => n + s.n, 0)
  const shown = segs.filter((s) => s.n > 0)
  const text = shown.map((s) => `${s.n} ${s.label}`).join(" · ")
  return (
    <span className="grid gap-1">
      <span
        className="flex h-1.5 w-full overflow-hidden rounded-full bg-muted"
        role="img"
        aria-label={text || "No cards"}
      >
        {total > 0 &&
          shown.map((s) => (
            <span
              key={s.key}
              className={cn("h-full", segTone[s.tone])}
              style={{ width: `${(s.n / total) * 100}%` }}
            />
          ))}
      </span>
      <span className="text-[11px] tabular-nums text-muted-foreground">
        {text || "No cards yet"}
      </span>
    </span>
  )
}

// RecentCards is a tile's few most recently active cards, with their
// holders' faces.
export function RecentCards({ cards }: { cards: CardSummary[] }) {
  if (cards.length === 0) return null
  return (
    <ul className="grid gap-1" aria-label="Recently active cards">
      {cards.map((c) => (
        <li key={c.id} className="flex min-w-0 items-center gap-2 text-[12.5px]">
          {c.holder.who.key ? (
            <Face who={c.holder.who} noHover noDot className="size-5" />
          ) : (
            <span
              className="size-5 shrink-0 border border-dashed border-muted-foreground/40"
              title="Nobody holds it"
              aria-label="Nobody holds it"
            />
          )}
          <span className="font-mono text-[11px] text-muted-foreground">
            {c.tag ?? ""}#{c.short}
          </span>
          <span className="min-w-0 truncate text-ink-2">{plainLine(c.title)}</span>
        </li>
      ))}
    </ul>
  )
}

// BoardTile is one space on the Spaces page: name, tag, who is on it, a
// bar of its cards by state, its three most recently active cards with
// their holders' faces, when it last had activity, and how many cards wait
// on you (card #126); it opens the board's page.
function BoardTile({
  board: b,
  cards,
  now,
}: {
  board: BoardSummary
  cards: CardSummary[]
  now: Date
}) {
  const mine = cards.filter((c) => c.board === b.boardId)
  const decide = mine.filter((c) => c.openBlockers > 0).length
  const ok = b.ready && !b.warnings?.length
  const recent = [...mine]
    .sort((x, y) => (y.updatedAt > x.updatedAt ? 1 : y.updatedAt < x.updatedAt ? -1 : 0))
    .slice(0, 3)
  // The board's own last activity, else its newest live card's.
  const last = b.lastAt || recent[0]?.updatedAt
  return (
    <li>
      <button
        type="button"
        className="glass-panel grid h-full w-full content-start gap-2 p-3 text-left transition-colors hover:border-primary/60 focus-visible:outline-2 focus-visible:outline-primary"
        onClick={() => go({ kind: "board", id: b.boardId })}
      >
        <span className="flex items-center gap-2">
          <span className={cn("size-2 shrink-0 rounded-full", ok ? "bg-success" : "bg-warning")} aria-hidden />
          <span className="min-w-0 truncate text-sm font-medium text-ink">
            {plainLine(b.name) || "(unnamed)"}
          </span>
          {b.owner && (
            <span className="ml-auto rounded bg-muted px-1 text-[11px] text-muted-foreground">owner</span>
          )}
        </span>
        <span className="text-xs text-muted-foreground">
          {b.ready ? b.people.map(plainLine).join(", ") : "Arriving from your other device"}
        </span>
        {b.ready && (
          <span className="truncate text-xs text-ink-2">
            {boardsOf(b).map((x) => plainLine(x.name)).join(" · ")}
          </span>
        )}
        {b.ready && <StateBar segs={boardSegs(mine, b.doneWeek ?? 0)} />}
        <RecentCards cards={recent} />
        <span className="flex flex-wrap items-center gap-x-2 text-xs">
          {last && (
            <span className="text-muted-foreground">
              Active {relativeTime(new Date(last), now)}
            </span>
          )}
          {decide > 0 && (
            <span className="text-destructive">
              {decide === 1 ? "1 card needs" : `${decide} cards need`} a decision
            </span>
          )}
        </span>
        {b.warnings?.slice(0, 1).map((w, i) => (
          <span key={i} className="text-xs text-warning">
            {plainLine(w)}
          </span>
        ))}
      </button>
    </li>
  )
}
