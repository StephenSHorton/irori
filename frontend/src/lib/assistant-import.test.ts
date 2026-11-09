import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test"

import { serializeHistory, type ChatEntry } from "./assistant"
import {
  applyHome,
  clearOldHistory,
  historyKey,
  importedKey,
  oldHistory,
  toImportEntries,
} from "./assistant-import"
import { AssistantService } from "./host"

function memoryStorage(start: Record<string, string>) {
  const m = new Map(Object.entries(start))
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
  } as unknown as Storage
}

const history: ChatEntry[] = [
  { role: "person", text: "close #12" },
  { role: "assistant", text: "Closed it.", looked: "Page: Board" },
  {
    role: "assistant",
    text: "#5 needs you",
    update: "thread-1",
    act: { kind: "answer", card: "task:1", blocker: "b1" },
  },
]

describe("moving today's history", () => {
  const was = globalThis.localStorage
  let imp: ReturnType<typeof spyOn>
  let store: Storage

  beforeEach(() => {
    store = memoryStorage({ [historyKey]: serializeHistory(history) })
    globalThis.localStorage = store
    imp = spyOn(AssistantService, "ChatImport").mockResolvedValue({
      turns: 1,
      updates: 1,
    })
  })
  afterEach(() => {
    imp.mockRestore()
    globalThis.localStorage = was
  })

  test("entries go to the host as its mapping reads them", () => {
    const out = toImportEntries(history)
    expect(out[0]).toMatchObject({
      role: "person",
      text: "close #12",
      update: "",
    })
    expect(out[1]).toMatchObject({ role: "assistant", looked: "Page: Board" })
    expect(out[2]).toMatchObject({
      update: "thread-1",
      act: { kind: "answer", card: "task:1", blocker: "b1" },
    })
  })

  test("the assistant's computer imports once, then sets the flag and stops", async () => {
    await applyHome({ here: true })
    expect(imp).toHaveBeenCalledTimes(1)
    expect(imp.mock.calls[0][0]).toHaveLength(3)
    expect(store.getItem(importedKey)).not.toBeNull()
    await applyHome({ here: true })
    expect(imp).toHaveBeenCalledTimes(1)
    expect(oldHistory()).toHaveLength(0)
  })

  test("a refused second import still sets the flag; another failure doesn't", async () => {
    imp.mockRejectedValueOnce(new Error("the history was already imported"))
    await applyHome({ here: true })
    expect(store.getItem(importedKey)).not.toBeNull()

    store.removeItem(importedKey)
    imp.mockRejectedValueOnce(new Error("the host is busy"))
    await applyHome({ here: true })
    expect(store.getItem(importedKey)).toBeNull()
    // The next change of the computer tries again.
    await applyHome({ here: true })
    expect(store.getItem(importedKey)).not.toBeNull()
  })

  test("another device imports nothing and shows its history read-only until New chat", async () => {
    await applyHome({ here: false })
    expect(imp).not.toHaveBeenCalled()
    expect(oldHistory().map((e) => e.text)).toEqual([
      "close #12",
      "Closed it.",
      "#5 needs you",
    ])
    clearOldHistory()
    expect(oldHistory()).toHaveLength(0)
    expect(store.getItem(historyKey)).toBeNull()
    await applyHome({ here: false })
    expect(oldHistory()).toHaveLength(0)
  })

  test("an empty history imports nothing and still sets the flag", async () => {
    store.removeItem(historyKey)
    await applyHome({ here: true })
    expect(imp).not.toHaveBeenCalled()
    expect(store.getItem(importedKey)).not.toBeNull()
  })
})
