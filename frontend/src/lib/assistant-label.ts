// The one label for a person's Office Assistant (card #1045,
// docs/design/call-agents.md §5). Mirrors coord::assistant_name::label.
//
// `person` is the name of the person who signed the assistant's name, taken
// from their member.profile, never from the chosen name's text, so a chosen
// name can't pose as a person. With no chosen name the label is just
// "Addam's assistant".

// MaxAssistantName mirrors coord::assistant_name::MAX_CHARS.
export const MaxAssistantName = 24

// validAssistantName: "" (the default), or 1 to 24 letters and spaces, no
// space at either end.
export function validAssistantName(n: string): boolean {
  if (n === "") return true
  return (
    [...n].length <= MaxAssistantName &&
    n === n.trim() &&
    /^[\p{L} ]+$/u.test(n) &&
    /\p{L}/u.test(n)
  )
}

export function defaultAssistantName(person: string): string {
  const p = person.trim()
  return p ? `${p}'s assistant` : "Your assistant"
}

export function assistantLabel(person: string, chosen: string): string {
  const def = defaultAssistantName(person)
  const c = chosen.trim()
  return c && validAssistantName(c) ? `${c} · ${def}` : def
}
