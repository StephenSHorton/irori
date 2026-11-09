// A labelled line in the Office Assistant chat (card #1062,
// docs/design/assistants-talk.md §8): what another person's assistant said
// to this person, or what theirs was asked or said. It is someone else's
// words: plain text, clipped, never parsed for drafts, go lines or
// auto-go, and links show as their URL.

import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import {
  answerThis,
  isAsking,
  letItAnswer,
  needsText,
  type From,
} from "@/lib/assistant-talk-follow"

import type { ChatEntry } from "@/lib/assistant"

const kindLabel: Record<From["kind"], string> = {
  answer: "Answer",
  needs: "Needs you",
  asked: "",
  held: "",
}

// fromHeader is "From Juno · Eve's assistant · #office-assistants on mlg".
export function fromHeader(f: From): string {
  const where = `#office-assistants${f.spaceName ? ` on ${f.spaceName}` : ""}`
  return f.kind === "asked"
    ? `Your assistant answered ${f.name} · ${where}`
    : `From ${f.name} · ${where}`
}

export function FromLine({
  entry,
  onTalk,
  drafted = false,
}: {
  entry: ChatEntry
  onTalk: (conv: string) => void
  // The owner's-OK draft for this question is in the chat already (card
  // #1064): Answer this isn't offered again.
  drafted?: boolean
}) {
  const f = entry.from
  if (!f) return null
  const quiet = f.kind === "asked" || f.kind === "held"
  return (
    <div
      className={
        quiet
          ? "grid min-w-0 gap-1.5 border-l-2 border-border pl-2.5 text-[13px] leading-relaxed [overflow-wrap:anywhere] text-muted-foreground"
          : "grid min-w-0 gap-1.5 border-l-2 border-primary/40 pl-2.5 text-[13px] leading-relaxed [overflow-wrap:anywhere]"
      }
      data-assistant-from={f.kind}
    >
      <p className="text-[11.5px] text-muted-foreground">
        {kindLabel[f.kind] && (
          <span className="mr-1 font-semibold tracking-wide text-primary uppercase">
            {kindLabel[f.kind]} ·
          </span>
        )}
        {fromHeader(f)}
      </p>
      {f.kind === "needs" ? (
        <>
          {f.question && <p className="whitespace-pre-wrap">{f.question}</p>}
          <p className="whitespace-pre-wrap">
            {entry.text || needsText(f.waiting)}
          </p>
        </>
      ) : f.kind === "asked" ? (
        <p className="whitespace-pre-wrap">“{entry.text}”</p>
      ) : (
        <>
          <p className="whitespace-pre-wrap">{entry.text}</p>
          {f.question && (
            <p className="text-muted-foreground">You asked: {f.question}</p>
          )}
        </>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {f.kind === "needs" && f.waiting === "ask" && <LetItAnswer from={f} />}
        {f.kind === "needs" && f.waiting === "owner" && !drafted && (
          <LetItAnswer from={f} how="draft" />
        )}
        <Button
          type="button"
          variant="link"
          size="sm"
          className="h-auto p-0 text-[12.5px]"
          onClick={() => onTalk(f.conv)}
        >
          Open in Talk
        </Button>
      </div>
    </div>
  )
}

// LetItAnswer taps the request once; it says it is asking until the line
// goes (the answer is posted) or the tap lapses.
function LetItAnswer({
  from,
  how = "let",
}: {
  from: From
  how?: "let" | "draft"
}) {
  const [state, setState] = useState<"" | "asking">(
    isAsking(from.postId, undefined, how) ? "asking" : ""
  )
  const [error, setError] = useState("")
  useEffect(() => {
    if (state !== "asking") return
    const id = window.setTimeout(() => setState(""), 30_000)
    return () => window.clearTimeout(id)
  }, [state])
  return (
    <>
      <Button
        type="button"
        size="sm"
        disabled={state === "asking"}
        onClick={() => {
          if (state === "asking") return
          setError("")
          setState("asking")
          const tap = how === "draft" ? answerThis(from) : letItAnswer(from)
          void tap.then((why) => {
            if (why) {
              setError(why)
              setState("")
            }
          })
        }}
      >
        {state === "asking"
          ? how === "draft"
            ? "Drafting your answer…"
            : "Asking your assistant…"
          : how === "draft"
            ? "Answer this"
            : "Let it answer"}
      </Button>
      {error && (
        <span className="text-[12px] text-destructive" role="alert">
          {error}
        </span>
      )}
    </>
  )
}
