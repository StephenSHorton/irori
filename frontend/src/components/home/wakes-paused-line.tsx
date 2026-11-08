// WakesPausedLine is Home's way out of the old inbox Pause all (card #787).
// Its switch is gone from the window and the tray: Stop everywhere is the
// one switch. A device whose wakes were paused before still holds every
// message, so while it is paused this line says so with Resume wakes, and
// nobody is stuck held with no button. Otherwise it shows nothing.

import { useState } from "react"
import { PlayIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { errorText, talk } from "@/lib/talk"

export function WakesPausedLine({
  paused,
  onResumed,
}: {
  // The inbox policy's paused flag (useInbox).
  paused: boolean
  onResumed: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  if (!paused) return null
  const resume = async () => {
    setBusy(true)
    setError("")
    try {
      await talk.setPaused(false)
      onResumed()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div
      role="status"
      className="flex flex-wrap items-center gap-2 px-1 text-[12.5px] text-muted-foreground"
    >
      <span className="min-w-0">
        Wakes are paused on this device: messages to its agents wait as held.
      </span>
      <Button
        size="xs"
        variant="outline"
        disabled={busy}
        onClick={() => void resume()}
      >
        <PlayIcon aria-hidden className="size-3" />
        Resume wakes
      </Button>
      {error && (
        <span className="basis-full text-destructive" role="alert">
          {error}
        </span>
      )}
    </div>
  )
}
