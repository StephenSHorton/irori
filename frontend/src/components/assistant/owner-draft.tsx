// An owner's-OK draft in the Office Assistant chat (card #1064,
// docs/design/assistants-talk.md §7): the answer the assistant drafted to
// another person's question, about private work. It is shown in full as
// plain text, never collapsed, never parsed for drafts or go lines, and what
// Send posts is exactly this text: the host normalises it before it is
// shown and again before it posts, so both are the same string.

import { useState } from "react"

import { Button } from "@/components/ui/button"
import { dismissDraft, sendDraft } from "@/lib/assistant-talk-follow"

import type { ChatEntry } from "@/lib/assistant"

export function OwnerDraftLine({ entry }: { entry: ChatEntry }) {
  const d = entry.ownerDraft
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  if (!d) return null
  const to = d.asker || "the asker"
  const run = (go: (request: string) => Promise<string>) => {
    if (busy) return
    setBusy(true)
    setError("")
    void go(d.request).then((why) => {
      setBusy(false)
      if (why) setError(why)
    })
  }
  return (
    <div
      className="grid min-w-0 gap-1.5 border-l-2 border-primary/40 pl-2.5 text-[13px] leading-relaxed [overflow-wrap:anywhere]"
      data-assistant-draft={d.closed || "open"}
    >
      <p className="text-[11.5px] text-muted-foreground">
        <span className="mr-1 font-semibold tracking-wide text-primary uppercase">
          Your answer ·
        </span>
        Drafted for {to}. Nothing is sent until you tap Send.
      </p>
      <p className="whitespace-pre-wrap" data-assistant-draft-text="">
        {entry.text}
      </p>
      {d.closed === "sent" && (
        <p className="text-[12.5px] text-muted-foreground">Sent to {to}.</p>
      )}
      {d.closed === "dismissed" && (
        <p className="text-[12.5px] text-muted-foreground">Not sent.</p>
      )}
      {!d.closed && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            size="sm"
            disabled={busy}
            onClick={() => run(sendDraft)}
          >
            Send to {to}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => run(dismissDraft)}
          >
            Don&apos;t send
          </Button>
        </div>
      )}
      {error && (
        <span className="text-[12px] text-destructive" role="alert">
          {error}
        </span>
      )}
    </div>
  )
}
