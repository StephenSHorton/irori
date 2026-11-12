// Who a Stop reaches (decision 71): "On cards", the agents Irori started
// (the roster's spawned mark), and "Yours", the terminal sessions the
// person started, worded like the sidebar. Both ticked is the Stop from
// before the choice and the default. Only On cards holds starts; Yours
// alone is a one-off hand-off run that lights no switch and ends with Call
// off. The choice is kept per computer (this app's localStorage), never
// synced. Words: stop-labels.ts.

import { useCallback, useEffect, useState } from "react"

import { stopLabels as L } from "./stop-labels"

export type StopWho = { cards: boolean; yours: boolean }

export const BOTH: StopWho = { cards: true, yours: true }

export const stopWhoKey = "irori.stop.who"
// The other copies of the hook (the header's Stop here and the dialog)
// hear a change made in one of them.
const changedEvent = "irori:stopWho"

// parseStopWho reads a kept choice: anything unreadable, or nothing
// ticked, is both.
export function parseStopWho(raw: string | null | undefined): StopWho {
  if (!raw) return BOTH
  try {
    const v = JSON.parse(raw) as Partial<StopWho> | null
    const who = { cards: v?.cards === true, yours: v?.yours === true }
    return who.cards || who.yours ? who : BOTH
  } catch {
    return BOTH
  }
}

// readStopWho is this computer's kept choice; no storage reads as both.
export function readStopWho(): StopWho {
  try {
    return parseStopWho(localStorage.getItem(stopWhoKey))
  } catch {
    return BOTH
  }
}

export function writeStopWho(who: StopWho) {
  try {
    localStorage.setItem(stopWhoKey, JSON.stringify(who))
  } catch {
    // No storage: the choice holds until the window reloads.
  }
  try {
    window.dispatchEvent(new CustomEvent(changedEvent, { detail: who }))
  } catch {
    // No window (tests).
  }
}

// useStopWho is the kept choice and its setter; every copy follows a
// change made in another.
export function useStopWho(): [StopWho, (who: StopWho) => void] {
  const [who, setWho] = useState<StopWho>(readStopWho)
  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<StopWho>).detail
      setWho(d ?? readStopWho())
    }
    window.addEventListener(changedEvent, on)
    return () => window.removeEventListener(changedEvent, on)
  }, [])
  const set = useCallback((w: StopWho) => {
    setWho(w)
    writeStopWho(w)
  }, [])
  return [who, set]
}

type Agent = { spawned: boolean; here?: boolean }

// stopCounts is how many agents each tick reaches: from the roster's
// spawned mark, the one the app stops by (not the sidebar's card rows: a
// kept or resting agent Irori started still counts as on cards). With
// `here`, this machine's only (Stop here).
export function stopCounts(
  agents: Agent[],
  here = false
): { cards: number; yours: number } {
  let cards = 0
  let yours = 0
  for (const a of agents) {
    if (here && !a.here) continue
    if (a.spawned) cards++
    else yours++
  }
  return { cards, yours }
}

// stopReach is how many agents a Stop with `who` reaches.
export function stopReach(
  who: StopWho,
  n: { cards: number; yours: number }
): number {
  return (who.cards ? n.cards : 0) + (who.yours ? n.yours : 0)
}

// stopButton is the dialog's Stop button: "Stop 3 agents", or for Yours
// alone "Ask 2 of yours to hand off". Empty when nothing is ticked (the
// button is then disabled). `named` says "everywhere" too, for a dialog
// titled Stop here, where the button still reaches every machine.
export function stopButton(
  who: StopWho,
  n: { cards: number; yours: number },
  named = false
): string {
  if (!who.cards && !who.yours) return ""
  if (!who.cards)
    return named ? L.who.stopYoursNamed(n.yours) : L.who.stopYours(n.yours)
  const all = stopReach(who, n)
  return named ? L.everywhere.stopNamed(all) : L.everywhere.stop(all)
}

// stopExplain is what Stop everywhere does with `who` ticked; nothing
// ticked reads as both, like the kept choice.
export function stopExplain(who: StopWho): string {
  if (who.cards && !who.yours) return L.everywhere.explainCards
  if (who.yours && !who.cards) return L.everywhere.explainYours
  return L.everywhere.explain
}

// stopHereBlocked is why the header's Stop here can't be clicked with
// `who` and this machine's counts, or "" when it can. On cards always
// can: with nobody to ask it still holds starts here.
export function stopHereBlocked(
  who: StopWho,
  here: { cards: number; yours: number }
): string {
  if (!who.cards && !who.yours) return L.who.none
  if (!who.cards && here.yours === 0) return L.here.noneYours
  return ""
}

// stopHereTitle is the header's Stop here hover: what one click does with
// the kept choice.
export function stopHereTitle(who: StopWho): string {
  if (!who.cards && !who.yours) return L.who.none
  if (!who.cards) return L.here.titleYours
  if (!who.yours) return L.here.titleCards
  return L.here.title
}
