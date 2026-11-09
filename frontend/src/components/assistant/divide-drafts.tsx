// The drafts a Divide reply ends with (card #1054; docs/design/call-agents.md
// §9): cards to make and items to settle. They show with the exact text a
// card would get; nothing is made or changed until the person taps Make cards
// or Apply. The huddle's canvas is read live, so an item another person
// carded since the reply is left out.

import { useMemo, useState } from "react"
import { CheckIcon, PlusIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { useTranscript } from "@/components/talk/transcript-pane"
import { usePersonalBoards, useBoards } from "@/lib/boards"
import { cards } from "@/lib/cards"
import { useConversations } from "@/lib/conversations"
import {
  applySettle,
  boardChoices,
  cardText,
  defaultChoice,
  markCarded,
  refOfTask,
  shownCards,
  shownSettle,
  suggestedChoice,
  useDivide,
  type CardDraft,
  type SettleDraft,
} from "@/lib/huddle-divide"
import { errorText } from "@/lib/talk"

type Made = { ref: string; failed: number }

export function DivideDrafts({
  cards: drafts,
  settle,
}: {
  cards: CardDraft[]
  settle: SettleDraft | null
}) {
  const divide = useDivide()
  if (!drafts.length && !settle) return null
  if (!divide)
    return (
      <p
        className="rounded-lg border border-dashed border-border px-3 py-2 text-[12.5px] text-muted-foreground"
        data-divide-idle=""
      >
        These drafts were for a huddle. Open its page and tap Divide with my
        assistant to make them.
      </p>
    )
  return (
    <Drafts
      callId={divide.callId}
      title={divide.title}
      drafts={drafts}
      settle={settle}
    />
  )
}

function Drafts({
  callId,
  title,
  drafts,
  settle,
}: {
  callId: string
  title: string
  drafts: CardDraft[]
  settle: SettleDraft | null
}) {
  const { t, error, reload } = useTranscript(callId, true)
  const { conversations } = useConversations()
  const { boards } = useBoards()
  const personal = usePersonalBoards().boards
  const items = useMemo(() => t?.canvas ?? [], [t])
  const spaceId = conversations.find((c) => c.convId === t?.convId)?.spaceId
  const choices = useMemo(
    () => boardChoices(personal, boards),
    [personal, boards]
  )
  const fallback = defaultChoice(choices, spaceId)
  // The board each draft goes on, by key; the picker overrides the
  // assistant's suggestion, which overrides where the huddle lives.
  const [picked, setPicked] = useState<Record<number, string>>({})
  const [made, setMade] = useState<Record<number, Made>>({})
  const [busy, setBusy] = useState("")
  const [problem, setProblem] = useState("")
  const [applied, setApplied] = useState("")

  if (!t)
    return (
      <p className="text-[12.5px] text-muted-foreground">
        {error ? `Could not read the canvas: ${error}` : "Reading the canvas…"}
      </p>
    )
  const live = !t.over
  // A draft stays in the list once made, so it can say so.
  const shown = drafts
    .map((c, at) => ({ c, at }))
    .filter(({ c, at }) => made[at] || shownCards([c], items).length > 0)
    .map(({ c, at }) => ({
      c: shownCards([c], items)[0] ?? { ...c, ready: [], skipped: [] },
      at,
    }))
  const list = shownSettle(settle, items)
  const choiceOf = (i: number, c: CardDraft) => {
    const key = picked[i]
    return (
      choices.find((x) => x.key === key) ??
      suggestedChoice(choices, c.board) ??
      fallback
    )
  }
  const todo = shown.filter(({ at }) => !made[at])

  const makeAll = async () => {
    if (busy || live) return
    setBusy("cards")
    setProblem("")
    try {
      for (const { c, at } of todo) {
        const where = choiceOf(at, c)
        const r = await cards.create({
          title: c.title,
          objective: cardText(c, title),
          board: where?.board || undefined,
          spaceBoard: where?.spaceBoard || undefined,
        })
        const ref = refOfTask(r.task)
        const m = await markCarded(
          callId,
          ref,
          c.ready.map((i) => i.itemId)
        )
        setMade((o) => ({ ...o, [at]: { ref, failed: m.failed.length } }))
        if (m.failed.length)
          setProblem(
            `Made ${ref}, but ${m.failed.length} of its items could not be marked: ${m.failed[0].why}`
          )
      }
    } catch (err) {
      setProblem(errorText(err))
    } finally {
      setBusy("")
      reload()
    }
  }

  const applyNow = async () => {
    if (busy || live) return
    setBusy("settle")
    setProblem("")
    try {
      const r = await applySettle(callId, list)
      setApplied(
        r.failed.length
          ? `Settled ${r.set}; ${r.failed.length} could not be: ${r.failed[0].why}`
          : `Settled ${r.set}.`
      )
    } catch (err) {
      setProblem(errorText(err))
    } finally {
      setBusy("")
      reload()
    }
  }

  return (
    <div className="grid gap-2.5" data-divide-drafts="">
      {live && (
        <p role="status" className="text-[12px] text-muted-foreground">
          This huddle is still on. Cards can be made once it is over.
        </p>
      )}
      {shown.map(({ c, at }) => (
        <section
          key={at}
          aria-label="Card draft"
          data-card-draft=""
          className="rounded-lg border border-border bg-background/60 px-3 py-2"
        >
          <p className="text-[13px] font-medium text-ink">{c.title}</p>
          {c.objective && (
            <p className="mt-0.5 text-[12.5px] whitespace-pre-wrap text-ink-2">
              {c.objective}
            </p>
          )}
          {c.ready.length > 0 && (
            <ul className="mt-1 grid gap-0.5 text-[12px] text-muted-foreground">
              {c.ready.map((i) => (
                <li key={i.itemId} className="truncate">
                  · {i.text}
                </li>
              ))}
            </ul>
          )}
          {made[at] ? (
            <p
              className="mt-1.5 flex items-center gap-1 text-[12px] font-medium text-success"
              data-made=""
            >
              <CheckIcon className="size-3.5" aria-hidden />
              Made {made[at].ref}
            </p>
          ) : (
            choices.length > 1 && (
              <label className="mt-1.5 flex items-center gap-1.5 text-[12px] text-muted-foreground">
                On
                <select
                  aria-label="Board for this card"
                  value={choiceOf(at, c)?.key ?? ""}
                  onChange={(e) =>
                    setPicked((o) => ({ ...o, [at]: e.target.value }))
                  }
                  className="h-7 min-w-0 flex-1 rounded-md border border-input bg-transparent px-1.5 text-[12px] text-ink"
                >
                  {choices.map((x) => (
                    <option key={x.key} value={x.key}>
                      {x.label}
                    </option>
                  ))}
                </select>
              </label>
            )
          )}
        </section>
      ))}
      {todo.length > 0 && (
        <Button
          size="sm"
          disabled={live || !!busy || choices.length === 0}
          onClick={() => void makeAll()}
          data-make-cards=""
        >
          <PlusIcon />
          {busy === "cards"
            ? "Making…"
            : todo.length === 1
              ? "Make card"
              : `Make ${todo.length} cards`}
        </Button>
      )}
      {(list.settle.length > 0 || list.drop.length > 0) && (
        <section
          aria-label="Settle list"
          data-settle-draft=""
          className="rounded-lg border border-border bg-background/60 px-3 py-2"
        >
          {list.settle.length > 0 && (
            <>
              <p className="text-[12px] font-medium text-muted-foreground">
                Settle
              </p>
              <ul className="mb-1 grid gap-0.5 text-[12.5px] text-ink-2">
                {list.settle.map((i) => (
                  <li key={i.itemId} className="truncate">
                    · {i.text}
                  </li>
                ))}
              </ul>
            </>
          )}
          {list.drop.length > 0 && (
            <>
              <p className="text-[12px] font-medium text-muted-foreground">
                Drop
              </p>
              <ul className="grid gap-0.5 text-[12.5px] text-ink-2">
                {list.drop.map((i) => (
                  <li key={i.itemId} className="truncate">
                    · {i.text}
                  </li>
                ))}
              </ul>
            </>
          )}
          <Button
            size="sm"
            variant="secondary"
            className="mt-2"
            disabled={live || !!busy}
            onClick={() => void applyNow()}
            data-apply=""
          >
            <CheckIcon />
            {busy === "settle" ? "Applying…" : "Apply"}
          </Button>
        </section>
      )}
      {applied && (
        <p role="status" className="text-[12px] text-success">
          {applied}
        </p>
      )}
      {problem && (
        <p role="alert" className="text-[12px] text-destructive">
          {problem}
        </p>
      )}
    </div>
  )
}
