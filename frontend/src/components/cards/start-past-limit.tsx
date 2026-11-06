// StartPastLimit is Start an agent on a card or thread that a limit keeps
// waiting (cards #450, #576): past the daily limit, a card starts only
// when the person asks, and under a usage limit their ask tries now rather
// than wait for the reset. Nothing when neither stops it.

import { useState } from "react"

import { Button } from "@/components/ui/button"
import { cards } from "@/lib/cards"
import { errorText } from "@/lib/talk"
import { startPastLimit, type SpawnState } from "@/lib/spawn-line"

export function StartPastLimit({
  id,
  state,
  size = "sm",
}: {
  id: string
  state: SpawnState
  size?: "sm" | "xs"
}) {
  const [asked, setAsked] = useState("")
  const title = startPastLimit(state)
  if (!title) return null
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-2 text-[13px]">
      <Button
        size={size}
        variant="outline"
        disabled={!!asked}
        title={title}
        onClick={() => {
          setAsked("Starting…")
          cards.startAgent(id, "").then(
            () => setAsked("Asked: an agent starts in a moment."),
            (e: Error) => setAsked(errorText(e))
          )
        }}
      >
        Start an agent
      </Button>
      {asked && <span className="text-muted-foreground">{asked}</span>}
    </span>
  )
}
