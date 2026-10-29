import { describe, expect, test } from "bun:test"

import { richBlocks, richInline, safeHref, type Inline } from "./rich"

// kinds is a line as [kind, text] pairs, links as their href.
function kinds(s: string): [string, string][] {
  return richInline(s).map((p) => [p.kind, p.kind === "link" ? p.href : p.text])
}

function linksOf(s: string): string[] {
  return richInline(s)
    .filter((p): p is Extract<Inline, { kind: "link" }> => p.kind === "link")
    .map((p) => p.href)
}

describe("links", () => {
  test("http(s) URLs are found, with the sentence's punctuation left out", () => {
    expect(kinds("open https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m and click Request access.")).toEqual([
      ["text", "open "],
      ["link", "https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m"],
      ["text", " and click Request access."],
    ])
    expect(linksOf("see http://localhost.dev:8080/x?a=1&b=2, then")).toEqual(["http://localhost.dev:8080/x?a=1&b=2"])
    expect(linksOf("(https://example.com/a_(b)) and https://example.com/c).")).toEqual([
      "https://example.com/a_(b)",
      "https://example.com/c",
    ])
    expect(linksOf("HTTPS://GitHub.com/acme/app/pull/57!")).toEqual(["https://github.com/acme/app/pull/57"])
  })

  test("bare links need a host with a dot and a known TLD", () => {
    expect(linksOf("open huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m and click")).toEqual([
      "https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m",
    ])
    expect(linksOf("on github.com, or www.notion.com/x.")).toEqual(["https://github.com/", "https://www.notion.com/x"])
    // File names, versions and abbreviations stay text.
    for (const s of [
      "edit board.go and cards.ts",
      "see README.md or main.rs, run.sh",
      "v0.6.1 and 1.2.3",
      "e.g. this, i.e. that",
      "package.json and libfoo.so",
      "localhost:8080",
    ])
      expect(linksOf(s)).toEqual([])
  })

  test("the host stands out in what the agent wrote", () => {
    const [l] = richInline("https://evil.com/github.com/acme").filter((p) => p.kind === "link")
    expect(l).toMatchObject({ pre: "https://", host: "evil.com", rest: "/github.com/acme", shownHost: "" })
    const [b] = richInline("huggingface.co/x").filter((p) => p.kind === "link")
    expect(b).toMatchObject({ pre: "", host: "huggingface.co", rest: "/x" })
    // An IDN host can't be found in the text: its punycode is named.
    const [i] = richInline("https://exämple.com/x").filter((p) => p.kind === "link")
    expect(i).toMatchObject({ host: "", shownHost: "xn--exmple-cua.com" })
  })

  test("unsafe schemes and hidden hosts are refused", () => {
    for (const s of [
      "javascript:alert(1)",
      "javascript://example.com/%0aalert(1)",
      "file:///etc/passwd",
      "file://server.com/share",
      "data:text/html,<script>alert(1)</script>",
      "vbscript:msgbox(1)",
      "xhttps://example.com",
      "mail me@example.com",
      "https://user:pw@example.com/",
      "https://",
    ])
      expect(linksOf(s)).toEqual([])
    expect(safeHref("javascript:alert(1)")).toBeNull()
    expect(safeHref("https://user@example.com")).toBeNull()
    expect(safeHref("example.com/a b")).toBe("https://example.com/a%20b")
    expect(safeHref("http://example.com")).toBe("http://example.com/")
  })

  test("control and bidi characters are made plain first", () => {
    expect(linksOf("https://example.com/‮gnp.exe")).toEqual(["https://example.com/"])
  })
})

describe("code", () => {
  test("backtick spans are code, without links inside", () => {
    expect(kinds("type `! hf auth login` in the Mac session")).toEqual([
      ["text", "type "],
      ["code", "! hf auth login"],
      ["text", " in the Mac session"],
    ])
    expect(kinds("run `curl https://example.com/x` then open example.com")).toEqual([
      ["text", "run "],
      ["code", "curl https://example.com/x"],
      ["text", " then open "],
      ["link", "https://example.com/"],
    ])
  })

  test("an unclosed or empty backtick stays text", () => {
    expect(kinds("it's `half")).toEqual([["text", "it's `half"]])
    expect(kinds("a `` b")).toEqual([["text", "a `` b"]])
  })
})

describe("blocks", () => {
  test("line breaks and paragraphs keep their shape", () => {
    const b = richBlocks("one\ntwo\n\nthree")
    expect(b.map((x) => x.kind)).toEqual(["para", "para"])
    expect(b[0].kind === "para" && b[0].lines.length).toBe(2)
  })

  test("list lines keep the marker the agent wrote", () => {
    const b = richBlocks("Two steps:\n1) open huggingface.co/x\n2) type `hf auth login`\n- and a dash\nafter")
    expect(b.map((x) => x.kind)).toEqual(["para", "list", "para"])
    const list = b[1]
    if (list.kind !== "list") throw new Error("no list")
    expect(list.items.map((i) => i.marker)).toEqual(["1)", "2)", "-"])
    expect(list.items[0].inline.map((p) => p.kind)).toEqual(["text", "link"])
    expect(list.items[1].inline.map((p) => p.kind)).toEqual(["text", "code"])
  })

  test("a line counting 1) 2) inline is shown as that list", () => {
    const b = richBlocks("Two things: 1) open huggingface.co/x and click Request access 2) type `! hf auth login` in the Mac session")
    expect(b.map((x) => x.kind)).toEqual(["para", "list"])
    const list = b[1]
    if (list.kind !== "list") throw new Error("no list")
    expect(list.items.map((i) => i.marker)).toEqual(["1)", "2)"])
    expect(list.items[1].inline.map((p) => p.kind)).toEqual(["text", "code", "text"])
    // One "1)" alone, numbers out of order, or a "(1)" are not a list.
    for (const s of ["only 1) here", "see 2) and 3) there", "a (1) b (2) c"])
      expect(richBlocks(s).map((x) => x.kind)).toEqual(["para"])
  })

  test("empty text has no blocks", () => {
    expect(richBlocks("  \n\n ")).toEqual([])
  })
})
