import { describe, expect, test } from "bun:test"

import { commandName, counted, onIpc } from "./ipc-count"

describe("IPC by command (card #1142)", () => {
  test("names a binding by its Rust command", () => {
    expect(commandName("boardCardsById")).toBe("board_cards_by_id")
    expect(commandName("coreReady")).toBe("core_ready")
  })

  test("counts each call and passes it through", async () => {
    const seen: string[] = []
    const off = onIpc((c) => seen.push(c))
    const api = counted({
      boardCards: (n: number) => Promise.resolve(n + 1),
      version: "1",
    })
    expect(await api.boardCards(1)).toBe(2)
    expect(api.version).toBe("1")
    off()
    await api.boardCards(1)
    expect(seen).toEqual(["board_cards"])
  })
})
