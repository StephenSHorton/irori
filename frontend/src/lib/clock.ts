// The window's one clock (card #1137). It used to be App's own useNow() every
// second, passed down as `now` to every page: the whole visible tree rendered
// once a second. Now one shared 1 s tick wakes subscribers, and each subscriber
// reads a *derived value* (the text of "5m ago", the minute it is in); React
// re-renders it only when that value changed. So a row that says "3h ago"
// renders once an hour, a "42s ago" every second, and nothing above them ever.
//
// The tick runs only while something is subscribed.

import { useMemo, useSyncExternalStore } from "react"

const subs = new Set<() => void>()
let timer = 0

function subscribe(cb: () => void): () => void {
  subs.add(cb)
  if (!timer) timer = window.setInterval(() => subs.forEach((f) => f()), 1000)
  return () => {
    subs.delete(cb)
    if (!subs.size && timer) {
      window.clearInterval(timer)
      timer = 0
    }
  }
}

/**
 * useClockValue is `read(now)` kept current: the component re-renders only
 * when the value changes (compared with Object.is, so return a string,
 * number or boolean, never a fresh object).
 */
export function useClockValue<T extends string | number | boolean | null>(
  read: (now: Date) => T
): T {
  const get = () => read(new Date())
  return useSyncExternalStore(subscribe, get, get)
}

/**
 * useNow is the start of the current intervalMs-long stretch of time, so it
 * is at most intervalMs behind: a component that needs `now`
 * for logic (an expiry, a day change, a stale colour) renders once per
 * interval instead of every second. Pass the coarsest interval its text can
 * stand.
 */
export function useNow(intervalMs = 1000): Date {
  const get = () => Math.floor(Date.now() / intervalMs)
  const bucket = useSyncExternalStore(subscribe, get, get)
  return useMemo(() => new Date(bucket * intervalMs), [bucket, intervalMs])
}

/**
 * useToday is local midnight of today: for "Today" / "Yesterday" day labels,
 * which change once a day, so the component renders once a day.
 */
export function useToday(): Date {
  const get = () => {
    const n = new Date()
    return new Date(n.getFullYear(), n.getMonth(), n.getDate()).getTime()
  }
  const day = useSyncExternalStore(subscribe, get, get)
  return useMemo(() => new Date(day), [day])
}
