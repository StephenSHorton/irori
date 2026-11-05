import { describe, expect, test } from "bun:test"

import {
  startedRow,
  withStarted,
  type StartedThread,
  type ThreadSummary,
} from "./cards"
import { TaskResult } from "./models"

const row = (id: string) => ({ id }) as ThreadSummary
const started = (seq: number, id: string, boardId = ""): StartedThread => ({
  seq,
  boardId,
  row: row(id),
})
const ids = (rows: ThreadSummary[]) => rows.map((r) => r.id)

describe("a just-started thread in a slow window (card #551)", () => {
  test("shows on top of a list read before it started", () => {
    const list = withStarted([row("old")], 0, "", [started(1, "new")])
    expect(ids(list)).toEqual(["new", "old"])
  })

  test("two started: the newest first", () => {
    const list = withStarted([], 0, "", [started(1, "a"), started(2, "b")])
    expect(ids(list)).toEqual(["b", "a"])
  })

  test("isn't there twice once a read has it", () => {
    const list = withStarted([row("new")], 0, "", [started(1, "new")])
    expect(ids(list)).toEqual(["new"])
  })

  test("a read begun after it is the truth, with the row or without", () => {
    expect(ids(withStarted([], 1, "", [started(1, "gone")]))).toEqual([])
  })

  test("shows only in the list of the board it started on", () => {
    const s = [started(1, "on-board", "b-mlg")]
    expect(ids(withStarted([], 0, "", s))).toEqual([])
    expect(ids(withStarted([], 0, "b-mlg", s))).toEqual(["on-board"])
  })

  test("an unchanged list stays the same array", () => {
    const rows = [row("old")]
    expect(withStarted(rows, 1, "", [started(1, "new")])).toBe(rows)
  })

  test("the row comes from StartThread's answer", () => {
    const task = {
      id: "t1",
      short: "549",
      title: "Why?",
      objective: "Why is it slow?",
      createdAt: "2026-11-05T21:00:00Z",
      updatedAt: "2026-11-05T21:00:01Z",
      ask: { wake: true, taker: "d/s" },
    } as TaskResult["task"]
    const r = startedRow(TaskResult({ task }))
    expect(r).toMatchObject({
      id: "t1",
      short: "549",
      title: "Why?",
      text: "Why is it slow?",
      state: "handed",
      agentsNow: true,
      at: "2026-11-05T21:00:00Z",
      replies: 0,
    })
    const quiet = { ...task, ask: undefined }
    expect(startedRow(TaskResult({ task: quiet }))).toMatchObject({
      state: "waiting",
      agentsNow: false,
    })
  })
})
