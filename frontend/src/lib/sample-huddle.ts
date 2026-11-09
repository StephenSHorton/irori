// The browser preview's huddle canvas (card #1052): made-up items and
// assistants for the huddle page, and the writes a person makes on it, kept
// in memory. Inside the app none of this runs.

import type { TalkAssistant, TalkCanvasItem } from "../gen/bindings"

const MIN = 60_000

type Extra = Map<string, TalkCanvasItem[]>
const added: Extra = new Map()
const edits = new Map<
  string,
  { text?: string; status?: string; x?: number; y?: number; card?: string }
>()
const listeners = new Set<() => void>()

export function onSampleHuddle(f: () => void): () => void {
  listeners.add(f)
  return () => listeners.delete(f)
}

function changed() {
  for (const f of [...listeners]) f()
}

function item(p: Partial<TalkCanvasItem> & { itemId: string }): TalkCanvasItem {
  return {
    kind: "note",
    text: "",
    status: "",
    reply: "",
    refs: [],
    why: "",
    assistant: false,
    byName: "you",
    label: "",
    askerName: "",
    mine: true,
    askerMine: false,
    atMs: Date.now(),
    x: null,
    y: null,
    card: "",
    cardByName: "",
    cardMine: false,
    ...p,
  }
}

const juno = "Juno · Stephen's assistant"
const kit = "Kit · Spencer's assistant"

function baseItems(callId: string, now: number): TalkCanvasItem[] {
  if (callId === "sample-call-3") return []
  const live = callId.startsWith("preview-call-")
  const t = (m: number) => now - m * MIN
  if (callId === "sample-call-1") {
    return [
      item({
        itemId: "a1",
        kind: "note",
        text: "Spencer is at the PC around two; the build can wait until then.",
        assistant: true,
        label: kit,
        byName: "Spencer",
        mine: false,
        atMs: t(80),
      }),
    ]
  }
  const items: TalkCanvasItem[] = [
    item({
      itemId: "q1",
      kind: "question",
      text: "Which certificate signs the Windows installer for 0.10.0?",
      byName: "Spencer",
      mine: false,
      atMs: t(live ? 5 : 1500),
      status: "open",
      why: "name",
    }),
    item({
      itemId: "ans1",
      kind: "answer",
      text: "The EV certificate in the PC's token, per docs/releasing.md. The old one expires on the 14th.",
      reply: "q1",
      assistant: true,
      label: juno,
      byName: "you",
      askerName: "Spencer",
      mine: true,
      atMs: t(live ? 4 : 1498),
      why: "name",
    }),
    item({
      itemId: "d1",
      kind: "decision",
      text: "Sign on the PC, and notarize on the Mac, in that order.",
      byName: "Aiko",
      mine: false,
      atMs: t(live ? 3 : 1490),
      status: live ? "open" : "settled",
    }),
    item({
      itemId: "t1",
      kind: "task",
      text: "Renew the EV certificate before the 14th, then record the new thumbprint in releasing.md.",
      assistant: true,
      label: kit,
      byName: "Spencer",
      mine: false,
      atMs: t(live ? 2 : 1485),
      status: "open",
      why: "digest",
    }),
    item({
      itemId: "n1",
      kind: "note",
      text: "Notarization on the Mac has taken between four and nine minutes lately. If it passes ten, check Apple's status page before retrying. That is one of the things that cost an hour on the last release, together with the stapling step that fails silently when the network drops mid-way.",
      assistant: true,
      label: juno,
      byName: "you",
      mine: true,
      atMs: t(live ? 1 : 1480),
      why: "digest",
    }),
  ]
  if (!live)
    items.push(
      item({
        itemId: "t2",
        kind: "task",
        text: "Add the notarization status check to the release script.",
        byName: "Aiko",
        mine: false,
        atMs: t(1470),
        status: "open",
      }),
      item({
        itemId: "t3",
        kind: "task",
        text: "Write the Windows installer signing steps into docs/releasing.md.",
        byName: "Spencer",
        mine: false,
        atMs: t(1468),
        status: "carded",
        card: "#118",
        cardByName: "Spencer",
        cardMine: false,
      }),
      item({
        itemId: "q2",
        kind: "question",
        text: "Do we still need the old updater endpoint?",
        byName: "you",
        mine: true,
        atMs: t(1465),
        status: "open",
      }),
      item({
        itemId: "sum1",
        kind: "summary",
        text: "Signing happens on the PC, notarizing on the Mac. The EV certificate must be renewed before the 14th. Action items: renew the certificate (Spencer), add a notarization status check to the release script (Aiko).",
        refs: ["q1", "d1"],
        assistant: true,
        label: juno,
        byName: "you",
        mine: true,
        atMs: t(1460),
        why: "end",
      })
    )
  if (live)
    items.push(
      item({
        itemId: "ph1",
        kind: "answer",
        text: "Juno is looking: where the thumbprint is recorded…",
        reply: "q1",
        assistant: true,
        label: juno,
        askerName: "Aiko",
        mine: true,
        atMs: t(0.2),
        why: "name",
      })
    )
  return items
}

// sampleCanvas is huddle `callId`'s items as the preview shows them.
export function sampleCanvas(callId: string, now: number): TalkCanvasItem[] {
  const all = [...baseItems(callId, now), ...(added.get(callId) ?? [])]
  return all.map((i) => {
    const e = edits.get(`${callId}/${i.itemId}`)
    if (!e) return i
    return {
      ...i,
      text: e.text ?? i.text,
      status: e.status ?? i.status,
      x: e.x ?? i.x,
      y: e.y ?? i.y,
      card: e.card ?? i.card,
      cardByName: e.card ? "you" : i.cardByName,
      cardMine: e.card ? true : i.cardMine,
    }
  })
}

// sampleAssistants is the roster of a huddle that is on.
export function sampleAssistants(live: boolean): TalkAssistant[] {
  if (!live) return []
  return [
    {
      person: "user:you",
      name: "you",
      mine: true,
      state: "connected",
      why: "",
    },
    {
      person: "person:spencer",
      name: "Spencer",
      mine: false,
      state: "connected",
      why: "",
    },
    {
      person: "person:aiko",
      name: "Aiko",
      mine: false,
      state: "not connected",
      why: "needs a computer that's on",
    },
  ]
}

let seq = 0

export async function sampleAdd(
  callId: string,
  kind: string,
  text: string
): Promise<TalkCanvasItem> {
  const i = item({
    itemId: `mine-${++seq}`,
    kind,
    text,
    status: kind === "note" ? "" : "open",
    atMs: Date.now(),
  })
  added.set(callId, [...(added.get(callId) ?? []), i])
  changed()
  return i
}

export async function sampleSet(
  callId: string,
  itemId: string,
  text: string | null,
  status: string | null,
  x: number | null,
  y: number | null,
  card: string | null = null
): Promise<TalkCanvasItem> {
  const key = `${callId}/${itemId}`
  const e = edits.get(key) ?? {}
  if (text !== null) e.text = text
  if (status !== null) e.status = status
  if (x !== null) e.x = x
  if (y !== null) e.y = y
  if (card !== null) e.card = card
  edits.set(key, e)
  changed()
  const found = sampleCanvas(callId, Date.now()).find(
    (i) => i.itemId === itemId
  )
  if (!found) throw new Error("that item isn't on this canvas")
  return found
}

// sampleAsk writes the person's question, then a placeholder that becomes
// an answer a moment later, as an assistant's look does.
export async function sampleAsk(
  callId: string,
  text: string
): Promise<TalkCanvasItem> {
  const q = await sampleAdd(callId, "question", text)
  const ph = item({
    itemId: `ans-${++seq}`,
    kind: "answer",
    text: "Juno is looking: " + text.slice(0, 60) + "…",
    reply: q.itemId,
    assistant: true,
    label: juno,
    why: "ask",
    atMs: Date.now() + 1,
  })
  window.setTimeout(() => {
    added.set(callId, [...(added.get(callId) ?? []), ph])
    changed()
  }, 600)
  window.setTimeout(() => {
    edits.set(`${callId}/${ph.itemId}`, {
      text: "From what the huddle has said so far: the thumbprint goes in docs/releasing.md under Signing.",
    })
    changed()
  }, 2_400)
  return q
}

// sampleDivideReply is the stand-in's answer when the chat is dividing a
// huddle (card #1054): two card drafts and a settle list, in the blocks the
// Core's prompt asks for, over the items of "sample-call-2".
export function sampleDivideReply(callId: string): string {
  if (callId !== "sample-call-2")
    return "There is nothing on that canvas to divide."
  const card = (c: object) => `<<cards>>
${JSON.stringify(c)}
<</cards>>`
  return [
    "Here is how I would divide it. The certificate renewal and the Windows signing steps belong together, and the notarization check is its own small card. The signing question was answered in the call, so I would settle it instead of carding it.",
    "",
    card({
      title: "Renew the EV certificate and record the thumbprint",
      objective:
        "Renew the Windows signing certificate before the 14th and record the new thumbprint in docs/releasing.md.",
      items: ["t1"],
    }),
    card({
      title: "Notarization status check in the release script",
      objective:
        "Add a check that looks at Apple's status page when notarization passes ten minutes, before retrying.",
      items: ["t2", "n1"],
    }),
    '<<settle>>{"settle":["q1"],"drop":["q2"]}<</settle>>',
  ].join("\n")
}
