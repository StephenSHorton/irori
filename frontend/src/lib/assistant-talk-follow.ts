// The labelled lines in the Office Assistant chat (card #1062,
// docs/design/assistants-talk.md §8; one-assistant.md §9): what another
// person's assistant said to this person, and what this person's own
// assistant was asked or said, built from the space's #office-assistants
// channel. They are never rows and never stored as the assistant's words:
// every device rebuilds them from the channel (on load and on
// talk:changed) and the chat merges them into its list by time. They are
// never sent to the model; the prompt is built in Rust from rows.
//
//   answer  another person's assistant answered a question this person
//           asked (typed, or from the chat). Lights the dot.
//   needs   a question to this person's assistant waits on them: Let it
//           answer (Ask me), or Answer this (it couldn't). Lights the dot,
//           and raises one toast.
//   asked   this person's assistant answered someone on its own. Quiet.
//   held    an answer or a batch of questions held back: not built here,
//           it is an assistant.chat.update row (key held:<post>).

import type { ChatEntry } from "./assistant"
import { Events, AssistantService, BoardsService } from "./host"
import { inWails } from "./irori"
import {
  EventTalkChanged,
  talkApi,
  type Conversation,
  type Post,
} from "./conversations"

export type FromKind = "answer" | "needs" | "asked" | "held"
export const FromKinds: readonly FromKind[] = [
  "answer",
  "needs",
  "asked",
  "held",
]

// From says where a labelled line came from.
export type From = {
  kind: FromKind
  // The other side: the assistant that answered, or the one who asked
  // ("Juno · Eve's assistant", "Eve", "sample-otter, Eve's agent").
  name: string
  // The space's id and its #office-assistants channel.
  space: string
  conv: string
  // The question's post id: Let it answer and Open in Talk go by it.
  postId: string
  // The question (what was asked), "" when unknown.
  question: string
  // The space's name, "" when unknown.
  spaceName: string
  // On a needs line: `ask` (Let it answer) or `owner` (Answer this).
  waiting?: "ask" | "owner"
  // The question was posted by this person's own assistant (asked from the
  // chat): its answer raises the toast, since Talk doesn't ring for it.
  mine?: boolean
}

// MaxFromText is the longest a line's text shows.
export const MaxFromText = 1500

// clipFrom cuts plain text for a line, with an ellipsis.
export function clipFrom(text: string): string {
  const chars = [...text.trim()]
  return chars.length > MaxFromText
    ? chars.slice(0, MaxFromText).join("") + "…"
    : chars.join("")
}

// The spaces' assistant cards, as the host gives them.
export type TalkCard = {
  space: string
  card: string
  conv: string
  owner: string
}

export type Channel = {
  card: TalkCard
  spaceName: string
  conv: Conversation | undefined
  posts: readonly Post[]
}

const isMine = (by: string, myKey: string) =>
  by === "user" || by.startsWith("user:") || by === myKey

// buildLines is the labelled lines a channel says to this person, oldest
// first (answer, needs and asked; held comes as a row). Each has a stable
// id, f:<kind>:<post>, and its time (ms) from the post.
export function buildLines(channels: readonly Channel[]): ChatEntry[] {
  const out: ChatEntry[] = []
  for (const ch of channels) {
    const myKey = `agent:${ch.card.card}`
    const byId = new Map(ch.posts.map((p) => [p.postId, p]))
    const members = ch.conv?.members ?? []
    const nameOf = (key: string): string => {
      const m = members.find((x) => x.key === key)
      const n = m?.name?.trim()
      if (!n || n === "you") return ""
      return m?.kind === "agent" && m.owner && !n.includes(m.owner)
        ? `${n}, ${m.owner}'s agent`
        : n
    }
    const assistants = new Set(
      members.filter((m) => m.assistant).map((m) => m.key)
    )
    // The questions my assistant has answered for real.
    const answered = new Set(
      ch.posts
        .filter((p) => p.by === myKey && p.replyTo && !p.waiting)
        .map((p) => p.replyTo as string)
    )
    const line = (
      kind: FromKind,
      p: Post,
      q: Post | undefined,
      name: string,
      text: string,
      waiting?: "ask" | "owner"
    ) => {
      const from: From = {
        kind,
        name,
        space: ch.card.space,
        conv: ch.card.conv,
        postId: q?.postId ?? p.replyTo ?? p.postId,
        question: q ? clipFrom(q.text) : "",
        spaceName: ch.spaceName,
      }
      if (waiting) from.waiting = waiting
      if (q?.by === myKey) from.mine = true
      out.push({
        id: `f:${kind}:${p.postId}`,
        role: "assistant",
        text: clipFrom(text),
        from,
        at: Date.parse(p.at) || 0,
      })
    }
    for (const p of ch.posts) {
      const q = p.replyTo ? byId.get(p.replyTo) : undefined
      if (!p.replyTo || !q) continue
      if (p.by === myKey) {
        if (q.by === myKey) continue
        if (p.waiting) {
          if (answered.has(q.postId)) continue
          line(
            "needs",
            p,
            q,
            nameOf(q.by) || "Someone",
            needsText(p.waiting === "owner" ? "owner" : "ask"),
            p.waiting === "owner" ? "owner" : "ask"
          )
        } else line("asked", p, q, nameOf(q.by) || "Someone", p.text)
      } else if (assistants.has(p.by) && !p.waiting && isMine(q.by, myKey)) {
        line("answer", p, q, nameOf(p.by) || "An assistant", p.text)
      }
    }
  }
  return out.sort((a, b) => (a.at ?? 0) - (b.at ?? 0))
}

// needsText is what a needs line says under its question.
export function needsText(w: "ask" | "owner" | undefined): string {
  return w === "owner"
    ? "Your assistant can't say yes for you, so it told them it asked you."
    : "This space is set to Ask me, so your assistant waited."
}

// the toasts: which lines are new since the cursor (ms) and worth one. A
// needs line always; an answer when my own assistant asked (the question
// was posted by its card), since a question typed in Talk rings there.
export function toastsFor(
  lines: readonly ChatEntry[],
  cursor: number,
  cards: readonly TalkCard[]
): Array<{ conv: string; title: string; body: string }> {
  const out: Array<{ conv: string; title: string; body: string }> = []
  for (const e of lines) {
    const f = e.from
    if (!f || (e.at ?? 0) <= cursor) continue
    const card = cards.find((c) => c.conv === f.conv)
    const owner = card?.owner.trim()
    if (f.kind === "needs" || (f.kind === "answer" && f.mine))
      out.push({
        conv: f.conv,
        title: owner ? `${owner}'s assistant` : "Your assistant",
        body:
          f.kind === "needs"
            ? `${f.name} asked: ${f.question || "a question"}`
            : `${f.name} answered: ${clipFrom(e.text).slice(0, 120)}`,
      })
  }
  return out
}

const cursorKey = "irori.assistant.talk.cursor"

function readCursor(): number | null {
  try {
    const v = localStorage.getItem(cursorKey)
    return v === null ? null : Number(v) || 0
  } catch {
    return null
  }
}

function writeCursor(ms: number) {
  try {
    localStorage.setItem(cursorKey, String(ms))
  } catch {
    // Blocked storage: a toast may show again after a restart.
  }
}

// startTalkFollow reads the channels now and on every talk:changed, and
// hands each read's lines to onLines. It returns its stop. Outside the app
// there is no channel, so it says nothing.
export function startTalkFollow(onLines: (l: ChatEntry[]) => void): () => void {
  if (!inWails()) return () => {}
  let reading = false
  let again = false
  let stopped = false
  const read = async (): Promise<void> => {
    if (reading) {
      again = true
      return
    }
    reading = true
    try {
      do {
        again = false
        const cards = await AssistantService.TalkCards()
        if (stopped) return
        if (!cards.length) {
          onLines([])
          continue
        }
        const [convs, boards] = await Promise.all([
          talkApi.conversations(),
          BoardsService.List().catch(() => []),
        ])
        const channels: Channel[] = await Promise.all(
          cards.map(async (c) => ({
            card: c,
            spaceName: boards.find((b) => b.boardId === c.space)?.name ?? "",
            conv: convs.find((x) => x.convId === c.conv),
            posts: await talkApi.posts(c.conv).catch(() => [] as Post[]),
          }))
        )
        if (stopped) return
        const lines = buildLines(channels)
        const newest = lines.reduce((m, l) => Math.max(m, l.at ?? 0), 0)
        const cursor = readCursor()
        // The first read on a device only sets the mark: nothing old rings.
        if (cursor !== null)
          for (const t of toastsFor(lines, cursor, cards))
            void AssistantService.TalkToast(t.conv, t.title, t.body).catch(
              () => {}
            )
        if (cursor === null || newest > cursor)
          writeCursor(Math.max(newest, cursor ?? 0))
        onLines(lines)
      } while (again)
    } catch {
      // Not readable now: the next change reads again.
    } finally {
      reading = false
    }
  }
  const off = Events.OnShown(EventTalkChanged, () => void read())
  void read()
  return () => {
    stopped = true
    off()
  }
}

// ---- Let it answer ---------------------------------------------------------------

// A tap stays "asking" this long (ms) while the assistant's computer
// serves it; if the needs line is still there after, the tap can be made
// again (the computer was away, or the turn failed: its record says so).
export const AskingMs = 30_000

const asking = new Map<string, number>()

// isAsking says a tap on this question is still being served. Answer this
// (`draft`) and Let it answer (`let`) are separate taps.
export function isAsking(
  postId: string,
  now = Date.now(),
  how: "let" | "draft" = "let"
): boolean {
  const at = asking.get(`${how}:${postId}`)
  return at !== undefined && now - at < AskingMs
}

// letItAnswer taps Let it answer on a question: it appends the request,
// which the assistant's computer serves (any device may tap). One tap at a
// time per question: a second while it is asking does nothing. It returns
// why it failed, or "".
export async function letItAnswer(
  from: Pick<From, "postId" | "space">,
  now = Date.now(),
  how: "let" | "draft" = "let"
): Promise<string> {
  if (isAsking(from.postId, now, how)) return ""
  const key = `${how}:${from.postId}`
  asking.set(key, now)
  try {
    await AssistantService.AnswerRequest(from.postId, from.space, how)
    return ""
  } catch (err) {
    asking.delete(key)
    return whyFailed(err, "Couldn't ask your assistant just now.")
  }
}

// answerThis taps Answer this on a question that needs the owner: the
// assistant's computer drafts an answer, which comes back as a draft the
// owner reads in full and sends with one tap (card #1064).
export function answerThis(
  from: Pick<From, "postId" | "space">,
  now = Date.now()
): Promise<string> {
  return letItAnswer(from, now, "draft")
}

// whyFailed is a failure in words, or `fallback`.
export function whyFailed(err: unknown, fallback: string): string {
  return err instanceof Error && err.message
    ? err.message
    : typeof err === "string" && err
      ? err
      : fallback
}

// sendDraft sends an owner's-OK draft: the host posts exactly its text, once.
// It returns why it failed, or "".
export async function sendDraft(request: string): Promise<string> {
  try {
    await AssistantService.AnswerSend(request)
    return ""
  } catch (err) {
    return whyFailed(err, "Couldn't send it just now.")
  }
}

// dismissDraft closes an owner's-OK draft on every device.
export async function dismissDraft(request: string): Promise<string> {
  try {
    await AssistantService.AnswerDismiss(request)
    return ""
  } catch (err) {
    return whyFailed(err, "Couldn't close it just now.")
  }
}

// forgetAsking is for tests.
export function forgetAsking() {
  asking.clear()
}
