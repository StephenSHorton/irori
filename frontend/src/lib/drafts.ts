import { useCallback, useState } from "react"

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
