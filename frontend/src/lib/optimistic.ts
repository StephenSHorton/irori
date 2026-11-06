// Instant actions (card #644): a click changes the screen at once, the
// host saves it in the background, and a refusal (answered elsewhere, out
// of time) takes the change back and says why in a toast. One action per
// key at a time: a second click on the same thing while it saves does
// nothing. Answers with files are not sent this way; they wait, as an
// upload can fail on its own.

import type {
  CardBlocker,
  CardDetail,
  CardSummary,
  Notification,
} from "./models"
import { AgentRef, TaskState } from "./models"
import { pending as thePending, type Patch, type Pending } from "./pending"

// What a failed action shows: a title and the host's reason.
export type Toast = (title: string, reason: string) => void

let toastSink: Toast = (title, reason) => console.warn(`${title}: ${reason}`)

// setToast is where failures go: the mounted Toaster (main.tsx).
export function setToast(t: Toast) {
  toastSink = t
}

export function reasonOf(err: unknown): string {
  if (err instanceof Error) return err.message
  if (err && typeof err === "object" && "message" in err)
    return String((err as { message: unknown }).message)
  return String(err)
}

export type Action<T> = {
  // The thing acted on: a blocker, a card, an ask.
  key: string
  // How the screen changes now; none for an action shown another way.
  patch?: Omit<Patch, "key">
  send: () => Promise<T>
  // The toast's title on a refusal, e.g. "Your answer wasn't saved".
  failed: string
  onSaved?: (r: T) => void
  // After the change is taken back: restore a draft, say it inline.
  onFailed?: (reason: string) => void
}

export function createInstant(pending: Pending, toast: () => Toast) {
  const busy = new Set<string>()
  // instant runs one action; false when that key is still saving.
  return function instant<T>(a: Action<T>): boolean {
    if (busy.has(a.key)) return false
    busy.add(a.key)
    const token = a.patch ? pending.add({ ...a.patch, key: a.key }) : 0
    let p: Promise<T>
    try {
      p = a.send()
    } catch (err) {
      p = Promise.reject(err)
    }
    p.then(
      (r) => {
        busy.delete(a.key)
        if (a.patch) pending.saved(a.key, token)
        a.onSaved?.(r)
      },
      (err) => {
        busy.delete(a.key)
        if (a.patch) pending.undo(a.key, token)
        const reason = reasonOf(err)
        toast()(a.failed, reason)
        a.onFailed?.(reason)
      }
    )
    return true
  }
}

// instant is the window's runner, on the window's pending changes.
export const instant = createInstant(thePending, () => toastSink)

// The keys actions go by: one per thing acted on, the same wherever it is
// answered (Needs you, Notifications, the card page), so a second click
// on it anywhere waits for the first.
export const answerKey = (cardId: string, blockerId: string) =>
  `answer:${cardId}/${blockerId}`
export const closeKey = (cardId: string) => `close:${cardId}`
export const askKey = (askId: string) => `ask:${askId}`

// ---- patches ------------------------------------------------------------------

// The note item: an input_required card's question (sample-cards.ts).
const NoteItem = "note"
const Answered = "answered"
const Resolved = "resolved"

const blockerShut = (d: CardDetail, id: string) => {
  const b = d.blockers.find((x) => x.id === id)
  return (
    !b ||
    !!b.resolved ||
    (b.state !== "" && b.state !== "open" && b.state !== "replied")
  )
}

const isOver = (c: { state: string }) =>
  c.state === TaskState.Completed ||
  c.state === TaskState.Canceled ||
  c.state === TaskState.Failed

// openBelow: the summary shows at most one open item fewer than it did
// when the change first met it. The first summary it patches sets the
// mark, so a read that already counts the answer isn't lowered twice.
function openBelow() {
  const seen = new Map<string, number>()
  const mark = (c: CardSummary) => {
    if (!seen.has(c.id)) seen.set(c.id, Math.max(0, c.openBlockers - 1))
    return seen.get(c.id)!
  }
  return {
    apply: (c: CardSummary) => ({
      ...c,
      openBlockers: Math.min(c.openBlockers, mark(c)),
    }),
    agrees: (c: CardSummary) => seen.has(c.id) && c.openBlockers <= mark(c),
  }
}

// The coord's Done answer: the done check's option that closes the card.
const DoneAnswer = "Done"

// isDone: option is Done on a done check, the pick the host closes the
// card on (coord DONE_ANSWER); any other option goes back to the agent.
export function isDone(
  b: Pick<CardBlocker, "doneCheck" | "options">,
  option: number
): boolean {
  return (
    !!b.doneCheck && option >= 0 && b.options[option]?.answer === DoneAnswer
  )
}

// answerPatch is an answer to one of a card's items: option is the
// picked one's index, -1 for own words. Done on a done check closes the
// card; anything else leaves the item with the agent.
export function answerPatch(
  cardId: string,
  b: Pick<CardBlocker, "id" | "doneCheck" | "options">,
  option: number
): Omit<Patch, "key"> {
  if (b.id === NoteItem) {
    const back = (c: CardSummary) =>
      c.state === TaskState.InputRequired
        ? { ...c, state: TaskState.Working }
        : c
    return {
      cardId,
      detail: (d) => {
        const out = back(d) as CardDetail
        return out.blockers.some((x) => x.id === NoteItem)
          ? { ...out, blockers: out.blockers.filter((x) => x.id !== NoteItem) }
          : out
      },
      detailAgrees: (d) => d.state !== TaskState.InputRequired,
      summary: back,
      summaryAgrees: (c) => c.state !== TaskState.InputRequired,
    }
  }
  if (isDone(b, option)) {
    const done = <T extends CardSummary>(c: T): T =>
      isOver(c) ? c : { ...c, state: TaskState.Completed, ready: undefined }
    const at = new Date().toISOString()
    return {
      cardId,
      detail: (d) => {
        const out = done(d)
        return {
          ...out,
          blockers: out.blockers.map((x) =>
            x.id === b.id && !x.resolved && x.state !== Resolved
              ? {
                  ...x,
                  state: Resolved,
                  // "Marked done by you" until the real one comes.
                  resolved: {
                    answer: DoneAnswer,
                    option,
                    by: AgentRef({ name: "You" }),
                    at,
                    closed: false,
                    note: "",
                  },
                }
              : x
          ),
        }
      },
      detailAgrees: isOver,
      summary: done,
      summaryAgrees: isOver,
    }
  }
  const open = openBelow()
  return {
    cardId,
    detail: (d) =>
      blockerShut(d, b.id)
        ? d
        : {
            ...d,
            ready: b.doneCheck ? undefined : d.ready,
            blockers: d.blockers.map((x) =>
              x.id === b.id
                ? { ...x, state: Answered, missing: [], optionsAside: false }
                : x
            ),
          },
    detailAgrees: (d) => blockerShut(d, b.id),
    summary: (c) => {
      const out = open.apply(c)
      return b.doneCheck ? { ...out, ready: undefined } : out
    },
    summaryAgrees: open.agrees,
  }
}

// closePatch is Done or Cancel on a card by hand.
export function closePatch(
  cardId: string,
  state: TaskState
): Omit<Patch, "key"> {
  const close = <T extends CardSummary>(c: T): T =>
    isOver(c) ? c : { ...c, state, ready: undefined }
  return {
    cardId,
    detail: close,
    detailAgrees: isOver,
    summary: close,
    summaryAgrees: isOver,
  }
}

// askPatch is a pick on an agent's ask in Notifications.
export function askPatch(noteKey: string, choice: string): Omit<Patch, "key"> {
  return {
    noteKey,
    notification: (n: Notification) =>
      !n.ask || !n.ask.open
        ? n
        : { ...n, ask: { ...n.ask, open: false, answered: true, choice } },
    notificationAgrees: (n) => !n.ask?.open,
  }
}
