// LeaseNoticesLine is Home's line for a lease another of the person's
// devices still holds while it is offline (card #786). Irori never
// releases one: only Release here does, and Undo takes the notice back.

import { useEffect, useState } from "react"
import { KeyRoundIcon } from "lucide-react"

import type { LeaseNotice } from "@/gen/bindings"
import { inWails } from "@/lib/irori"
import { BoardService } from "@/lib/host"

export function LeaseNoticesLine() {
  const [list, setList] = useState<LeaseNotice[]>([])
  const [error, setError] = useState("")
  const load = () => {
    if (!inWails()) return
    BoardService.LeaseNotices().then(setList, () => {})
  }
  useEffect(() => {
    load()
    const t = setInterval(load, 30_000)
    window.addEventListener("focus", load)
    return () => {
      clearInterval(t)
      window.removeEventListener("focus", load)
    }
  }, [])
  if (list.length === 0) return null
  const act = (f: Promise<unknown>, id: string) => {
    setError("")
    f.then(
      () => setList((l) => l.filter((n) => n.id !== id)),
      (e: unknown) => setError(e instanceof Error ? e.message : String(e))
    )
  }
  return (
    <div
      role="status"
      className="grid gap-1 px-1 text-[12.5px] text-muted-foreground"
    >
      {list.map((n) => (
        <div key={n.id} className="flex items-center gap-2">
          <KeyRoundIcon className="size-3.5 shrink-0" aria-hidden />
          <span className="min-w-0 flex-1">{n.text}</span>
          <button
            type="button"
            className="cursor-pointer font-medium text-ink-2 underline underline-offset-2 hover:text-ink"
            onClick={() => act(BoardService.ReleaseLease(n.id), n.id)}
          >
            Release
          </button>
          <button
            type="button"
            className="cursor-pointer font-medium text-ink-2 underline underline-offset-2 hover:text-ink"
            onClick={() => act(BoardService.UndoLeaseNotice(n.id), n.id)}
          >
            Undo
          </button>
        </div>
      ))}
      {error && <span className="text-destructive">{error}</span>}
    </div>
  )
}
