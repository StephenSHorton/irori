// <Terminal> draws one live terminal with suzuri's view package (ui/view,
// inline-terminals.md section 4): it mounts on its box, gets frames as raw
// ArrayBuffers and the rest as TermMsg, and sends keys, size and acks back
// through lib/term. Unmounting detaches; the shell keeps running.

import { useEffect, useRef, useState } from "react"
import { GOHU_FONT, mount, type TermView } from "suzuri-view"
import "suzuri-view/view.css"

import { Button } from "@/components/ui/button"
import { cn } from "cn"
import { cards } from "@/lib/cards"
import { copyText } from "@/lib/clipboard"
import { attachTerm, detachTerm, termHost, type TermMsg } from "@/lib/term"

export function Terminal({
  id,
  cwd,
  onRestart,
  fill = false,
  onCwd,
}: {
  id: number
  cwd: string
  onRestart: () => void
  // fill: the terminal takes its container's height (the bottom panel,
  // card #734) instead of its own resizable 360px.
  fill?: boolean
  // onCwd hears the folder the shell is in each time it reports one.
  onCwd?: (cwd: string) => void
}) {
  const box = useRef<HTMLDivElement>(null)
  // The latest onCwd, so a new callback doesn't remount the view.
  const cwdTo = useRef(onCwd)
  useEffect(() => {
    cwdTo.current = onCwd
  })
  const [exit, setExit] = useState<{ code: number | null } | null>(null)
  const [clip, setClip] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

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
      if (m.t === "shell") {
        if (m.cwd) cwdTo.current?.(m.cwd)
      } else if (m.t === "exit") {
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

  return (
    <div
      className={cn("gap-2", fill ? "flex min-h-0 flex-1 flex-col" : "grid")}
    >
      <div
        ref={box}
        className={cn(
          "overflow-hidden rounded-md border border-border bg-[#0c0c0c]",
          fill ? "min-h-0 flex-1" : "h-[360px] min-h-[200px] resize-y"
        )}
        aria-label={`Terminal in ${cwd}`}
      />
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
