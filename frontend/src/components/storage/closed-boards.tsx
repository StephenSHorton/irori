// ClosedBoards is Storage's list of closed boards (card #728 with #690):
// every board closed in Personal or a shared space, to look through and
// open. A closed board keeps its cards, read-only, in its space's log, so
// it has no size of its own; its done cards clear with Clear done cards on
// the log's page, and it reopens from its own page.

import { ChevronRightIcon } from "lucide-react"

import { useBoards, usePersonalBoards } from "@/lib/boards"
import { plainLine, relativeTime } from "@/lib/irori"
import { go } from "@/lib/nav"
import { Personal } from "@/lib/spaces"

// ClosedBoard is one closed board and the space it is in.
export type ClosedBoard = {
  space: string
  spaceName: string
  boardId: string
  name: string
  description: string
  closedAt: string
}

// useClosedBoards lists the closed boards on this device, newest closed
// first.
export function useClosedBoards(): {
  list: ClosedBoard[]
  error: string | null
} {
  const shared = useBoards()
  const personal = usePersonalBoards()
  const list: ClosedBoard[] = []
  for (const b of personal.boards)
    if (b.closed)
      list.push({
        space: Personal,
        spaceName: "Personal",
        boardId: b.boardId,
        name: b.name,
        description: b.description ?? "",
        closedAt: b.closedAt ?? "",
      })
  for (const s of shared.boards)
    for (const b of s.boards ?? [])
      if (b.closed)
        list.push({
          space: s.boardId,
          spaceName: s.name,
          boardId: b.boardId,
          name: b.name,
          description: b.description ?? "",
          closedAt: b.closedAt ?? "",
        })
  list.sort(
    (a, b) => Date.parse(b.closedAt || "0") - Date.parse(a.closedAt || "0")
  )
  return { list, error: shared.error ?? personal.error }
}

export function ClosedBoards({
  list,
  error,
  now,
}: {
  list: ClosedBoard[]
  error: string | null
  now: Date
}) {
  return (
    <section aria-label="Closed boards" className="grid gap-2">
      {error && <p className="text-xs text-destructive">{error}</p>}
      {list.length === 0 ? (
        <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-[13px] text-muted-foreground">
          No closed boards. Close a board from its ⋯ menu once you're done with
          it.
        </p>
      ) : (
        <ul
          className="overflow-hidden p-0! glass-panel"
          aria-label="Closed boards"
        >
          {list.map((b) => (
            <li
              key={`${b.space}/${b.boardId}`}
              className="border-b border-border last:border-b-0"
            >
              <button
                type="button"
                onClick={() =>
                  go({ kind: "board", id: b.space, board: b.boardId })
                }
                className="flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-secondary/50"
              >
                <span className="grid min-w-0 flex-1 gap-0.5">
                  <span className="truncate text-[13px] text-ink">
                    {plainLine(b.name)}
                  </span>
                  <span className="truncate text-[12px] text-muted-foreground">
                    {[
                      plainLine(b.spaceName),
                      b.closedAt
                        ? `closed ${relativeTime(new Date(b.closedAt), now)}`
                        : "closed",
                      plainLine(b.description),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </span>
                <ChevronRightIcon
                  className="size-4 shrink-0 text-muted-foreground"
                  aria-hidden
                />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
