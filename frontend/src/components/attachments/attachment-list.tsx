import { useEffect, useRef, useState, type RefObject } from "react"
import { cn } from "cn"
import {
  DownloadIcon,
  ExternalLinkIcon,
  FileIcon,
  ImageIcon,
  ImageOffIcon,
  LoaderCircleIcon,
  RotateCwIcon,
  WifiOffIcon,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  canOpen,
  errorText,
  humanSize,
  load,
  looksLikeImage,
  MaxAutoPreview,
  open,
  save,
  type FileRef,
  type Loaded,
} from "@/lib/attachments"
import { plainLine } from "@/lib/irori"

// AttachmentList shows the files on a Board comment (card #14): images as
// thumbnails that open full size (a big one loads on a click), everything
// else as a chip with Save, and Open for types the OS shows rather than
// runs. Files on another device are fetched when shown; when that device
// is offline the item says so and offers Retry. Drop it under a comment:
//
//   <AttachmentList attachments={comment.attachments ?? []} />
export function AttachmentList({
  attachments,
  className,
}: {
  attachments: FileRef[]
  className?: string
}) {
  if (attachments.length === 0) return null
  const images = attachments.filter(looksLikeImage)
  const files = attachments.filter((f) => !looksLikeImage(f))
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      {images.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {images.map((f) => (
            <Thumbnail key={f.sha256} file={f} />
          ))}
        </div>
      )}
      {files.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {files.map((f) => (
            <FileChip key={f.sha256} file={f} />
          ))}
        </div>
      )}
    </div>
  )
}

// fileName is the name to show: one plain line.
function fileName(f: FileRef): string {
  return plainLine(f.name) || "attachment"
}

// useVisible turns true once the element has scrolled near the view, so a
// long thread doesn't fetch every picture at once.
function useVisible<T extends Element>(): [RefObject<T | null>, boolean] {
  const ref = useRef<T>(null)
  const [visible, setVisible] = useState(
    () => typeof IntersectionObserver === "undefined"
  )
  useEffect(() => {
    const el = ref.current
    if (!el || visible) return
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) setVisible(true)
      },
      { rootMargin: "200px" }
    )
    io.observe(el)
    return () => io.disconnect()
  }, [visible])
  return [ref, visible]
}

// useLoaded fetches a file once visible; retry fetches again.
function useLoaded(sha: string, visible: boolean) {
  const [attempt, setAttempt] = useState(0)
  const key = `${sha}:${attempt}`
  // The answer for one sha and attempt; a newer key shows as loading.
  const [got, setGot] = useState<{ key: string; loaded: Loaded } | null>(null)
  useEffect(() => {
    if (!visible) return
    const ctl = new AbortController()
    let url: string | null = null
    load(sha, ctl.signal).then(
      (l) => {
        if (l.kind === "image" && l.url.startsWith("blob:")) url = l.url
        if (ctl.signal.aborted) {
          if (url) URL.revokeObjectURL(url)
          return
        }
        setGot({ key, loaded: l })
      },
      () => {}
    )
    return () => {
      ctl.abort()
      if (url) URL.revokeObjectURL(url)
    }
  }, [sha, visible, key])
  return {
    state: got?.key === key ? got.loaded : null,
    retry: () => setAttempt((n) => n + 1),
  }
}

function Thumbnail({ file }: { file: FileRef }) {
  const [ref, visible] = useVisible<HTMLDivElement>()
  // A big picture crosses devices whole: only when asked.
  const [wanted, setWanted] = useState(file.size <= MaxAutoPreview)
  const { state, retry } = useLoaded(file.sha256, visible && wanted)
  const [full, setFull] = useState(false)
  const name = fileName(file)

  // The route serves it as a download after all: show a chip.
  if (state?.kind === "file") return <FileChip file={file} />

  return (
    <div ref={ref} className="w-40">
      {state?.kind === "image" ? (
        <button
          type="button"
          onClick={() => setFull(true)}
          title={`${name} (${humanSize(file.size)})`}
          className="block h-28 w-40 overflow-hidden rounded-lg border border-border bg-muted outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
        >
          <img
            src={state.url}
            alt={name}
            draggable={false}
            className="size-full object-cover"
          />
        </button>
      ) : state?.kind === "error" ? (
        <div
          className="flex h-28 w-40 flex-col items-center justify-center gap-1.5 rounded-lg border border-dashed border-border bg-muted/40 p-2 text-center text-xs text-muted-foreground"
          title={state.message}
        >
          {state.offline ? (
            <WifiOffIcon className="size-4" />
          ) : (
            <ImageOffIcon className="size-4" />
          )}
          <span className="line-clamp-2">
            {state.offline ? offlineLine(state.message) : "Couldn't load it"}
          </span>
          <Button size="xs" variant="ghost" onClick={retry}>
            <RotateCwIcon />
            Retry
          </Button>
        </div>
      ) : !wanted ? (
        <div className="flex h-28 w-40 flex-col items-center justify-center gap-1.5 rounded-lg border border-border bg-muted/40 p-2 text-center text-xs text-muted-foreground">
          <ImageIcon className="size-4" />
          <Button size="xs" variant="ghost" onClick={() => setWanted(true)}>
            Load preview ({humanSize(file.size)})
          </Button>
        </div>
      ) : (
        <div
          className="flex h-28 w-40 animate-pulse items-center justify-center rounded-lg border border-border bg-muted/60 text-muted-foreground"
          aria-label={`Loading ${name}`}
        >
          <LoaderCircleIcon className="size-4 animate-spin" />
        </div>
      )}
      <div className="mt-1 truncate text-xs text-muted-foreground" title={name}>
        {name}
      </div>
      {state?.kind === "image" && (
        <Dialog open={full} onOpenChange={setFull}>
          <DialogContent className="sm:max-w-[min(90vw,1200px)]">
            <DialogHeader>
              <DialogTitle className="truncate">{name}</DialogTitle>
              <DialogDescription>{humanSize(file.size)}</DialogDescription>
            </DialogHeader>
            <div className="flex max-h-[75vh] justify-center overflow-auto rounded-lg bg-muted/40">
              <img
                src={state.url}
                alt={name}
                className="max-h-[75vh] max-w-full object-contain"
              />
            </div>
            <DialogFooter>
              <FileActions file={file} />
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  )
}

// offlineLine shortens the route's offline reason for a thumbnail: the
// device's name when it can find it.
function offlineLine(message: string): string {
  const m = /^the file is on (.+?), which isn't connected/.exec(message)
  return m ? `On ${m[1]}, offline` : "Its device is offline"
}

function FileChip({ file }: { file: FileRef }) {
  const name = fileName(file)
  return (
    <div className="flex max-w-full min-w-0 items-center gap-2 rounded-lg border border-border bg-muted/40 py-1 pr-1 pl-2 text-sm">
      <FileIcon className="size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate" title={name}>
        {name}
      </span>
      <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
        {humanSize(file.size)}
      </span>
      <FileActions file={file} compact />
    </div>
  )
}

// FileActions are Save (to Downloads) and, for a type the OS shows rather
// than runs, Open (with the OS default app), with what happened on one
// line.
function FileActions({ file, compact }: { file: FileRef; compact?: boolean }) {
  const [busy, setBusy] = useState<"save" | "open" | null>(null)
  const [note, setNote] = useState<{ text: string; error: boolean } | null>(
    null
  )

  const run = async (what: "save" | "open") => {
    setBusy(what)
    setNote(null)
    try {
      if (what === "save") {
        const path = await save(file.sha256)
        setNote({ text: `Saved to ${path}`, error: false })
      } else {
        await open(file.sha256)
      }
    } catch (err) {
      setNote({ text: errorText(err), error: true })
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="flex min-w-0 items-center gap-1">
      <Button
        size={compact ? "xs" : "sm"}
        variant="ghost"
        disabled={busy !== null}
        onClick={() => run("save")}
        title="Save to Downloads"
      >
        {busy === "save" ? (
          <LoaderCircleIcon className="animate-spin" />
        ) : (
          <DownloadIcon />
        )}
        Save
      </Button>
      {canOpen(file) && (
        <Button
          size={compact ? "xs" : "sm"}
          variant="ghost"
          disabled={busy !== null}
          onClick={() => run("open")}
          title="Open with the default app"
        >
          {busy === "open" ? (
            <LoaderCircleIcon className="animate-spin" />
          ) : (
            <ExternalLinkIcon />
          )}
          Open
        </Button>
      )}
      {note && (
        <span
          className={cn(
            "max-w-64 truncate text-xs",
            note.error ? "text-destructive" : "text-muted-foreground"
          )}
          title={note.text}
          role={note.error ? "alert" : "status"}
        >
          {note.error && /isn't connected/.test(note.text) ? (
            <WifiOffIcon className="mr-1 inline size-3" />
          ) : null}
          {note.text}
        </span>
      )}
    </div>
  )
}
