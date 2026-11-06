import { useState, type FormEvent } from "react"

import {
  AttachButton,
  PendingFiles,
  useFileAttach,
} from "@/components/attachments"

import { AgentsNowToggle } from "@/components/common/agents-now-toggle"
import { MentionTextarea } from "@/components/common/mention-textarea"
import { Button } from "@/components/ui/button"
import { useBoards } from "@/lib/boards"
import { useAgentsNow } from "@/lib/agents-now"
import { threads } from "@/lib/cards"
import { useDraft, useMentionsDraft } from "@/lib/drafts"
import { composing, MentionBoard } from "@/lib/mentions"
import { errorText } from "@/lib/talk"

// AskBox is the box over the grid that starts a thread (cards #116, #134):
// a question or a conversation with the room, not a card. The agents on
// the cards it names (#12) and the agents it @mentions hear it, woken,
// and nobody waits on them (card #512): at once an idle agent with Offer
// it cards on gets it, or with none free Irori starts its own agent (on a
// shared board, on the poster's device). It shows in
// the Threads panel, where the person replies and resolves it. Enter
// sends, Shift+Enter is a new line. Files go with it (card #124): pick them
// with the clip, drop them on the box, or paste a screenshot. On a shared
// board's page it starts the thread on that board: only that board's
// agents and people see it, the other people's agents hear it through
// their own devices, and its page lists it in its own Threads panel. A
// board thread takes no mentions (a board log carries none; name a card
// instead) and no files yet. New cards come from the header's New card.
// "Agents answer now" (card #391) starts off when the text names only
// people: then no agent is woken or handed it, and the agents on the cards
// it names read it at their next turn.
export function AskBox({
  onThread,
  board: onPage = "",
}: {
  // onThread gets the new thread, for the Threads panel to open.
  onThread: (id: string) => void
  board?: string
}) {
  const draft = onPage ? `ask/board/${onPage}` : "ask/room"
  const [text, setText] = useDraft(draft)
  const [mentions, setMentions] = useMentionsDraft(draft)
  const agentsNow = useAgentsNow(mentions)
  const { boards } = useBoards()
  // The page's board, while it is still a ready board of ours.
  const onBoard = boards.some(
    (b) => b.boardId === onPage && b.ready && !b.leaving
  )
    ? onPage
    : ""
  const [busy, setBusy] = useState(false)
  const attach = useFileAttach(busy)
  const files = attach.files
  const [line, setLine] = useState<{ text: string; error?: boolean } | null>(
    null
  )

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (busy || !text.trim()) return
    if (onPage && !onBoard) {
      setLine({ text: "This space isn't ready here yet.", error: true })
      return
    }
    if (onBoard && files.length) {
      setLine({
        text: "Files don't go on a space thread yet: remove them, or attach them on a card.",
        error: true,
      })
      return
    }
    setBusy(true)
    setLine(null)
    try {
      const now = agentsNow.on
      const res = onBoard
        ? await threads.start(text, mentions, [], onBoard, now)
        : await threads.start(text, mentions, files, "", now)
      // On a board the other people's agents are told by their own
      // devices: count every agent the thread's round names.
      const n = onBoard ? (res.task.ask?.told ?? 0) : (res.told.to?.length ?? 0)
      // Handed at once to an idle agent with Offer it cards on: no agent
      // starts for it.
      const handed = !!res.task.ask?.taker
      const more = handed
        ? ""
        : " An idle agent with Offer it cards on picks it up, or Irori starts one when spawning is on."
      setLine({
        text: !now
          ? "Sent. Agents answer now was off: no agent was woken or handed it."
          : handed && n <= 1
            ? "Sent. An agent with Offer it cards on has it."
            : n === 0
              ? `Sent.${more}`
              : `Sent to ${n === 1 ? "1 agent" : `${n} agents`}.${more}`,
      })
      setText("")
      setMentions([])
      agentsNow.reset()
      attach.clear()
      onThread(res.task.id)
    } catch (err) {
      setLine({ text: errorText(err), error: true })
    } finally {
      setBusy(false)
    }
  }

  // On a board the @ picker offers its other people and their agents too
  // (cards #177, #187); they hear it through their own devices.
  return (
    <MentionBoard.Provider value={onBoard}>
      <form
        className="grid gap-1"
        onSubmit={(e) => void submit(e)}
        {...attach.drop}
      >
        <div className="flex items-start gap-2 p-1.5 glass-panel">
          <MentionTextarea
            autoSize
            voice="main"
            rows={1}
            value={text}
            mentions={mentions}
            onChange={(v, ms) => {
              setText(v)
              setMentions(ms)
              setLine(null)
            }}
            onPaste={(e) => {
              if (attach.onPaste(e)) setLine(null)
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !composing(e)) {
                e.preventDefault()
                e.currentTarget.form?.requestSubmit()
              }
            }}
            placeholder="Ask about anything…"
            aria-label="Start a thread"
            className="max-h-40 min-h-9 flex-1 border-transparent bg-transparent shadow-none focus-visible:border-transparent"
            // readOnly, not disabled: disabling the focused box would blur it,
            // and the next message would need a click back in.
            readOnly={busy}
            aria-busy={busy}
          />
          {/* The controls ride the box's first line (h-9, its min height)
            whatever their own heights, and stay up top as it grows. */}
          <div className="flex h-9 shrink-0 items-center gap-2">
            <AgentsNowToggle
              on={agentsNow.on}
              onChange={agentsNow.set}
              disabled={busy}
            />
            {attach.input}
            <AttachButton attach={attach} disabled={busy || !!onBoard} />
            <Button
              type="submit"
              size="sm"
              className="shrink-0"
              disabled={busy || !text.trim()}
            >
              {busy ? "Sending…" : "Send"}
            </Button>
          </div>
        </div>
        <PendingFiles
          attach={attach}
          disabled={busy}
          label="Files with the thread"
        />
        {line && (
          <p
            className={
              line.error
                ? "px-1 text-xs text-destructive"
                : "px-1 text-xs text-muted-foreground"
            }
            role={line.error ? "alert" : undefined}
          >
            {line.text}
          </p>
        )}
      </form>
    </MentionBoard.Provider>
  )
}
