import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test"

import { AssistantService } from "./host"
import { splitReply } from "./assistant"
import { forgetSent, prepareAsk, sendAsk, splitAsk } from "./assistant-ask"

const prepared = {
  space: "s1",
  spaceName: "MLG farming",
  person: "p1",
  name: "Bob",
  text: "Is the 🚀 build green?",
  held: "",
}

describe("splitAsk", () => {
  test("takes the block out and keeps the words around it", () => {
    const s = splitAsk(
      "Asking Bob.\n<<ask Bob | MLG farming>>\nIs it green?\n<</ask>>"
    )
    expect(s.text).toBe("Asking Bob.")
    expect(s.ask).toEqual({
      person: "Bob",
      space: "MLG farming",
      text: "Is it green?",
      partial: false,
    })
  })
  test("near misses of the closing line close it", () => {
    for (const close of ["</ask>>", "<</ask>", "<</ ask >>"])
      expect(splitAsk(`<<ask Bob | S>>\nhi\n${close}`).ask?.partial).toBe(false)
  })
  test("an unclosed block is partial, so it can't be sent", () => {
    expect(splitAsk("<<ask Bob | S>>\nhi").ask?.partial).toBe(true)
  })
  test("a marker being written never flashes", () => {
    expect(splitAsk("Asking.\n<<as").text).toBe("Asking.")
    expect(splitAsk("Asking.\n<<ask Bo").text).toBe("Asking.")
    expect(splitAsk("a < b").text).toBe("a < b")
  })
  test("a thread or go line inside it is the question's words", () => {
    const r = splitReply(
      "ok\n<<ask Bob | S>>\n<<thread>>\nx\n<</thread>>\n<<go page home>>\n<</ask>>"
    )
    expect(r.draft).toBe("")
    expect(r.go).toBeNull()
    expect(r.ask?.text).toContain("<<thread>>")
  })
  test("a reply with no block has no ask", () => {
    expect(splitReply("Just an answer.").ask).toBeNull()
  })
})

describe("Send", () => {
  let prep: ReturnType<typeof spyOn>
  let send: ReturnType<typeof spyOn>
  beforeEach(() => {
    forgetSent()
    prep = spyOn(AssistantService, "AskPrepare").mockResolvedValue(prepared)
    send = spyOn(AssistantService, "AskSend").mockResolvedValue({
      postId: "post_1",
      conv: "c",
      space: "s1",
      text: prepared.text,
    })
  })
  afterEach(() => {
    prep.mockRestore()
    send.mockRestore()
  })
  test("preparing posts nothing", async () => {
    await prepareAsk({ person: "Bob", space: "S", text: "raw", partial: false })
    expect(prep).toHaveBeenCalledWith("Bob", "S", "raw")
    expect(send).not.toHaveBeenCalled()
  })
  test("a tap posts exactly the text shown, once", async () => {
    const a = sendAsk(prepared)
    const b = sendAsk(prepared)
    expect(await a).toBe("post_1")
    expect(await b).toBe("post_1")
    expect(send).toHaveBeenCalledTimes(1)
    expect(send).toHaveBeenCalledWith("s1", "p1", prepared.text)
    expect(await sendAsk(prepared)).toBe("")
    expect(send).toHaveBeenCalledTimes(1)
  })
})
