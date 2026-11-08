import { describe, expect, test } from "bun:test"

import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import {
  clipboardFiles,
  filesWithheld,
  PendingFiles,
  withoutSent,
  type FileAttach,
} from "./file-attach"

// A paste or drop as the webview hands it over: a file list and items.
function transfer(files: File[], items: { kind: string; file?: File }[]) {
  return {
    files: files as unknown as FileList,
    items: items.map((i) => ({
      kind: i.kind,
      getAsFile: () => i.file ?? null,
    })) as unknown as DataTransferItemList,
  }
}

describe("clipboardFiles (card #195)", () => {
  const shot = new File(["png"], "image.png", { type: "image/png" })
  const doc = new File(["# hi"], "notes.md", { type: "text/markdown" })

  test("takes the file list when it has files", () => {
    expect(
      clipboardFiles(transfer([shot], [{ kind: "file", file: doc }]))
    ).toEqual([shot])
  })

  test("falls back to file items when the list is empty", () => {
    expect(
      clipboardFiles(
        transfer(
          [],
          [{ kind: "string" }, { kind: "file", file: doc }, { kind: "file" }]
        )
      )
    ).toEqual([doc])
  })

  test("pasted text is no files", () => {
    expect(clipboardFiles(transfer([], [{ kind: "string" }]))).toEqual([])
    expect(clipboardFiles(null)).toEqual([])
  })
})

describe("filesWithheld (card #321)", () => {
  const shot = new File(["png"], "image.png", { type: "image/png" })
  const paste = (types: string[], files: File[]) => ({
    types,
    ...transfer(
      files,
      files.map((file) => ({ kind: "file", file }))
    ),
  })

  test("says files but hands none over: WebKit with a HEIC-only clipboard", () => {
    expect(filesWithheld(paste(["Files"], []))).toBe(true)
  })

  test("files handed over, plain text, or nothing are not withheld", () => {
    expect(filesWithheld(paste(["Files"], [shot]))).toBe(false)
    expect(filesWithheld(paste(["text/plain"], []))).toBe(false)
    expect(filesWithheld(null)).toBe(false)
  })
})

describe("withoutSent (card #999)", () => {
  test("a file that arrived during the upload stays", () => {
    const a = new File(["a"], "a.png")
    const late = new File(["p"], "late.png")
    expect(withoutSent([a, late], [a])).toEqual([late])
  })
})

describe("PendingFiles (card #999)", () => {
  const doc = new File(["# hi"], "notes.md", { type: "text/markdown" })
  const render = (files: File[], refused: string | null) =>
    renderToStaticMarkup(
      createElement(PendingFiles, {
        attach: { files, refused, remove: () => {} } as unknown as FileAttach,
      })
    )

  test("says why a file was refused beside the files still listed", () => {
    const html = render([doc], "Add files once the reply is done.")
    expect(html).toContain("notes.md")
    expect(html).toContain("Add files once the reply is done.")
  })

  test("says why with no files listed, and nothing with neither", () => {
    expect(render([], "No.")).toContain("No.")
    expect(render([], null)).toBe("")
  })
})
