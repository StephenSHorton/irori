import { useCallback, useState } from "react"

import type { Mention } from "./mentions"

// Drafts keeps what the person has typed on a card's page (a comment, a
// blocker answer, a resolve note) outside the page, so leaving it (Esc,
// Back, the browser's or the mouse's back) and coming back finds the text
// where it was. Memory only: a reload starts empty. Keys are
// "<card id>/<field>".
const drafts = new Map<string, string>()

export function draft(key: string): string {
  return drafts.get(key) ?? ""
}

export function setDraft(key: string, text: string) {
  if (text) drafts.set(key, text)
  else drafts.delete(key)
}

// useDraft is useState for one draft: it starts from the kept text and
// keeps every change.
export function useDraft(key: string): [string, (text: string) => void] {
  const [text, setText] = useState(() => draft(key))
  const set = useCallback(
    (t: string) => {
      setDraft(key, t)
      setText(t)
    },
    [key]
  )
  return [text, set]
}

// The mentions picked in a draft (card #77): the text keeps "@name", the
// key lives here, so coming back to the box keeps who hears.
const mentionDrafts = new Map<string, Mention[]>()

// useMentionsDraft is useDraft for a box's mentions; key is the text's.
export function useMentionsDraft(
  key: string
): [Mention[], (mentions: Mention[]) => void] {
  const [mentions, setMentions] = useState(() => mentionDrafts.get(key) ?? [])
  const set = useCallback(
    (m: Mention[]) => {
      if (m.length) mentionDrafts.set(key, m)
      else mentionDrafts.delete(key)
      setMentions(m)
    },
    [key]
  )
  return [mentions, set]
}

// CardForm is New card's form as the person left it (card #1071).
export type CardForm = {
  title: string
  objective: string
  doneWhen: string
  repoKey: string
  assignTo: string
  // The board picked inside the space, or null for the default.
  picked: string | null
  files: File[]
}

// A Talk message's Make a card, kept while Settings is open (card #1071):
// Settings is a page and takes the conversation with it, so its link
// keeps the post the card is made from and the form here, and the
// conversation opens the form again when it's back. One at a time: the
// form is a dialog. Memory only, as drafts above.
export type MakingCard<P> = { convId: string; post: P; form: CardForm }

let making: MakingCard<unknown> | null = null

export function keepMakingCard<P>(m: MakingCard<P>) {
  making = m
}

// makingCard is the Make a card kept for conversation convId, if any. It
// stays kept until dropped, so reading it twice (React's strict mode runs
// state initialisers twice) finds it both times.
export function makingCard<P>(convId: string): MakingCard<P> | undefined {
  return making?.convId === convId ? (making as MakingCard<P>) : undefined
}

// dropMakingCard forgets conversation convId's kept Make a card: the form
// was closed or the card made.
export function dropMakingCard(convId: string) {
  if (making?.convId === convId) making = null
}
