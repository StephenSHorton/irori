// <Terminal> draws one live terminal with suzuri's view package (ui/view,
// inline-terminals.md section 4): it mounts on its box, gets frames as raw
// ArrayBuffers and the rest as TermMsg, and sends keys, size and acks back
// through lib/term. Unmounting detaches; the shell keeps running.

import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react"
import { GOHU_FONT, mount, type TermView } from "suzuri-view"
import "suzuri-view/view.css"

import { Button } from "@/components/ui/button"
import { cards } from "@/lib/cards"
import { copyText } from "@/lib/clipboard"
import { attachTerm, detachTerm, termHost, type TermMsg } from "@/lib/term"
import {
  clampHeight,
  defaultHeight,
  maxHeight,
  minHeight,
  readHeight,
  saveHeight,
} from "@/lib/term-height"
import { cn } from "@/lib/utils"

export function Terminal({
  id,
  cwd,
  onRestart,
}: {
  id: number
  cwd: string
  onRestart: () => void
}) {
  const box = useRef<HTMLDivElement>(null)
  const [exit, setExit] = useState<{ code: number | null } | null>(null)
  const [clip, setClip] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  // The box's height (card #730): the one last chosen on this device.
  const [height, setHeight] = useState(readHeight)
  const [dragging, setDragging] = useState(false)
  const drag = useRef(false)

  useEffect(() => {
    const el = box.current
    if (!el) return
    const host = termHost(id, {
      onError: (e) => setError(e.message),
      openUrl: (url) => void cards.openURL(url),
    })
    const view: TermView = mount(el, host)
    view.setStyle({
      bg: "#0c0c0c",
      selection: "rgba(184, 160, 200, 0.38)",
      cursor: "#b8a0c8",
      font: GOHU_FONT,
    })
    const onMsg = (m: TermMsg) => {
      if (m.t === "exit") {
        setExit({ code: m.code })
      } else if (m.t === "clip") {
        // A program's copy is offered, never written for it (design 5).
        setClip(m.text)
        return
      }
      view.message(m)
    }
    attachTerm(id, (f) => view.feed(f), onMsg).catch((e: Error) =>
      setError(e.message)
    )
    view.focus()
    return () => {
      detachTerm(id)
      view.unmount()
    }
  }, [id])

  // Dragging the bottom edge follows the pointer on the window rather than
  // through pointer capture, which WKWebView drops after the first move
  // (as the sidebar's edge does). The view refits by its ResizeObserver.
  const startDrag = (e: ReactPointerEvent<HTMLDivElement>) => {
    const el = box.current
    if (e.button !== 0 || drag.current || !el) return
    e.preventDefault()
    drag.current = true
    setDragging(true)
    const y0 = e.clientY
    const h0 = el.getBoundingClientRect().height
    let h = h0
    // The page under the pointer would show its own cursor and select text.
    document.body.style.cursor = "row-resize"
    document.body.style.userSelect = "none"
    const move = (ev: PointerEvent) => {
      h = clampHeight(h0 + ev.clientY - y0, window.innerHeight)
      setHeight(h)
    }
    const end = () => {
      window.removeEventListener("pointermove", move)
      window.removeEventListener("pointerup", end)
      window.removeEventListener("pointercancel", end)
      window.removeEventListener("blur", end)
      drag.current = false
      setDragging(false)
      document.body.style.cursor = ""
      document.body.style.userSelect = ""
      saveHeight(h)
    }
    window.addEventListener("pointermove", move)
    window.addEventListener("pointerup", end)
    window.addEventListener("pointercancel", end)
    window.addEventListener("blur", end)
  }
  // The arrow keys move the edge too, a step of 16px (Shift: 64).
  const keyEdge = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 64 : 16
    const now = clampHeight(height, window.innerHeight)
    let next: number
    if (e.key === "ArrowUp") next = now - step
    else if (e.key === "ArrowDown") next = now + step
    else if (e.key === "Home") next = minHeight
    else if (e.key === "End") next = maxHeight(window.innerHeight)
    else return
    e.preventDefault()
    next = clampHeight(next, window.innerHeight)
    setHeight(next)
    saveHeight(next)
  }
  const resetHeight = () => {
    setHeight(defaultHeight)
    saveHeight(null)
  }

  return (
    <div className="grid gap-2">
      <div>
        <div
          ref={box}
          // The window's height caps it in CSS too, so a window made
          // smaller than the saved height still shows the handle.
          className="max-h-[calc(100svh-6rem)] min-h-[200px] overflow-hidden rounded-md border border-border bg-[#0c0c0c]"
          style={{ height }}
          aria-label={`Terminal in ${cwd}`}
        />
        <div
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize the terminal"
          aria-valuemin={minHeight}
          aria-valuemax={maxHeight(window.innerHeight)}
          aria-valuenow={clampHeight(height, window.innerHeight)}
          tabIndex={0}
          title="Drag to change the terminal's height. Double-click: default height"
          onPointerDown={startDrag}
          onDoubleClick={resetHeight}
          onKeyDown={keyEdge}
          className={cn(
            // 12px to grab under the box, a grip in the middle.
            "group flex h-3 cursor-row-resize touch-none items-center justify-center rounded-sm outline-none",
            "focus-visible:ring-2 focus-visible:ring-ring/50"
          )}
        >
          <span
            aria-hidden
            className={cn(
              "h-1 w-10 rounded-full bg-muted-foreground/40 transition-colors",
              "group-hover:bg-primary/70 group-focus-visible:bg-primary",
              dragging && "bg-primary"
            )}
          />
        </div>
      </div>
      {clip !== null && (
        <div className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
          <span className="min-w-0 truncate">
            The program wants to copy: <span className="font-mono">{clip}</span>
          </span>
          <Button
            size="sm"
            variant="outline"
            className="h-6"
            onClick={() => {
              void copyText(clip)
              setClip(null)
            }}
          >
            Copy
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-6"
            onClick={() => setClip(null)}
          >
            Dismiss
          </Button>
        </div>
      )}
      {exit && (
        <div className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
          <span>
            The shell ended{exit.code !== null ? ` (exit ${exit.code})` : ""}.
          </span>
          <Button
            size="sm"
            variant="outline"
            className="h-6"
            onClick={onRestart}
          >
            Restart
          </Button>
        </div>
      )}
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </div>
  )
}
