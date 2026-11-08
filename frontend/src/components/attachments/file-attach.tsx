import {
  useCallback,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
} from "react"
import { PaperclipIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { MaxUploadFiles } from "@/lib/cards"
import { clipboardImage } from "@/lib/clipboard"

import { FileKindIcon } from "./file-kind-icon"

// useFileAttach is a box's files waiting to go (cards #25, #73, #124,
// #195): pick them with the clip, drop them on the box, or paste a
// screenshot or a file copied in Finder. Every box the person talks to
// agents in shares it: the ask box, comments and thread Replies, blocker
// answers, a message's Reply and New card.
// off (while sending, or a box that takes no files) takes no new files;
// with why, a paste or drop it refuses says so under the box instead of
// vanishing. initial is files already waiting (a box opened again).
export function useFileAttach(
  off = false,
  why = "",
  initial: readonly File[] = []
) {
  const [files, setFiles] = useState<File[]>(() => [...initial])
  const [refused, setRefused] = useState<string | null>(null)
  const picker = useRef<HTMLInputElement>(null)
  const clear = useCallback(() => {
    setFiles([])
    setRefused(null)
  }, [])
  const add = (list: FileList | File[] | null | undefined) => {
    const more = pastedFiles(list)
    if (!more.length || off) return false
    setFiles((fs) => [...fs, ...more].slice(0, MaxUploadFiles))
    setRefused(null)
    return true
  }
  // took: whether a paste or drop of these files goes in; a refusal with
  // a reason is shown.
  const took = (more: File[]) => {
    if (!more.length) return false
    if (off) {
      if (why) setRefused(why)
      return !!why
    }
    return add(more)
  }
  return {
    files,
    full: files.length >= MaxUploadFiles,
    add,
    refused,
    clear,
    remove: (i: number) => setFiles((fs) => fs.filter((_, j) => j !== i)),
    pick: () => picker.current?.click(),
    // A pasted screenshot or Finder file is a file; pasted text stays
    // text. True when the paste was files (taken, or refused with why).
    onPaste: (e: ClipboardEvent) => {
      const more = clipboardFiles(e.clipboardData)
      if (!more.length) {
        // WebKit says a HEIC-only clipboard (a macOS screenshot) is
        // files but hands over none (card #321): the host reads the
        // picture instead.
        if (!filesWithheld(e.clipboardData)) return false
        e.preventDefault()
        if (off) {
          if (why) setRefused(why)
          return false
        }
        clipboardImage().then(
          (f) => add([f]),
          (err: unknown) =>
            setRefused(
              `Couldn't paste the picture: ${err instanceof Error ? err.message : String(err)}`
            )
        )
        return true
      }
      const done = took(more)
      if (done || !off) e.preventDefault()
      return done && !off
    },
    // Spread on the element files are dropped on.
    drop: {
      onDragOver: (e: DragEvent) => {
        if (e.dataTransfer.types.includes("Files")) e.preventDefault()
      },
      onDrop: (e: DragEvent) => {
        const more = clipboardFiles(e.dataTransfer)
        if (!more.length) return
        e.preventDefault()
        took(more)
      },
    },
    // The hidden file input the clip opens; render it once in the box.
    input: (
      <input
        ref={picker}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          add(e.target.files)
          e.target.value = ""
        }}
      />
    ),
  }
}

export type FileAttach = ReturnType<typeof useFileAttach>

// clipboardFiles is the files a paste or drop carries: its file list, or
// its file items when the list is empty (some webviews fill only one).
export function clipboardFiles(
  d: Pick<DataTransfer, "files" | "items"> | null | undefined
): File[] {
  if (!d) return []
  const listed = pastedFiles(d.files)
  if (listed.length) return listed
  return Array.from(d.items ?? [])
    .filter((i) => i.kind === "file")
    .map((i) => i.getAsFile())
    .filter((f): f is File => !!f)
}

// filesWithheld is a paste that says it carries files but hands none
// over: WebKit does this for a clipboard holding only a HEIC picture.
export function filesWithheld(
  d: Pick<DataTransfer, "files" | "items" | "types"> | null | undefined
): boolean {
  if (!d || !Array.from(d.types ?? []).includes("Files")) return false
  return clipboardFiles(d).length === 0
}

function pastedFiles(list: FileList | File[] | null | undefined): File[] {
  return Array.from(list ?? [])
}

// composerIcon sits a 28 px icon button on a composer's 36 px line: the
// rows top-align (a box grows downward as it wraps), so without the margin
// the icon rides 4 px above the box's first line and the send button (#358).
export const composerIcon = "my-1 shrink-0"

// FilesOff is why a box took no files: its clip, shown disabled with the
// reason as its title.
export function FilesOff({ why }: { why: string }) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="ghost"
      className={composerIcon}
      title={why}
      aria-label={`No files here: ${why}`}
      disabled
    >
      <PaperclipIcon />
    </Button>
  )
}

// AttachButton is the clip: it opens the file picker.
export function AttachButton({
  attach,
  disabled,
}: {
  attach: FileAttach
  disabled?: boolean
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="ghost"
      className={composerIcon}
      title="Add files (or drop them here, or paste a screenshot or file)"
      aria-label="Add files"
      disabled={disabled || attach.full}
      onClick={attach.pick}
    >
      <PaperclipIcon />
    </Button>
  )
}

// PendingFiles lists the files waiting to go, each with an × to take it
// back off.
export function PendingFiles({
  attach,
  disabled,
  label = "Files to send",
  className = "flex flex-wrap gap-2 px-1",
}: {
  attach: FileAttach
  disabled?: boolean
  label?: string
  className?: string
}) {
  if (!attach.files.length)
    return attach.refused ? (
      <p role="status" className="px-1 text-xs text-muted-foreground">
        {attach.refused}
      </p>
    ) : null
  return (
    <ul className={className} aria-label={label}>
      {attach.files.map((f, i) => (
        <PendingFile
          key={fileKey(f)}
          file={f}
          disabled={!!disabled}
          onRemove={() => attach.remove(i)}
        />
      ))}
    </ul>
  )
}

// fileKey keeps a pending file's row (and its preview) when another one
// before it is removed.
const keys = new WeakMap<File, string>()
let nextKey = 0
function fileKey(f: File): string {
  let k = keys.get(f)
  if (!k) keys.set(f, (k = `f${nextKey++}`))
  return k
}

// Pictures the webview can draw: these show as a thumbnail before sending.
const Previewable = /^image\/(png|jpe?g|gif|webp|bmp|avif|svg\+xml)$/i
const MaxThumb = 32 << 20

// PendingFile is one file waiting to go (card #73): a pasted or picked
// picture shows itself, anything else an icon for its kind and its name.
function PendingFile({
  file,
  disabled,
  onRemove,
}: {
  file: File
  disabled: boolean
  onRemove: () => void
}) {
  const name = file.name || "pasted image"
  const image = Previewable.test(file.type) && file.size <= MaxThumb
  // One object URL per picture, made as its <img> mounts and let go as it
  // unmounts (a ref with cleanup, React 19). A memo with an effect that
  // revokes it broke under StrictMode, which unmounts once on mount.
  const showFile = useCallback(
    (el: HTMLImageElement | null) => {
      if (!el) return
      const u = URL.createObjectURL(file)
      el.src = u
      return () => URL.revokeObjectURL(u)
    },
    [file]
  )
  const remove = (
    <button
      type="button"
      className="text-muted-foreground hover:text-foreground disabled:opacity-50"
      aria-label={`Remove ${name}`}
      title="Remove"
      disabled={disabled}
      onClick={onRemove}
    >
      <XIcon className="size-3" />
    </button>
  )
  if (image) {
    return (
      <li className="group relative" title={name}>
        <img
          ref={showFile}
          alt={name}
          draggable={false}
          className="h-20 max-w-40 min-w-12 rounded-md object-cover ring-1 ring-border"
        />
        <span className="absolute top-1 right-1 flex rounded-full bg-background/85 p-0.5 shadow-sm ring-1 ring-border">
          {remove}
        </span>
      </li>
    )
  }
  return (
    <li className="flex h-8 items-center gap-1.5 rounded-md bg-secondary px-2 text-xs ring-1 ring-border">
      <FileKindIcon
        name={file.name}
        mime={file.type}
        className="size-4 shrink-0 text-muted-foreground"
      />
      <span className="max-w-48 truncate" title={name}>
        {name}
      </span>
      {remove}
    </li>
  )
}
