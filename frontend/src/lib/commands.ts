// The command registry (card #213): every action the window offers from the
// keyboard registers here once, and the command palette (Cmd/Ctrl+K or P)
// lists what is registered. A feature adds its commands with useCommands
// where it lives, so a new feature shows up in the palette by itself, its
// shortcut shown beside it.

import { useEffect, useRef, useSyncExternalStore } from "react"

export type Command = {
  // A stable id ("nav.home", "board.pin"): recents remember it.
  id: string
  // What the palette shows ("New card", "Go to Settings").
  title: string
  // Group heading in the palette (default "Commands").
  group?: string
  // Other words it is found by ("print" is not one: Cmd+P is the palette).
  keywords?: string[]
  // The keys that also run it, as shown: "/" or "Mod+K" (Mod is Cmd on a
  // Mac, Ctrl elsewhere). The palette only shows it; the feature listens.
  shortcut?: string
  // Where it sits in its group when the query is empty (lower first;
  // default 100).
  order?: number
  // quiet: listed only for a typed query, not in the empty palette (the
  // Settings sections: "Settings" itself stands for them there).
  quiet?: boolean
  // when says whether it applies right now; left out it always does.
  when?: () => boolean
  run: () => void
}

const registry = new Map<string, Command>()
const listeners = new Set<() => void>()
// version changes with every register and unregister, so useSyncExternalStore
// sees a new snapshot.
let snapshot: Command[] = []

function changed() {
  snapshot = [...registry.values()]
  for (const fn of listeners) fn()
}

// register adds commands (replacing any with the same id) and returns what
// takes them out again.
export function register(cmds: Command[]): () => void {
  for (const c of cmds) registry.set(c.id, c)
  changed()
  return () => {
    let any = false
    for (const c of cmds)
      if (registry.get(c.id) === c) {
        registry.delete(c.id)
        any = true
      }
    if (any) changed()
  }
}

// allCommands is every registered command, applicable or not.
export function allCommands(): Command[] {
  return snapshot
}

// activeCommands is the commands that apply now (when() true or absent).
export function activeCommands(cmds: Command[] = snapshot): Command[] {
  return cmds.filter((c) => {
    try {
      return c.when ? c.when() : true
    } catch {
      return false
    }
  })
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  return () => {
    listeners.delete(fn)
  }
}

// useRegistry is the registered commands, for the palette.
export function useRegistry(): Command[] {
  return useSyncExternalStore(subscribe, allCommands, allCommands)
}

// useCommands registers cmds while the component is mounted. The list is
// re-read on every render (through a ref), so run() and when() always see
// the component's latest state; it re-registers only when the ids or
// titles change.
export function useCommands(cmds: Command[]): void {
  const latest = useRef(cmds)
  useEffect(() => {
    latest.current = cmds
  })
  const key = cmds
    .map((c) => `${c.id}\u0000${c.title}\u0000${c.shortcut ?? ""}`)
    .join("\u0001")
  useEffect(() => {
    const live: Command[] = latest.current.map((c, i) => ({
      ...c,
      when: () => {
        const now = latest.current[i]
        return now?.when ? now.when() : true
      },
      run: () => latest.current[i]?.run(),
    }))
    return register(live)
    // key is what the registration depends on.
  }, [key])
}

// isMac says whether Mod is Cmd here.
export function isMac(): boolean {
  const p =
    (navigator as Navigator & { userAgentData?: { platform?: string } })
      .userAgentData?.platform ?? navigator.platform
  return /mac|iphone|ipad/i.test(p)
}

// shortcutKeys splits a shortcut into the keys to show: "Mod+K" is ["⌘",
// "K"] on a Mac and ["Ctrl", "K"] elsewhere.
export function shortcutKeys(s: string, mac = isMac()): string[] {
  return s.split("+").map((k) => {
    if (k === "Mod") return mac ? "⌘" : "Ctrl"
    if (k === "Shift") return mac ? "⇧" : "Shift"
    if (k === "Alt") return mac ? "⌥" : "Alt"
    return k
  })
}
