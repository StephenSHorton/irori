import { useEffect, useState, type FormEvent } from "react"

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
  AutonomyPresets,
  ThreadsModes,
  boards as actions,
  InviteState,
  ReceivePresets,
  inviteOpen,
  MaxBoardName,
  suggestTag,
  validTag,
  MaxBoardTag,
  type BoardSettings,
  type InviteStatus,
} from "@/lib/boards"
import { copyText } from "@/lib/clipboard"
import { devices, type HomeServerView } from "@/lib/devices"
import { plainLine, shortRepo } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// The pieces of the Boards pages (card #96; design/v0.5-shared-boards.md
// §8): the New board dialog (make one, or join with an invite), an open invite either side, how an
// invite works, and a board's settings (members, what reaches my agents,
// what my agents may do, reviewers).

// useInviteReach says why an invite link from this device may not reach
// someone away from your home, or "" when it should (card #107). With
// "Meet through public relays" on (decision 43, docs/design/public-relays.md)
// the invite dials n0's public relays, so it reaches any network, CGNAT
// included, and there is nothing to warn about (issue 9). With it off, the
// link names this device's home server: off, or on but not reachable from
// outside, it only works for someone on your network.
export function useInviteReach(open: boolean): string {
  const [why, setWhy] = useState("")
  useEffect(() => {
    if (!open) return
    let live = true
    devices
      .homeServer()
      .then((h) => live && setWhy(inviteReachLine(h)))
      .catch(() => live && setWhy(""))
    return () => {
      live = false
    }
  }, [open])
  return why
}

export function inviteReachLine(h: HomeServerView): string {
  if (h.publicRelays) return ""
  if (!h.can || h.elsewhere) return ""
  if (!h.on) {
    return "Your home server is off, so the link only works for someone who can reach this device directly, like on your network. Turn it on in Settings, Devices to invite people from anywhere."
  }
  if (!h.running || h.reach === "outside") return ""
  const fix = !h.outside
    ? "Turn on Reachable from outside home in Settings, Devices."
    : plainLine(h.line ?? "") || "Irori is still asking your router to open a port; check Settings, Devices."
  return `Your home server can't be reached from outside your home yet, so the link only works for someone on your network. ${fix}`
}

// InviteHow says once, under the boards, what an invite does, and why the
// link may not reach far (card #96).
export function InviteHow({ reach }: { reach: string }) {
  return (
    <div className="grid gap-1 border-l-2 border-primary/60 pl-2.5 text-xs text-muted-foreground">
      <p>
        <span className="text-ink">How an invite works:</span> you invite a
        person, not a device. Send them the one-use link (good for an hour),
        then they give you the five words their screen shows to check it's
        them. Their other devices follow once they're in.
      </p>
      {reach && <p className="text-warning">{reach}</p>}
    </div>
  )
}

// NewBoard makes a board: a name, and the short tag its cards' refs carry
// ("tag#12"), suggested from the name until it is typed.
export function NewBoard({ taken, onMade }: { taken: string[]; onMade: () => Promise<void> }) {
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
    <form className="grid gap-2" onSubmit={submit}>
      <div className="flex gap-1.5">
        <Input
          value={name}
          autoFocus
          onChange={(e) => setName(e.target.value)}
          maxLength={MaxBoardName}
          placeholder="Board name"
          aria-label="New board's name"
          className="h-8 min-w-0 flex-1 text-[13px]"
        />
        <Input
          value={typedTag ? tag : t}
          onChange={(e) => {
            setTypedTag(true)
            setTag(e.target.value.toLowerCase())
          }}
          maxLength={MaxBoardTag}
          placeholder="tag"
          aria-label="New board's tag, the short name in card refs like tag#12"
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
            ? `A tag is a lowercase letter, then letters, digits or '-' (${MaxBoardTag} at most).`
            : `Its cards are ${t || "tag"}#1, ${t || "tag"}#2, …`}
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}

// NewBoardDialog is the header's New board on Boards (card #126): make a
// board, or join one with an invite someone sent, as two tabs. Joining
// closes it; the invite then runs over the Boards list. join is false
// while an invite is open: one at a time.
export function NewBoardDialog({
  open,
  onClose,
  taken,
  onMade,
  join,
  onJoined,
}: {
  open: boolean
  onClose: () => void
  taken: string[]
  onMade: () => Promise<void>
  join: boolean
  onJoined: (s: InviteStatus) => void
}) {
  const [tab, setTab] = useState<"make" | "join">("make")
  const t = join ? tab : "make"
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (o) return
        setTab("make")
        onClose()
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t === "make" ? "New board" : "Join a board"}</DialogTitle>
          <DialogDescription>
            {t === "make"
              ? "One board per project. Only the people you invite see it."
              : "Paste the invite link someone sent you."}
          </DialogDescription>
        </DialogHeader>
        {join && (
          <div className="flex gap-1" role="tablist" aria-label="New board or join one">
            {(
              [
                ["make", "Make a board"],
                ["join", "Join with an invite"],
              ] as const
            ).map(([k, label]) => (
              <Button
                key={k}
                type="button"
                size="sm"
                role="tab"
                aria-selected={t === k}
                variant={t === k ? "secondary" : "ghost"}
                onClick={() => setTab(k)}
              >
                {label}
              </Button>
            ))}
          </div>
        )}
        {t === "make" ? (
          <NewBoard
            taken={taken}
            onMade={async () => {
              await onMade()
              onClose()
            }}
          />
        ) : (
          <JoinInvite
            onJoined={(s) => {
              onJoined(s)
              setTab("make")
              onClose()
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

// JoinInvite is Join with an invite: the irori-invite: link someone sent.
export function JoinInvite({ onJoined }: { onJoined: (s: InviteStatus) => void }) {
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
    <form className="grid gap-2" onSubmit={submit}>
      <div className="flex gap-1.5">
        <Input
          value={link}
          autoFocus
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
        Paste the link, then give whoever sent it the five words you'll see.
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}

// InvitePanel is the open invite, either side (§4.1, B4). The inviter
// sends the link, then types the five words the invitee gives them; both
// check each other's fingerprint against their own screen.
export function InvitePanel({ s, onClose }: { s: InviteStatus; onClose: () => void }) {
  const open = inviteOpen(s)
  const host = s.role === "host"
  const board = plainLine(s.boardName) || "the board"
  const peer = plainLine(s.peerName) || "them"
  return (
    <section className="grid grid-cols-[minmax(0,1fr)] gap-3 text-[13px]" aria-label="Invite">
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
  const reach = useInviteReach(true)
  const until = s.expiresAt ? new Date(s.expiresAt) : null
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2">
      <p className="text-muted-foreground">
        Send this link to the person you're inviting. It works once{until ? `, until ${until.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : ""}.
      </p>
      <div className="flex gap-1.5">
        <code
          className="min-w-0 flex-1 truncate rounded-md bg-muted px-2 py-1.5 font-mono text-xs select-all"
          title={s.link}
        >
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
      {reach && <p className="text-xs text-warning">{reach}</p>}
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
        Give {peer} your fingerprint, and check that theirs matches what's shown here.
      </dd>
    </dl>
  )
}

// TypeInviteWords is the inviter: the words the invitee gives them. The
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
      <p className="text-ink">Type the five words {peer} gives you:</p>
      <Input
        value={words}
        onChange={(e) => setWords(e.target.value)}
        aria-label={`The words ${peer} gives you`}
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
      <p className="text-ink">Give these words to {peer}:</p>
      <p className="rounded-md bg-muted px-3 py-2 font-mono text-[15px] text-ink">
        {(s.words ?? []).join(" ")}
      </p>
    </div>
  )
}

// Reviewers is the board's peer-review setting (design §10). With nothing set,
// a review request from someone else that names nobody goes to any free agent
// of yours (open_reviewer, crates/coord/src/board/review.rs); a pinned agent
// here, for the whole board or one repo, only overrides that. The review is
// that agent's, under your GitHub account; Irori only routes the request.
function Reviewers({
  s,
  save,
}: {
  s: BoardSettings
  save: (f: () => Promise<BoardSettings>) => Promise<void>
}) {
  const [agent, setAgent] = useState("")
  const [repo, setRepo] = useState("")
  const repos = [...new Set(s.agents.map((a) => a.repo).filter(Boolean))]
  return (
    <section className="grid gap-1" aria-label="Reviews">
      <span className="font-semibold text-ink">Reviews</span>
      <p className="text-muted-foreground">
        Any free agent of yours reviews other people&apos;s PRs on this board, under your GitHub
        account. Nothing to set up.
      </p>
      <p className="text-muted-foreground">
        Optional: always use one agent, for the whole board or one repo.
      </p>
      {s.reviewers.length > 0 && (
        <ul className="grid gap-0.5" aria-label="Pinned reviewers">
          {s.reviewers.map((r) => (
            <li key={r.repo || "*"} className="flex items-center gap-1.5">
              <span className="text-ink">{plainLine(r.name) || r.agent}</span>
              <span className="text-muted-foreground">
                {r.repo ? `for ${shortRepo(r.repo)}` : "for the whole board"}
              </span>
              <Button
                size="xs"
                variant="ghost"
                className="ml-auto"
                onClick={() => void save(() => actions.setReviewer(s.boardId, "", r.repo))}
              >
                Remove
              </Button>
            </li>
          ))}
        </ul>
      )}
      {s.agents.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <select
            className="h-7 rounded-md border border-border bg-transparent px-1.5"
            aria-label="Agent to always use"
            value={agent}
            onChange={(e) => setAgent(e.target.value)}
          >
            <option value="">Pick an agent…</option>
            {s.agents.map((a) => (
              <option key={a.agent} value={a.agent}>
                {plainLine(a.name)}
                {a.repo ? ` (${shortRepo(a.repo)})` : ""}
              </option>
            ))}
          </select>
          <select
            className="h-7 rounded-md border border-border bg-transparent px-1.5"
            aria-label="For which repo"
            value={repo}
            onChange={(e) => setRepo(e.target.value)}
          >
            <option value="">the whole board</option>
            {repos.map((r) => (
              <option key={r} value={r}>
                {shortRepo(r)}
              </option>
            ))}
          </select>
          <Button
            size="sm"
            variant="secondary"
            disabled={!agent}
            onClick={() =>
              void save(async () => {
                const next = await actions.setReviewer(s.boardId, agent, repo)
                setAgent("")
                return next
              })
            }
          >
            Always use
          </Button>
        </div>
      ) : (
        <p className="text-muted-foreground">No agent of yours is running to pin.</p>
      )}
    </section>
  )
}

// BoardSettingsPanel is a board opened (§8): its members, what reaches your
// agents from other people on it (§7.2), and what your agents may do there
// (§7.3). Only the person changes these, here.
export function BoardSettingsPanel({
  boardId,
  onChanged,
}: {
  boardId: string
  onChanged: () => Promise<void>
}) {
  const [s, setS] = useState<BoardSettings | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    void actions
      .settings(boardId)
      .then((v) => live && setS(v))
      .catch((err) => live && setError(errorText(err)))
    return () => {
      live = false
    }
  }, [boardId])
  const save = async (f: () => Promise<BoardSettings>) => {
    setError(null)
    try {
      setS(await f())
    } catch (err) {
      setError(errorText(err))
    }
  }
  if (!s) {
    return error ? (
      <p className="text-xs text-destructive">{error}</p>
    ) : null
  }
  const others = s.members.filter((m) => !m.me)
  const amOwner = s.members.some((m) => m.me && m.owner)
  const owners = s.members.filter((m) => m.owner).length
  const receiveHint = ReceivePresets.find((p) => p.value === s.receive)?.hint
  const autonomyHint = AutonomyPresets.find((p) => p.value === s.autonomy)?.hint
  return (
    <div className="grid gap-3 text-xs">
      <section className="grid gap-1" aria-label="Members">
        <span className="font-semibold text-ink">Members</span>
        <ul className="grid gap-0.5">
          {s.members.map((m) => (
            <li key={m.personId} className="grid gap-0.5">
              <span className="flex items-center gap-1.5">
                <span className="text-ink">{plainLine(m.name) || "someone"}</span>
                {m.me && <span className="text-muted-foreground">(you)</span>}
                {m.owner && (
                  <span className="rounded bg-muted px-1 text-[11px] text-muted-foreground">owner</span>
                )}
                <span className="ml-auto text-muted-foreground">
                  {m.devices === 1 ? "1 device" : `${m.devices} devices`}
                </span>
              </span>
              <span className="flex items-center gap-1.5 text-muted-foreground">
                <span className="font-mono" title="Their person fingerprint">
                  {plainLine(m.fingerprint)}
                </span>
                {amOwner && (
                  <span className="ml-auto flex items-center gap-1">
                    {m.me ? (
                      // Your own row: stepping down is confirmed, and only
                      // offered while another owner stays (a board needs one).
                      owners > 1 && (
                        <ConfirmButton
                          label="Step down"
                          confirmLabel="Step down to member"
                          question={
                            <>
                              Step down to a member? You can no longer invite, remove people or change
                              roles here; another owner would have to make you an owner again.
                            </>
                          }
                          onConfirm={() => save(() => actions.setRole(boardId, m.personId, "member"))}
                        />
                      )
                    ) : (
                      // Owners are equals (card #142): another owner has
                      // neither Make a member nor Remove.
                      !m.owner && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => void save(() => actions.setRole(boardId, m.personId, "owner"))}
                        >
                          Make an owner
                        </Button>
                      )
                    )}
                    {!m.me && !m.owner && (
                      <ConfirmButton
                        label="Remove"
                        variant="destructive"
                        confirmLabel={`Remove ${plainLine(m.name) || "them"}`}
                        question={
                          <>
                            {plainLine(m.name) ? `${plainLine(m.name)} keeps` : "They keep"}{" "}
                            everything already on their devices. Removing them stops them syncing and writing
                            anything this board accepts.
                          </>
                        }
                        onConfirm={async () => {
                          await save(() => actions.removeMember(boardId, m.personId))
                          await onChanged()
                        }}
                      />
                    )}
                  </span>
                )}
              </span>
            </li>
          ))}
        </ul>
        {s.conflicts.length > 0 && (
          <ul className="grid gap-0.5 text-warning" aria-label="Membership changes that didn't take">
            {s.conflicts.map((c, i) => (
              <li key={i}>{plainLine(c)}</li>
            ))}
          </ul>
        )}
      </section>
      <section className="grid gap-1" aria-label="What reaches my agents">
        <span className="font-semibold text-ink">What reaches my agents</span>
        <div className="flex flex-wrap gap-1" role="radiogroup" aria-label="Receive preset">
          {ReceivePresets.map((p) => (
            <Button
              key={p.value}
              size="sm"
              variant={s.receive === p.value ? "secondary" : "ghost"}
              role="radio"
              aria-checked={s.receive === p.value}
              onClick={() =>
                void save(() =>
                  actions.setReceive(boardId, p.value, p.value === "members" ? s.people : [])
                )
              }
            >
              {p.label}
            </Button>
          ))}
        </div>
        <p className="text-muted-foreground">{receiveHint}</p>
        {s.receive === "members" &&
          (others.length > 0 ? (
            <ul className="grid gap-0.5" aria-label="People who wake your agents">
              {others.map((m) => {
                const on = s.people.includes(m.personId)
                return (
                  <li key={m.personId}>
                    <label className="flex items-center gap-1.5">
                      <input
                        type="checkbox"
                        checked={on}
                        onChange={() =>
                          void save(() =>
                            actions.setReceive(
                              boardId,
                              "members",
                              on ? s.people.filter((p) => p !== m.personId) : [...s.people, m.personId]
                            )
                          )
                        }
                      />
                      {plainLine(m.name) || "someone"}
                    </label>
                  </li>
                )
              })}
            </ul>
          ) : (
            <p className="text-muted-foreground">Nobody else is on this board yet.</p>
          ))}
      </section>
      <section className="grid gap-1" aria-label="What my agents may do">
        <span className="font-semibold text-ink">What my agents may do</span>
        <div className="flex flex-wrap gap-1" role="radiogroup" aria-label="Autonomy preset">
          {AutonomyPresets.map((p) => (
            <Button
              key={p.value}
              size="sm"
              variant={s.autonomy === p.value ? "secondary" : "ghost"}
              role="radio"
              aria-checked={s.autonomy === p.value}
              onClick={() => void save(() => actions.setAutonomy(boardId, p.value))}
            >
              {p.label}
            </Button>
          ))}
        </div>
        <p className="text-muted-foreground">{autonomyHint}</p>
      </section>
      <section className="grid gap-1" aria-label="My threads">
        <span className="font-semibold text-ink">My threads</span>
        <div className="flex flex-wrap gap-1" role="radiogroup" aria-label="Where my threads live">
          {ThreadsModes.map((m) => (
            <Button
              key={m.value}
              size="sm"
              variant={s.threads === m.value ? "secondary" : "ghost"}
              role="radio"
              aria-checked={s.threads === m.value}
              onClick={() => void save(() => actions.setThreads(boardId, m.value))}
            >
              {m.label}
            </Button>
          ))}
        </div>
        <p className="text-muted-foreground">
          {ThreadsModes.find((m) => m.value === s.threads)?.hint}
        </p>
      </section>
      <Reviewers s={s} save={save} />
      {error && <p className="text-destructive">{error}</p>}
    </div>
  )
}
