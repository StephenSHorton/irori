import { describe, expect, test } from "bun:test"

import { clipboardFiles, filesWithheld } from "./file-attach"

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
