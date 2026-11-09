import { useEffect, useState, type FormEvent } from "react"

import { Ago } from "@/components/common/ago"
import { Badge } from "@/components/ui/badge"
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
  AttachButton,
  PendingFiles,
  useFileAttach,
} from "@/components/attachments"
import { MentionTextarea } from "@/components/common/mention-textarea"
import { parseTime, plainBlock, plainLine } from "@/lib/irori"
import { notDelivered, type Mention } from "@/lib/mentions"
import {
  errorText,
  holdText,
  State,
  talk,
  useRepoNames,
  type Delivery,
} from "@/lib/talk"

// MessageDialog is one message a notification's Open named (there is no
// inbox in the window): a message Irori held, with Release and Dismiss, or
// one an agent sent the person, in full, with Reply and Mark read. A reply
// can @mention other agents (card #77): each gets its own copy, woken. When
// one wasn't delivered, the dialog stays open to say so.
export function MessageDialog({
  id,
  name,
  onClose,
}: {
  id: string
  name: (key: string) => string
  onClose: () => void
}) {
  const [d, setD] = useState<Delivery | null>(null)
  const repo = useRepoNames()
  const [error, setError] = useState<string | null>(null)
  const [reply, setReply] = useState("")
  const [mentions, setMentions] = useState<Mention[]>([])
  const [missed, setMissed] = useState("")
  const [busy, setBusy] = useState(false)
  // Files go with the Reply (card #195): the clip, a drop or a paste.
  const attach = useFileAttach(busy)

  useEffect(() => {
    let live = true
    talk.get(id).then(
      (x) => live && setD(x),
      (err) => live && setError(errorText(err))
    )
    return () => {
      live = false
    }
  }, [id])

  // act runs fn and closes, unless fn's answer says a mention wasn't
  // delivered: then the reply has gone, and the dialog says who missed it.
  const act = async (fn: () => Promise<unknown>) => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      const res = await fn()
      const line = Array.isArray(res) ? notDelivered(res) : ""
      if (line) {
        setReply("")
        setMentions([])
        attach.clear()
        setMissed(`Replied. ${line}`)
        return
      }
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!reply.trim()) {
      setError("Write a reply first.")
      return
    }
    const files = attach.files
    void act(() => talk.reply(id, reply, mentions, files))
  }

  // Stop and read (card #207): the sender's running turn is cut and the
  // reply is its next prompt. Text only: mentions and files go with Reply.
  const stopAndRead = () => {
    if (!reply.trim()) {
      setError("Write a reply first.")
      return
    }
    void act(() => talk.replyStop(id, reply))
  }

  const from = d
    ? plainLine(d.message.from.name) || name(d.message.from.key)
    : ""
  const to = !d
    ? ""
    : d.to.startsWith("user:")
      ? "you"
      : d.to
        ? name(d.to)
        : plainLine(repo(d.repoKey ?? "")) || "a repo"
  const held = d?.state === State.Held
  const forYou =
    !!d &&
    d.to.startsWith("user:") &&
    (d.state === State.Queued ||
      d.state === State.Delivered ||
      d.state === State.Engaged ||
      d.state === State.Deferred)
  const sent = parseTime(d?.message.sentAt)
  // An agent's message whose recipient had ended came to the person instead
  // (the inbox's recipient-gone fallback); a reply goes to its sender.
  const gone = d?.for?.startsWith("recipient gone") ? d.for : ""

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {!d
              ? error
                ? "Couldn't open the message"
                : "Opening the message…"
              : held
                ? `${from} sent ${to} a message Irori held`
                : `${from} sent ${to} a message`}
          </DialogTitle>
          {d && (
            <DialogDescription>
              {held && plainLine(holdText(d, { name, repo }))}{" "}
              <time
                dateTime={sent?.toISOString()}
                title={sent?.toLocaleString()}
              >
                <Ago at={sent} />
              </time>
              {!held && !forYou && ` · ${d.state}`}
            </DialogDescription>
          )}
        </DialogHeader>
        {gone && (
          <p className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
            <Badge variant="warning" className="h-4 px-1.5 text-[10.5px]">
              Recipient gone
            </Badge>
            {plainLine(gone)}
          </p>
        )}
        {d && (
          <div className="max-h-[50vh] overflow-y-auto rounded-lg border border-border bg-background/60 px-3 py-2.5 text-[13.5px] whitespace-pre-wrap text-ink">
            {plainBlock(d.message.text)}
          </div>
        )}
        {forYou && !missed && (
          <form
            id="message-reply"
            className="grid gap-2"
            onSubmit={submit}
            {...attach.drop}
          >
            <div className="flex items-start gap-1.5">
              <MentionTextarea
                value={reply}
                mentions={mentions}
                onChange={(v, ms) => {
                  setReply(v)
                  setMentions(ms)
                }}
                onPaste={(e) => void attach.onPaste(e)}
                voice="main"
                placeholder={`Reply to ${from}`}
                aria-label={`Your reply to ${from}`}
                autoSize
                className="max-h-48 min-w-0 flex-1"
                disabled={busy}
              />
              {attach.input}
              <AttachButton attach={attach} disabled={busy} />
            </div>
            <PendingFiles
              attach={attach}
              disabled={busy}
              label="Files with the reply"
              className="flex flex-wrap gap-2"
            />
          </form>
        )}
        {missed && <p className="text-xs text-destructive">{missed}</p>}
        {error && <p className="text-xs text-destructive">{error}</p>}
        {d && (held || forYou) && !missed && (
          <DialogFooter>
            {held ? (
              <>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => void act(() => talk.dismiss(id))}
                >
                  Dismiss
                </Button>
                <Button
                  disabled={busy}
                  onClick={() => void act(() => talk.release(id))}
                >
                  Release
                </Button>
              </>
            ) : (
              <>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => void act(() => talk.markRead(id))}
                >
                  Mark read
                </Button>
                {!d.message.from.key.startsWith("user:") && (
                  <Button
                    variant="outline"
                    disabled={busy}
                    title="Cut the sender's running turn: it reads this first and answers you before anything else"
                    onClick={stopAndRead}
                  >
                    Stop and read
                  </Button>
                )}
                <Button type="submit" form="message-reply" disabled={busy}>
                  {busy ? "Sending…" : "Reply"}
                </Button>
              </>
            )}
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  )
}
