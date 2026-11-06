// A board's agent permissions in Settings › Agents (card #449,
// docs/design/spawned-agents.md §7 "Permission mode", decision 8): what a
// new agent may do without asking, extra rules, and how each repo's work
// reaches main. The app only writes them; no tool can. Outside the app the
// preview keeps a sample in memory.

import { useCallback, useEffect, useState } from "react"

import type { AgentPermissions } from "../gen/bindings"
import { AgentsService } from "./host"
import { inWails } from "./irori"

export type { AgentPermissions }

// The choices, as the Core names them; "" is stored for bypass too.
export const BYPASS = "bypass"
export const LIMITED = "limited"
export const EDITS = "edits"

export const permissionChoices = [
  { value: BYPASS, label: "Anything, without asking" },
  { value: LIMITED, label: "Its checks, git and Irori without asking" },
  { value: EDITS, label: "Edit files, ask before any command" },
]

// choiceOf is the shown choice: an empty setting is bypass.
export function choiceOf(p: AgentPermissions): string {
  return p.permission || BYPASS
}

// rulesOf splits the box's text into one rule per non-empty line.
export function rulesOf(text: string): string[] {
  const out: string[] = []
  for (const line of text.split("\n")) {
    const r = line.trim()
    if (r && !out.includes(r)) out.push(r)
  }
  return out
}

export const MERGE_PR = "pr"
export const MERGE_MAIN = "main"

export const sampleAgentPermissions: AgentPermissions = {
  boardId: "",
  permission: "",
  allow: ["Bash(cargo test:*)", "Bash(bun test:*)"],
  deny: [],
  merge: { "remote:github.com/sample-org/irori": MERGE_MAIN },
}

// usePermissions loads a board's agent permissions while open and saves
// a change, keeping the last good ones when a save fails.
export function usePermissions(open: boolean, boardId = "") {
  const [p, setP] = useState<AgentPermissions | null>(() =>
    inWails() ? null : { ...sampleAgentPermissions, boardId }
  )
  const [error, setError] = useState("")
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    AgentsService.Permissions(boardId).then(
      (v) => live && (setP(v), setError("")),
      (e: Error) => live && setError(e.message)
    )
    return () => {
      live = false
    }
  }, [open, boardId])
  const save = useCallback(
    // True when saved; a failure keeps the last good ones and says why.
    async (next: AgentPermissions): Promise<boolean> => {
      if (!inWails()) {
        setP(next)
        return true
      }
      try {
        setP(await AgentsService.SetPermissions(next))
        setError("")
        return true
      } catch (e) {
        setError((e as Error).message)
        return false
      }
    },
    []
  )
  // loading is true until the first read lands (card #598).
  return { permissions: p, error, loading: p === null && !error, save }
}
