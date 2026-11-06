import { useEffect, useState, type FormEvent } from "react"

import {
  AttachButton,
  PendingFiles,
  useFileAttach,
} from "@/components/attachments"
import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import {
  cards,
  displayName,
  useMentionCandidates,
  type AskRepo,
} from "@/lib/cards"
import type { BoardSummary } from "@/lib/boards"
import { plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { effortLabel, modelLabel, useAgents } from "@/lib/agents"
import { openSettings } from "@/components/settings/settings-menu"
import { ZapIcon } from "lucide-react"

// NewCardDialog is New card (card #116): the person makes a card straight
// from the header, no ask. Title is all it needs; goal, done-when and the
// repo say more. With nobody picked, Irori starts an agent for it where
// spawning is on, else hands it to one free agent (idle with Offer it
// cards on, card #453), as it does any unheld card; picking an agent hands it
// to that one, woken. Files go with it (card #124): the clip, a drop on
// the form, or a screenshot pasted in any field; they go on the card as
// its first comment and with the hand-out. onMade gets the new card's id.
// From a board's page (card #126) it goes on that board, where anyone on
// it can take it up. On your own board with spawning on (card #456,
// spawned agents §10, sa-5) nobody picked is "A fresh agent": Irori starts
// one for the card with the board's model and effort; "Any free agent"
// shows only when spawning is off. Agents Irori started for other cards
// aren't offered: one never takes a second card.
export function NewCardDialog({
  open,
  onClose,
  onMade,
  board,
  from,
  spawned,
}: {
  open: boolean
  onClose: () => void
  onMade: (id: string) => void
  // The shared board whose page it was made from; left out: your own.
  board?: BoardSummary
  // A Talk message it's made from (card #390): the form starts filled.
  from?: { title: string; objective: string }
  // Agents Irori started for a card, left out of the picker.
  spawned?: Set<string>
}) {
  const [title, setTitle] = useState("")
  const [objective, setObjective] = useState("")
  const [doneWhen, setDoneWhen] = useState("")
  const [repoKey, setRepoKey] = useState("")
  const [assignTo, setAssignTo] = useState("")
  const [repos, setRepos] = useState<AskRepo[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const attach = useFileAttach(busy)
  const agents = useMentionCandidates(open).filter(
    (c) => c.who.live && !c.board && !spawned?.has(c.who.key)
  )
  // Each space has its own switch, off by default (card #573). The other
  // members can be given the card: their Irori asks them first.
  const { settings: spawn } = useAgents(open, board?.boardId ?? "")
  const fresh = !!spawn?.on
  const members = board?.members.filter((m) => !m.me) ?? []
  const giving = assignTo.startsWith("person:")

  // A message's Make a card fills the form as it opens (once per message).
  const [filled, setFilled] = useState<typeof from>(undefined)
  if (open && from && filled !== from) {
    setFilled(from)
    setTitle(from.title)
    setObjective(from.objective)
  }

  // A member belongs to their space: a pick doesn't outlive the dialog or
  // follow it to another board (card #573).
  const boardId = board?.boardId ?? ""
  useEffect(() => {
    setAssignTo("")
  }, [open, boardId])

  useEffect(() => {
    if (!open) return
    let live = true
    void cards.askRepos().then((r) => live && setRepos(r))
    return () => {
      live = false
    }
  }, [open])

  const reset = () => {
    setTitle("")
    setObjective("")
    setDoneWhen("")
    setRepoKey("")
    setAssignTo("")
    setError(null)
    attach.clear()
  }

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (busy) return
    if (!title.trim()) {
      setError("Give the card a title.")
      return
    }
    setBusy(true)
    setError(null)
    try {
      const res = await cards.create(
        { title, objective, doneWhen, repoKey, assignTo, board: board?.boardId },
        attach.files
      )
      reset()
      onMade(res.task.id)
      onClose()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && !busy && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <form
          className="grid gap-3"
          onSubmit={(e) => void submit(e)}
          onPaste={(e) => void attach.onPaste(e)}
          {...attach.drop}
        >
          <DialogHeader>
            <DialogTitle>
              {board ? `New card on ${plainLine(board.name)}` : "New card"}
            </DialogTitle>
            <DialogDescription>
              {board
                ? `Goes on ${plainLine(board.tag)}#, visible to ${board.people.map(plainLine).join(", ")}.${giving ? "" : fresh ? " Irori starts your agent for it, unless you give it to someone." : " Nobody picked: it waits there for someone on the board to take it up."}`
                : fresh
                  ? "Work for an agent. Irori starts a fresh one for it, unless you give it to one of yours."
                  : "Work for an agent to pick up. Nobody picked: Irori hands it to an idle agent with Offer it cards on."}
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-1.5">
            <Label htmlFor="new-card-title">Title</Label>
            <Input
              id="new-card-title"
              value={title}
              autoFocus
              maxLength={200}
              placeholder="What needs doing"
              onChange={(e) => setTitle(e.target.value)}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="new-card-goal">Goal</Label>
            <Textarea
              id="new-card-goal"
              value={objective}
              autoSize
              className="max-h-48"
              placeholder="What and why (optional)"
              onChange={(e) => setObjective(e.target.value)}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="new-card-done">Done when</Label>
            <Input
              id="new-card-done"
              value={doneWhen}
              placeholder="How to tell it's done (optional)"
              onChange={(e) => setDoneWhen(e.target.value)}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <ChoiceSelect
              label="Repo"
              showLabel
              value={repoKey}
              onValueChange={setRepoKey}
              className="w-full"
              choices={[
                { value: "", label: "No repo" },
                ...repos.map((r) => ({ value: r.repoKey, label: r.label })),
              ]}
            />
            <ChoiceSelect
              label={board ? "Give to" : "Agent"}
              showLabel
              value={assignTo}
              onValueChange={setAssignTo}
              className="w-full"
              choices={[
                {
                  value: "",
                  label: fresh ? "A fresh agent" : board ? "Nobody yet" : "Any free agent",
                },
                ...members.map((m) => ({
                  value: `person:${m.personId}`,
                  label: plainLine(m.name) || "A member",
                })),
                ...agents.map((c) => ({
                  value: c.who.key,
                  label: c.repo
                    ? `${displayName(c.who)} · ${c.repo}`
                    : displayName(c.who),
                })),
              ]}
            />
          </div>
          {giving && (
            <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-[12.5px] text-muted-foreground">
              It's theirs to pick up. Their Irori asks them before it starts an
              agent on it.
            </p>
          )}
          {board && spawn && !spawn.on && assignTo === "" && (
            <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-[12.5px] text-muted-foreground">
              Irori won't start an agent for this card: your agents are off in
              this space.{" "}
              <Button
                type="button"
                variant="link"
                size="xs"
                className="h-auto px-0 align-baseline"
                onClick={() => openSettings("agents")}
              >
                Turn them on in Settings › Agents
              </Button>
            </p>
          )}
          {fresh && spawn && assignTo === "" && (
            <p className="flex items-start gap-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-[12.5px] text-muted-foreground">
              <ZapIcon className="mt-0.5 size-3.5 shrink-0 text-primary" aria-hidden />
              <span>
                A fresh agent starts for this card with{" "}
                <span className="text-ink">
                  {modelLabel(spawn.model, "")} · {effortLabel(spawn.effort, "").toLowerCase()}
                </span>{" "}
                and ends when it's done.{" "}
                <Button
                  type="button"
                  variant="link"
                  size="xs"
                  className="h-auto px-0 align-baseline"
                  onClick={() => openSettings("agents")}
                >
                  Change in Settings › Agents
                </Button>
              </span>
            </p>
          )}
          <PendingFiles
            attach={attach}
            disabled={busy}
            label="Files with the card"
            className="flex flex-wrap gap-2"
          />
          {error && <p className="text-xs text-destructive">{error}</p>}
          <DialogFooter>
            {attach.input}
            <span className="mr-auto">
              <AttachButton attach={attach} disabled={busy} />
            </span>
            <Button
              type="button"
              variant="ghost"
              disabled={busy}
              onClick={onClose}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? "Making it…" : "Make the card"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
