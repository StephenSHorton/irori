// What a person reads where an agent or person is named (card #478): a key
// (board:<board>/<card>, agent:<card>, person:<id>, <device>/<session>) is
// never a name. Something nothing names reads "unknown agent".

import { plainLine } from "@/lib/irori"

export const unknownAgent = "unknown agent"
export const unknownPerson = "another person"

// rawKey matches an actor key, or a bare hex id from one, anywhere in a
// string: what a person must never read as a name.
export const rawKey =
  /\b(?:board:[0-9A-Za-z]+\/\S+|agent:[0-9A-Za-z]{6,}|person:[0-9A-Za-z]{6,}|[0-9a-f]{16,}\/[0-9a-f-]{8,})/

// isKeyLike is whether s is (or holds) a key rather than a name.
export function isKeyLike(s: string): boolean {
  return rawKey.test(s)
}

// nameOr is `name` for a person to read, or the fallback when it is empty
// or a key.
export function nameOr(
  name: string | null | undefined,
  fallback = unknownAgent
): string {
  const n = plainLine(name ?? "")
  return n && !isKeyLike(n) ? n : fallback
}

// memberName is a Talk member's name: "you" stays as Talk says it, a person
// nothing names is "another person", an agent "unknown agent".
export function memberName(m: { kind: string; name?: string | null }): string {
  return nameOr(m.name, m.kind === "person" ? unknownPerson : unknownAgent)
}

// isOthersKey is whether key is another member's agent or person on a
// shared board: this device's roster never holds it, so missing from the
// roster doesn't make it gone.
export function isOthersKey(key: string): boolean {
  return (
    key.startsWith("board:") ||
    key.startsWith("agent:") ||
    key.startsWith("person:")
  )
}
