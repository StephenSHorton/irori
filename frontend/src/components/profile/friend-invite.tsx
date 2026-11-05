// FriendInvite is the profile's "+ Invite a friend" and "Got a friend
// link?" (card #517, docs/design/talk-channels.md §10.1): one link and
// five words, tied to no board. Once you're friends, New board's "Who's on
// it?" lists them, and boards you make with them reach them with no
// invite. The open invite shows here, either side.

import { useState } from "react"

import { InvitePanel, JoinInvite } from "@/components/boards/parts"
import { boards as actions, inviteOpen, useInvite } from "@/lib/boards"
import { errorText } from "@/lib/talk"

export function FriendInvite() {
  const { invite, setInvite } = useInvite(true)
  const [paste, setPaste] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const open = invite !== null && inviteOpen(invite)
  const start = async () => {
    setBusy(true)
    setError("")
    try {
      setInvite(await actions.friendInvite())
      setPaste(false)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  // An invite's ended event can land after Close: "" state is none.
  if (invite && invite.state !== "") {
    return (
      <div className="rounded-md border border-border/60 p-3">
        <InvitePanel s={invite} onClose={() => setInvite(null)} />
      </div>
    )
  }
  return (
    <div className="grid gap-2 px-1 pt-1">
      <div className="flex flex-wrap gap-x-3 gap-y-1">
        <button
          type="button"
          disabled={busy || open}
          onClick={() => void start()}
          className="text-xs text-primary hover:underline disabled:opacity-50"
        >
          + Invite a friend
        </button>
        <button
          type="button"
          onClick={() => setPaste((p) => !p)}
          className="text-xs text-muted-foreground hover:text-ink hover:underline"
          aria-expanded={paste}
        >
          Got a friend link?
        </button>
      </div>
      {paste && (
        <JoinInvite
          onJoined={(s) => {
            setInvite(s)
            setPaste(false)
          }}
        />
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}
