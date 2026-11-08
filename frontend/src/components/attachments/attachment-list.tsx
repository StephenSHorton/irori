import { useEffect, useRef, useState, type RefObject } from "react"
import { cn } from "cn"
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  DownloadIcon,
  ExternalLinkIcon,
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
import { navDirection, stepIndex } from "@/lib/image-viewer"
import { plainLine } from "@/lib/irori"

import { FileKindIcon } from "./file-kind-icon"

// AttachmentList shows the files on a Board comment (card #14): images as
// thumbnails that open full size (a big one loads on a click), everything
// else as a chip with Save, and Open for types the OS shows rather than
// runs. Files on another device are fetched when shown; when that device
// is offline the item says so and offers Retry. Drop it under a comment:
//
//   <AttachmentList attachments={comment.attachments ?? []} />
//
// compact lists every file as a chip, pictures too: for a list of files
// whose pictures show elsewhere (a card page's side column).
export function AttachmentList({
  attachments,
  className,
  compact,
}: {
  attachments: FileRef[]
  className?: string
  compact?: boolean
}) {
  // The picture open in the viewer (an index into images), and what the
  // thumbnails have already loaded, so the viewer doesn't fetch it twice.
  const [viewing, setViewing] = useState<number | null>(null)
  const [loaded, setLoaded] = useState<Record<string, Loaded>>({})
  if (attachments.length === 0) return null
  const images = compact ? [] : attachments.filter(looksLikeImage)
  const files = compact
    ? attachments
    : attachments.filter((f) => !looksLikeImage(f))
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      {images.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {images.map((f, i) => (
            <Thumbnail
              key={f.sha256}
              file={f}
              onOpen={() => setViewing(i)}
              onLoaded={(l) =>
                setLoaded((m) =>
                  m[f.sha256] === l ? m : { ...m, [f.sha256]: l }
                )
              }
            />
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
      {images.length > 0 && (
        <ImageViewer
          images={images}
          index={viewing}
          loaded={loaded}
          onIndex={setViewing}
        />
      )}
    </div>
  )
}

// ImageViewer is the one full-size view for a message's pictures: Previous
// and Next (and the arrow keys) wrap around when there are several. A
// picture the thumbnail hasn't loaded (a big one waits for a click) loads
// when the viewer reaches it, under a loading state.
function ImageViewer({
  images,
  index,
  loaded,
  onIndex,
}: {
  images: FileRef[]
  index: number | null
  loaded: Record<string, Loaded>
  onIndex: (i: number | null) => void
}) {
  const cur = index !== null && index < images.length ? index : null
  const open = cur !== null
  const file = cur !== null ? images[cur] : null
  const many = images.length > 1

  useEffect(() => {
    if (cur === null || !many) return
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return
      const dir = navDirection(e)
      if (dir === null) return
      e.preventDefault()
      onIndex(stepIndex(cur, dir, images.length))
    }
    // Capture: the dialog may stop the event before it bubbles to window.
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  }, [cur, many, images.length, onIndex])

  const name = file ? fileName(file) : ""
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onIndex(null)}>
      {file && (
        <DialogContent className="sm:max-w-[min(90vw,1200px)]">
          <DialogHeader>
            <DialogTitle className="truncate">{name}</DialogTitle>
            <DialogDescription>
              {humanSize(file.size)}
              {many && ` · ${(cur ?? 0) + 1} of ${images.length}`}
            </DialogDescription>
          </DialogHeader>
          <div className="relative flex max-h-[75vh] min-h-40 justify-center overflow-auto rounded-lg bg-muted/40">
            <ViewerImage
              key={file.sha256}
              file={file}
              name={name}
              have={loaded[file.sha256]}
            />
            {many && (
              <>
                <Button
                  size="icon"
                  variant="secondary"
                  className="absolute top-1/2 left-2 -translate-y-1/2"
                  aria-label="Previous image"
                  onClick={() =>
                    onIndex(stepIndex(cur ?? 0, -1, images.length))
                  }
                >
                  <ChevronLeftIcon />
                </Button>
                <Button
                  size="icon"
                  variant="secondary"
                  className="absolute top-1/2 right-2 -translate-y-1/2"
                  aria-label="Next image"
                  onClick={() => onIndex(stepIndex(cur ?? 0, 1, images.length))}
                >
                  <ChevronRightIcon />
                </Button>
              </>
            )}
          </div>
          <DialogFooter>
            <FileActions file={file} />
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  )
}

// ViewerImage shows one picture: the thumbnail's copy when it has one, else
// it loads the file now.
function ViewerImage({
  file,
  name,
  have,
}: {
  file: FileRef
  name: string
  have: Loaded | undefined
}) {
  const own = useLoaded(file.sha256, have?.kind !== "image")
  const state = have?.kind === "image" ? have : own.state
  if (state?.kind === "image") {
    return (
      <img
        src={state.url}
        alt={name}
        className="max-h-[75vh] max-w-full object-contain"
      />
    )
  }
  if (state) {
    return (
      <div className="flex flex-col items-center justify-center gap-1.5 p-6 text-center text-xs text-muted-foreground">
        <ImageOffIcon className="size-4" />
        <span>
          {state.kind === "error"
            ? state.offline
              ? offlineLine(state.message)
              : "Couldn't load it"
            : "Not a picture"}
        </span>
        {state.kind === "error" && (
          <Button size="xs" variant="ghost" onClick={own.retry}>
            <RotateCwIcon />
            Retry
          </Button>
        )}
      </div>
    )
  }
  return (
    <div
      className="flex items-center justify-center p-10 text-muted-foreground"
      aria-label={`Loading ${name}`}
    >
      <LoaderCircleIcon className="size-5 animate-spin" />
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

function Thumbnail({
  file,
  onOpen,
  onLoaded,
}: {
  file: FileRef
  onOpen: () => void
  onLoaded: (l: Loaded) => void
}) {
  const [ref, visible] = useVisible<HTMLDivElement>()
  // A big picture crosses devices whole: only when asked.
  const [wanted, setWanted] = useState(file.size <= MaxAutoPreview)
  const { state, retry } = useLoaded(file.sha256, visible && wanted)
  const name = fileName(file)
  useEffect(() => {
    if (state) onLoaded(state)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state])

  // The route serves it as a download after all: show a chip.
  if (state?.kind === "file") return <FileChip file={file} />

  return (
    <div ref={ref} className="w-40">
      {state?.kind === "image" ? (
        <button
          type="button"
          onClick={onOpen}
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
      <FileKindIcon
        name={file.name}
        mime={file.mime}
        className="size-4 shrink-0 text-muted-foreground"
      />
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
