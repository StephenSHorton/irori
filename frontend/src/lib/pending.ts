// Pending changes (card #644): what the person just did, shown at once
// while the host saves it. A click adds a patch for a card's detail, its
// summary (the grid and Needs you) or a notification; every read of that
// data passes through it. A patch is undone when the save fails, and goes
// once real data agrees with it, or SettleCapMs after the save, whichever
// comes first. Patches set what the answer will make true (a blocker
// answered, a card closed), so applying one to data that already says so
// changes nothing: a stale read never flashes the old state back.

import type { CardDetail, CardSummary, Notification } from "./models"

// One change's patches. Each part is optional: an ask's answer touches
// only its notification, a close only the card.
export type Patch = {
  key: string
  cardId?: string
  detail?: (d: CardDetail) => CardDetail
  detailAgrees?: (d: CardDetail) => boolean
  summary?: (c: CardSummary) => CardSummary
  summaryAgrees?: (c: CardSummary) => boolean
  noteKey?: string
  notification?: (n: Notification) => Notification
  notificationAgrees?: (n: Notification) => boolean
}

// How long a saved change still shows when no read has agreed with it:
// board:changed comes within 250 ms of a save and its read soon after, so
// this is only for a read that never comes (nothing watches that card).
export const SettleCapMs = 15_000

type Entry = {
  patch: Patch
  token: number
  detail: boolean
  summary: boolean
  notification: boolean
  timer?: ReturnType<typeof setTimeout>
}

export type Pending = ReturnType<typeof createPending>

export function createPending(
  capMs = SettleCapMs,
  timers: {
    set: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>
    clear: (t: ReturnType<typeof setTimeout>) => void
  } = {
    set: (fn, ms) => setTimeout(fn, ms),
    clear: (t) => clearTimeout(t),
  }
) {
  const entries = new Map<string, Entry>()
  const listeners = new Set<() => void>()
  let version = 0
  let tokens = 0

  function changed() {
    version++
    for (const fn of [...listeners]) fn()
  }

  function drop(key: string, token?: number) {
    const e = entries.get(key)
    if (!e || (token !== undefined && e.token !== token)) return false
    if (e.timer !== undefined) timers.clear(e.timer)
    entries.delete(key)
    return true
  }

  // An entry with no part left showing goes.
  function prune(key: string, e: Entry) {
    if (!e.detail && !e.summary && !e.notification) drop(key, e.token)
  }

  const live = () => [...entries.values()]

  return {
    // add shows patch now, replacing one with the same key; it returns
    // the change's token for saved and undo.
    add(patch: Patch): number {
      drop(patch.key)
      const token = ++tokens
      entries.set(patch.key, {
        patch,
        token,
        detail: !!(patch.cardId && patch.detail),
        summary: !!(patch.cardId && patch.summary),
        notification: !!(patch.noteKey && patch.notification),
      })
      changed()
      return token
    },
    // saved: the host took it. It shows until a read agrees, at most
    // capMs more.
    saved(key: string, token: number) {
      const e = entries.get(key)
      if (!e || e.token !== token) return
      e.timer = timers.set(() => {
        if (drop(key, token)) changed()
      }, capMs)
    },
    // undo takes the change back (its save failed).
    undo(key: string, token: number) {
      if (drop(key, token)) changed()
    },
    has: (key: string) => entries.has(key),
    version: () => version,
    subscribe(fn: () => void): () => void {
      listeners.add(fn)
      return () => {
        listeners.delete(fn)
      }
    },

    // touches: whether any change shows on this card's detail.
    touches: (cardId: string) =>
      live().some((e) => e.detail && e.patch.cardId === cardId),
    detail(d: CardDetail): CardDetail {
      let out = d
      for (const e of live())
        if (e.detail && e.patch.cardId === d.id) out = e.patch.detail!(out)
      return out
    },
    summaries(list: CardSummary[]): CardSummary[] {
      const on = live().filter((e) => e.summary)
      if (!on.length) return list
      return list.map((c) => {
        let out = c
        for (const e of on)
          if (e.patch.cardId === c.id) out = e.patch.summary!(out)
        return out
      })
    },
    notifications(list: Notification[]): Notification[] {
      const on = live().filter((e) => e.notification)
      if (!on.length) return list
      return list.map((n) => {
        let out = n
        for (const e of on)
          if (e.patch.noteKey === n.key) out = e.patch.notification!(out)
        return out
      })
    },

    // observe*: real data came in; a part it agrees with stops showing.
    // A card or notification missing from a full list agrees too (a
    // closed card leaves the live list).
    observeDetail(d: CardDetail) {
      let any = false
      for (const [key, e] of [...entries]) {
        if (!e.detail || e.patch.cardId !== d.id) continue
        if (!e.patch.detailAgrees || !e.patch.detailAgrees(d)) continue
        e.detail = false
        any = true
        prune(key, e)
      }
      if (any) changed()
    },
    observeSummaries(list: CardSummary[]) {
      const byId = new Map(list.map((c) => [c.id, c]))
      let any = false
      for (const [key, e] of [...entries]) {
        if (!e.summary) continue
        const c = byId.get(e.patch.cardId!)
        if (c && !(e.patch.summaryAgrees && e.patch.summaryAgrees(c))) continue
        e.summary = false
        any = true
        prune(key, e)
      }
      if (any) changed()
    },
    observeNotifications(list: Notification[]) {
      const byKey = new Map(list.map((n) => [n.key, n]))
      let any = false
      for (const [key, e] of [...entries]) {
        if (!e.notification) continue
        const n = byKey.get(e.patch.noteKey!)
        if (
          n &&
          !(e.patch.notificationAgrees && e.patch.notificationAgrees(n))
        )
          continue
        e.notification = false
        any = true
        prune(key, e)
      }
      if (any) changed()
    },
  }
}

// pending is the window's one set of pending changes.
export const pending = createPending()
