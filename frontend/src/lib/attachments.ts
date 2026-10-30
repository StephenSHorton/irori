// Attachments gateway (card #14): the files agents attach to Board comments.
// The window loads a file from AttachmentRoute + sha256, a route the app's
// own webview serves (never the loopback API); Save and Open go through
// AttachmentsService. Like cards.ts, components import from here, never
// from bindings/. In a plain browser there are no files: images draw a
// sample, and Save and Open say they need the app.

import { AttachmentsService } from "./host"
import { AttachmentInfo, FileRef } from "./models"
import { inWails } from "./irori"
import { errorText } from "./talk"

export { AttachmentInfo, FileRef }

// AttachmentRoute mirrors core.AttachmentRoute.
export const AttachmentRoute = "/irori/attachment/"

// InlineImages mirrors core.InlineImages: the types the route shows inline
// (it decides by the bytes; mime here is the attaching device's guess).
const InlineImages = ["image/png", "image/jpeg", "image/gif", "image/webp"]
const ImageExt = /\.(png|jpe?g|gif|webp)$/i

// MaxPreview mirrors core.MaxPreview: the route refuses anything bigger
// (without fetching it), so such a file is a chip.
export const MaxPreview = 32 << 20

// MaxAutoPreview: a picture up to this size loads by itself when it
// scrolls near; a bigger one waits for a click (it crosses devices whole).
export const MaxAutoPreview = 8 << 20

// looksLikeImage reports whether a file should get a thumbnail: by the
// attaching device's type, else its name, and no bigger than MaxPreview.
// The route has the last word: a file it serves as a download falls back
// to a chip.
export function looksLikeImage(f: FileRef): boolean {
  if (f.size > MaxPreview) return false
  const m = (f.mime ?? "").split(";")[0].trim().toLowerCase()
  return InlineImages.includes(m) || (!m && ImageExt.test(f.name))
}

// OpenableExts mirrors attachments_service.go's openableExts: the types
// Open hands to the OS. Everything else is Save only.
const OpenableExts = new Set([
  ".png",
  ".jpg",
  ".jpeg",
  ".gif",
  ".webp",
  ".bmp",
  ".heic",
  ".pdf",
  ".txt",
  ".md",
  ".log",
  ".csv",
  ".json",
  ".yaml",
  ".yml",
  ".docx",
  ".xlsx",
  ".pptx",
  ".mp4",
  ".mov",
  ".mp3",
  ".wav",
])

// canOpen reports whether Open would take a file, by its name (Open checks
// the bytes too, and AttachmentInfo.openable is the binding's answer).
export function canOpen(f: FileRef): boolean {
  const name = f.name.replace(/[. ]+$/, "")
  const dot = name.lastIndexOf(".")
  return dot > 0 && OpenableExts.has(name.slice(dot).toLowerCase())
}

// schemeOrigin is where the app's `irori` URI scheme answers (surface.md
// 7.7): Tauri serves custom schemes at http://<scheme>.localhost on
// Windows, where the page itself is http://tauri.localhost, and at
// <scheme>://localhost elsewhere. AttachmentInfo.url is the same URL.
function schemeOrigin(): string {
  const { protocol, hostname } = window.location
  if ((protocol === "http:" || protocol === "https:") && hostname.endsWith(".localhost")) {
    return `${protocol}//irori.localhost`
  }
  return "irori://localhost"
}

export function attachmentURL(sha: string): string {
  return schemeOrigin() + AttachmentRoute + sha
}

// humanSize mirrors blob.humanSize.
export function humanSize(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1 << 20) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1 << 30) return `${(n / (1 << 20)).toFixed(1)} MB`
  return `${(n / (1 << 30)).toFixed(1)} GB`
}

// Loaded is a fetched file: an object URL to show when it is an inline
// image, else served as a download.
export type Loaded =
  | { kind: "image"; url: string }
  | { kind: "file" }
  | { kind: "error"; message: string; offline: boolean }

// sampleImage stands in for a picture in a plain browser.
const sampleImage =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#94a3b8"/><text x="160" y="105" font-family="sans-serif" font-size="16" text-anchor="middle" fill="#fff">sample screenshot</text></svg>`
  )

// load fetches a file through the route. The body of a failed answer is
// the reason in plain text; 503 means the devices that hold it are offline.
// The caller revokes an image's url (URL.revokeObjectURL) when done.
export async function load(sha: string, signal?: AbortSignal): Promise<Loaded> {
  if (!inWails()) return { kind: "image", url: sampleImage }
  let res: Response
  try {
    res = await fetch(attachmentURL(sha), { signal })
  } catch (err) {
    if (signal?.aborted) throw err
    return { kind: "error", message: errorText(err), offline: false }
  }
  if (!res.ok) {
    const text = (await res.text().catch(() => "")).trim()
    return {
      kind: "error",
      message: text || `couldn't load the file (${res.status})`,
      offline: res.status === 503,
    }
  }
  const type = (res.headers.get("Content-Type") ?? "").split(";")[0].trim()
  if (!InlineImages.includes(type)) {
    res.body?.cancel().catch(() => {})
    return { kind: "file" }
  }
  return { kind: "image", url: URL.createObjectURL(await res.blob()) }
}

const noApp = "Saving and opening files needs the Irori app."

// save writes the file into Downloads (never over another file) and
// returns its path.
export async function save(sha: string): Promise<string> {
  if (!inWails()) throw new Error(noApp)
  return AttachmentsService.Save(sha)
}

// open opens the file with the OS default app. Programs and scripts are
// refused: save those instead.
export async function open(sha: string): Promise<void> {
  if (!inWails()) throw new Error(noApp)
  await AttachmentsService.Open(sha)
}

// info describes a file without fetching it (here: on this device).
export async function info(sha: string): Promise<AttachmentInfo> {
  return AttachmentsService.Info(sha)
}

export { errorText }
