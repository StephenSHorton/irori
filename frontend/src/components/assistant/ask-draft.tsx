// An ask the assistant drafted for someone else's assistant (card #1063,
// docs/design/assistants-talk.md §9). The model wrote it and cannot post
// it: this shows the exact text the host normalised, and only the tap on
// Send posts that same text, as the person's own assistant card.

import { useEffect, useState } from "react"
import { CheckIcon, Loader2Icon } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  prepareAsk,
  sendAsk,
  wasSent,
  type AskDraft,
} from "@/lib/assistant-ask"
import type { AssistantAskPrepared } from "@/gen/bindings"
import { errorText } from "@/lib/talk"

type Prep =
  | { state: "loading" }
  | { state: "error"; why: string }
  | { state: "ready"; p: AssistantAskPrepared; sent: boolean }

// askTitle is "Ask Addam's assistant · Space".
export function askTitle(p: Pick<AssistantAskPrepared, "name" | "spaceName">) {
  return `Ask ${p.name}'s assistant${p.spaceName ? ` · ${p.spaceName}` : ""}`
}

export function AskDraftCard({ ask }: { ask: AskDraft }) {
  const [prep, setPrep] = useState<Prep>({ state: "loading" })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const writing = ask.partial
  const { person, space, text } = ask
  // The parent keys this on the ask, so a new one starts at "loading".
  useEffect(() => {
    if (writing) return
    let live = true
    prepareAsk({ person, space, text, partial: false })
      .then(async (p) => {
        const sent = await wasSent(p)
        if (live) setPrep({ state: "ready", p, sent })
      })
      .catch((err) => {
        if (live) setPrep({ state: "error", why: errorText(err) })
      })
    return () => {
      live = false
    }
  }, [person, space, text, writing])
  const send = async (p: AssistantAskPrepared) => {
    if (busy) return
    setBusy(true)
    setError("")
    try {
      await sendAsk(p)
      setPrep({ state: "ready", p, sent: true })
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const p = prep.state === "ready" ? prep.p : null
  return (
    <div
      className="grid gap-2 rounded-lg border border-border bg-background/60 p-2.5"
      data-assistant-ask=""
    >
      <p className="text-[11.5px] font-medium tracking-wide text-muted-foreground uppercase">
        {p ? askTitle(p) : "Ask someone's assistant"}
      </p>
      {writing ? (
        <p className="text-[12.5px] text-muted-foreground">
          The question is still being written, or was cut short. Ask again for
          the whole thing.
        </p>
      ) : prep.state === "loading" ? (
        <p className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
          <Loader2Icon
            className="size-3.5 animate-spin motion-reduce:animate-none"
            aria-hidden
          />
          Checking who that is…
        </p>
      ) : prep.state === "error" ? (
        <p className="text-[12.5px] text-destructive" role="alert">
          Can't send that: {prep.why}
        </p>
      ) : (
        <>
          {/* The exact text a tap posts. */}
          <div
            className="max-h-44 overflow-y-auto overscroll-contain text-[13px] [overflow-wrap:anywhere] whitespace-pre-wrap"
            data-assistant-ask-text=""
          >
            {prep.p.text}
          </div>
          {prep.sent ? (
            <p className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
              <CheckIcon className="size-3.5 text-success" aria-hidden />
              Sent. {prep.p.name}'s assistant will answer here.
            </p>
          ) : prep.p.held ? (
            <p className="text-[12.5px] text-destructive" role="alert">
              {prep.p.held}.
            </p>
          ) : (
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                size="sm"
                disabled={busy}
                onClick={() => void send(prep.p)}
              >
                {busy && (
                  <Loader2Icon
                    className="animate-spin motion-reduce:animate-none"
                    aria-hidden
                  />
                )}
                Send
              </Button>
              <span className="text-[12px] text-muted-foreground">
                Everyone in {prep.p.spaceName || "the space"} can read it.
              </span>
              {!busy && error && (
                <span className="text-[12px] text-destructive" role="alert">
                  Couldn't send it: {error}
                </span>
              )}
            </div>
          )}
        </>
      )}
    </div>
  )
}
