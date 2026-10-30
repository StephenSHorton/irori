import { useState, type FormEvent } from "react"

import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import {
  boards as actions,
  InviteState,
  inviteOpen,
  MaxBoardName,
  suggestTag,
  useBoards,
  useInvite,
  validTag,
  type BoardSummary,
  type InviteStatus,
} from "@/lib/boards"
import { copyText } from "@/lib/clipboard"
import { plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// BoardsDialog is Settings → Boards (design/v0.5-shared-boards.md §8): each
// board with its name, tag and one status dot; Rename, Invite (owners),
// Leave (every one of your devices) and Forget (this device only); New
// board; and Join with an invite. An open invite takes the dialog over
// until it ends (§4.1, B4).
export function BoardsDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { boards, error, reload } = useBoards()
  const { invite, setInvite } = useInvite(open)
  const showInvite = !!invite && invite.state !== ""
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="gap-3 sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Boards</DialogTitle>
          <DialogDescription>
            Each board is its own set of cards, apart from My devices.
            Invite someone to share a board with them.
          </DialogDescription>
        </DialogHeader>
        {showInvite ? (
          <InvitePanel
            s={invite}
            onClose={() => {
              setInvite(null)
              void reload()
            }}
          />
        ) : (
          <>
            {boards.length > 0 ? (
              <ul className="grid gap-2" aria-label="Your boards">
                {boards.map((b) => (
                  <BoardRow
                    key={b.boardId}
                    board={b}
                    onChanged={reload}
                    onInvite={setInvite}
                  />
                ))}
              </ul>
            ) : (
              <p className="text-[13px] text-muted-foreground">No boards yet.</p>
            )}
            {error && <p className="text-xs text-destructive">{error}</p>}
            <NewBoard taken={boards.map((b) => b.tag)} onMade={reload} />
            <JoinInvite onJoined={setInvite} />
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

function BoardRow({
  board: b,
  onChanged,
  onInvite,
}: {
  board: BoardSummary
  onChanged: () => Promise<void>
  onInvite: (s: InviteStatus) => void
}) {
  const [renaming, setRenaming] = useState(false)
  const [name, setName] = useState(b.name)
  const [error, setError] = useState<string | null>(null)
  const rename = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      await actions.rename(b.boardId, name)
      setRenaming(false)
      await onChanged()
    } catch (err) {
      setError(errorText(err))
    }
  }
  const cards = b.cards === 1 ? "1 card" : `${b.cards} cards`
  const devices = b.devices === 1 ? "1 device" : `${b.devices} devices`
  return (
    <li className="grid grid-cols-[8px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-1 text-[13px]">
      <span
        className={cn("size-2 rounded-full", b.ready ? "bg-success" : "bg-warning")}
        aria-hidden
        title={b.ready ? "Ready" : "Its cards are still arriving from your other device"}
      />
      {renaming ? (
        <form className="flex min-w-0 gap-1.5" onSubmit={rename}>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={MaxBoardName}
            aria-label="Board name"
            className="h-7 text-[13px]"
            autoFocus
          />
          <Button type="submit" size="sm" disabled={!name.trim()}>
            Save
          </Button>
        </form>
      ) : (
        <span className="min-w-0 truncate">
          <span className="text-ink">{plainLine(b.name) || "(unnamed)"}</span>{" "}
          <span className="font-mono text-xs text-muted-foreground">{plainLine(b.tag)}#</span>{" "}
          <small className="text-xs text-muted-foreground">
            {b.ready ? `${cards} · ${devices}` : "arriving from your other device"}
          </small>
          {b.tagClash && (
            <small className="block text-xs text-destructive">
              Another board is also {plainLine(b.tag)}#: its refs need the card's id.
            </small>
          )}
        </span>
      )}
      <span className="flex items-center gap-1.5">
        {renaming ? (
          <Button variant="ghost" size="sm" onClick={() => setRenaming(false)}>
            Cancel
          </Button>
        ) : (
          b.owner && (
            <>
              <Button variant="ghost" size="sm" onClick={() => setRenaming(true)}>
                Rename
              </Button>
              {b.ready && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={async () => {
                    setError(null)
                    try {
                      onInvite(await actions.invite(b.boardId))
                    } catch (err) {
                      setError(errorText(err))
                    }
                  }}
                >
                  Invite
                </Button>
              )}
            </>
          )
        )}
        <ConfirmButton
          label="Leave"
          variant="destructive"
          confirmLabel="Leave the board"
          question={
            <>
              Leave <b>{plainLine(b.name)}</b>? It goes from every one of your
              devices, with its cards. Coming back needs a new board.
            </>
          }
          onConfirm={async () => {
            await actions.leave(b.boardId)
            await onChanged()
          }}
        />
      </span>
      <span className="col-start-2 col-end-4 flex items-center gap-2 text-xs text-muted-foreground">
        <span className="min-w-0 flex-1">
          Visible to: {b.people.map(plainLine).join(", ")}
        </span>
        <ConfirmButton
          label="Forget here"
          confirmLabel="Forget on this device"
          question={
            <>
              Forget <b>{plainLine(b.name)}</b> on this device only? Its file
              goes from here; your other devices keep it.
            </>
          }
          onConfirm={async () => {
            await actions.forget(b.boardId)
            await onChanged()
          }}
        />
      </span>
      {error && <span className="col-start-2 col-end-4 text-xs text-destructive">{error}</span>}
    </li>
  )
}

// NewBoard makes a board: a name, and the short tag its cards' refs carry
// ("mlg#12"), suggested from the name until it is typed.
function NewBoard({ taken, onMade }: { taken: string[]; onMade: () => Promise<void> }) {
  const [name, setName] = useState("")
  const [tag, setTag] = useState("")
  const [typedTag, setTypedTag] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const t = (typedTag ? tag : suggestTag(name)).trim().toLowerCase()
  const clash = taken.includes(t)
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await actions.create(name, t)
      setName("")
      setTag("")
      setTypedTag(false)
      await onMade()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="grid gap-2 border-t border-border pt-3" onSubmit={submit}>
      <span className="text-[13px] font-semibold text-ink">New board</span>
      <div className="flex gap-1.5">
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          maxLength={MaxBoardName}
          placeholder="Name (MLG)"
          aria-label="New board's name"
          className="h-8 min-w-0 flex-1 text-[13px]"
        />
        <Input
          value={typedTag ? tag : t}
          onChange={(e) => {
            setTypedTag(true)
            setTag(e.target.value.toLowerCase())
          }}
          maxLength={16}
          placeholder="tag"
          aria-label="New board's tag, for refs like mlg#12"
          className="h-8 w-24 font-mono text-[13px]"
        />
        <Button type="submit" size="sm" className="h-8" disabled={busy || !name.trim() || !validTag(t) || clash}>
          Create
        </Button>
      </div>
      <p className={cn("text-xs", clash || (t && !validTag(t)) ? "text-warning" : "text-muted-foreground")}>
        {clash
          ? `Another board is tagged ${t} already.`
          : t && !validTag(t)
            ? "A tag is a lowercase letter, then letters, digits or '-' (16 at most)."
            : `Its cards are ${t || "tag"}#1, ${t || "tag"}#2, …`}
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}

// JoinInvite is Join with an invite: the irori-invite: link someone sent.
function JoinInvite({ onJoined }: { onJoined: (s: InviteStatus) => void }) {
  const [link, setLink] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      onJoined(await actions.joinInvite(link.trim()))
      setLink("")
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="grid gap-2 border-t border-border pt-3" onSubmit={submit}>
      <span className="text-[13px] font-semibold text-ink">Join with an invite</span>
      <div className="flex gap-1.5">
        <Input
          value={link}
          onChange={(e) => setLink(e.target.value)}
          placeholder="irori-invite:…"
          aria-label="The invite link someone sent you"
          className="h-8 min-w-0 flex-1 font-mono text-[13px]"
        />
        <Button type="submit" size="sm" className="h-8" disabled={busy || !link.trim()}>
          Join
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Paste the link, then get on a voice call with whoever sent it.
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}

// InvitePanel is the open invite, either side (§4.1, B4). The inviter
// sends the link, then types the five words the invitee reads aloud; both
// read out each other's fingerprint from their own screen.
function InvitePanel({ s, onClose }: { s: InviteStatus; onClose: () => void }) {
  const open = inviteOpen(s)
  const host = s.role === "host"
  const board = plainLine(s.boardName) || "the board"
  const peer = plainLine(s.peerName) || "them"
  return (
    <section className="grid gap-3 text-[13px]" aria-label="Invite">
      <p className="font-semibold text-ink">
        {host ? `Inviting someone to ${board}` : `Joining ${board}`}
      </p>
      {host && s.state === InviteState.Waiting && <SendLink s={s} />}
      {s.state === InviteState.Words && (
        <>
          <Fingerprints s={s} peer={peer} />
          {host ? <TypeInviteWords s={s} peer={peer} /> : <ReadInviteWords s={s} peer={peer} />}
        </>
      )}
      {s.state === InviteState.Joining && (
        <p className="text-muted-foreground">The words match. Joining…</p>
      )}
      {s.state === InviteState.Done && (
        <p className="text-ink">
          {host ? `${peer} joined ${board}.` : `You joined ${board}.`}
        </p>
      )}
      {s.state === InviteState.Failed && (
        <p className="text-destructive">{plainLine(s.error ?? "") || "The invite ended."}</p>
      )}
      <div className="flex justify-end">
        {open ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={async () => {
              await actions.cancelInvite()
              onClose()
            }}
          >
            Cancel invite
          </Button>
        ) : (
          <Button size="sm" onClick={onClose}>
            Close
          </Button>
        )}
      </div>
    </section>
  )
}

function SendLink({ s }: { s: InviteStatus }) {
  const [copied, setCopied] = useState(false)
  const until = s.expiresAt ? new Date(s.expiresAt) : null
  return (
    <div className="grid gap-2">
      <p className="text-muted-foreground">
        Send this link to the person you're inviting, then get on a voice
        call. It works once{until ? `, until ${until.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : ""}.
      </p>
      <div className="flex gap-1.5">
        <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-2 py-1.5 font-mono text-xs">
          {s.link}
        </code>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => void copyText(s.link ?? "").then(setCopied)}
        >
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">Waiting for them to join…</p>
    </div>
  )
}

function Fingerprints({ s, peer }: { s: InviteStatus; peer: string }) {
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-md bg-muted px-3 py-2 text-xs">
      <dt className="text-muted-foreground">{peer}</dt>
      <dd className="font-mono text-ink">{plainLine(s.peerFingerprint)}</dd>
      <dt className="text-muted-foreground">You</dt>
      <dd className="font-mono text-ink">{plainLine(s.myFingerprint)}</dd>
      <dd className="col-span-2 pt-1 text-muted-foreground">
        Read your fingerprint aloud, and check that {peer} reads theirs as shown here.
      </dd>
    </dl>
  )
}

// TypeInviteWords is the inviter: the words the invitee reads aloud. The
// invite goes on only when they match; two misses end it.
function TypeInviteWords({ s, peer }: { s: InviteStatus; peer: string }) {
  const [words, setWords] = useState("")
  const [error, setError] = useState<string | null>(null)
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!words.trim()) return
    setError(null)
    try {
      await actions.confirmInviteWords(words.trim())
      setWords("")
    } catch (err) {
      setError(errorText(err))
    }
  }
  return (
    <form className="grid gap-1.5" onSubmit={submit}>
      <p className="text-ink">Type the five words {peer} reads to you:</p>
      <Input
        value={words}
        onChange={(e) => setWords(e.target.value)}
        aria-label={`The words ${peer} reads out`}
        placeholder="Type the words, then Enter"
        className="h-8 font-mono text-[13px]"
        autoFocus
      />
      {s.misses > 0 && (
        <p className="text-xs text-warning">
          Those words didn't match. One more miss ends the invite.
        </p>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}

function ReadInviteWords({ s, peer }: { s: InviteStatus; peer: string }) {
  return (
    <div className="grid gap-1.5">
      <p className="text-ink">Read these words to {peer}:</p>
      <p className="rounded-md bg-muted px-3 py-2 font-mono text-[15px] text-ink">
        {(s.words ?? []).join(" ")}
      </p>
    </div>
  )
}
