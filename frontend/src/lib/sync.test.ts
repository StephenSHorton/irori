import { describe, expect, test } from "bun:test"

import { setSyncing, subscribeSyncing, syncingNow } from "./sync"

// A browser: no window.__TAURI__, no query.
;(globalThis as { window?: unknown }).window ??= {
  location: { search: "" },
}

describe("sync state (card #1089)", () => {
  test("readers hear only a flip, and a browser starts not syncing", () => {
    let heard = 0
    const off = subscribeSyncing(() => heard++)
    expect(syncingNow()).toBe(false)
    setSyncing(true)
    setSyncing(true)
    expect(heard).toBe(1)
    expect(syncingNow()).toBe(true)
    setSyncing(false)
    expect(heard).toBe(2)
    off()
  })
})
