import { useState } from "react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { plainLine, shortRepo, type AgentCard } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import {
  board,
  BlockerKind,
  MaxReason,
  MaxTitle,
  type NewCard,
} from "@/lib/board"
import { fieldClass, textareaClass } from "./bits"

type Kind = "task" | "blocker"

// RaiseForm posts a card as the person: a task (open to pick up, shared, or
// assigned to an agent who hears at its next turn) or a blocker (a small
// task created blocked, decision 17). onDone gets the new card's id, or ""
// when the form was closed.
export function RaiseForm({
  agents,
  repos,
  defaultRepo,
  onDone,
}: {
  agents: AgentCard[]
  repos: string[]
  defaultRepo: string
  onDone: (id: string) => void
}) {
  const [kind, setKind] = useState<Kind>("task")
  const [title, setTitle] = useState("")
  const [body, setBody] = useState("")
  const [repo, setRepo] = useState(defaultRepo)
  const [forWho, setForWho] = useState("")
  const [link, setLink] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const trimmed = title.trim()
  const ready = trimmed !== "" && trimmed.length <= MaxTitle && !busy

  const choose = (k: Kind) => {
    setKind(k)
    setForWho(k === "task" ? "" : "person")
  }

  const submit = async () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    const blocker = kind === "blocker"
    const card: NewCard = {
      title: trimmed,
      objective: blocker ? "" : body.trim(),
      doneCriteria: "",
      repoKey: repo,
      shared: !blocker && forWho === "shared",
      links: link.trim() ? [link.trim()] : [],
      assignTo: !blocker && forWho.startsWith("agent:") ? forWho.slice(6) : "",
      blockReason: blocker ? body.trim() || trimmed : "",
      blockedOn: !blocker
        ? ("" as BlockerKind)
        : forWho === "person"
          ? BlockerKind.BlockedOnPerson
          : forWho.startsWith("agent:")
            ? BlockerKind.BlockedOnAgent
            : forWho === "external"
              ? BlockerKind.BlockedOnExternal
              : ("" as BlockerKind),
      blockedRef: blocker && forWho.startsWith("agent:") ? forWho.slice(6) : "",
    }
    try {
      const r = await board.create(card)
      onDone(r.task.id)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const agentName = (a: AgentCard) =>
    plainLine(a.name) || a.sessionId.slice(0, 8)
  const target = forWho.startsWith("agent:")
    ? agents.find((a) => a.key === forWho.slice(6))
    : undefined
  const hint =
    kind === "task"
      ? target
        ? `${agentName(target)} hears it at its next turn (no wake).`
        : forWho === "shared"
          ? "Shared: any number of agents can join and work it."
          : "Shows in Open to pick up; any agent can claim it."
      : forWho === "person"
        ? "Shows in Needs you."
        : target
          ? `Shows in Blocked, waiting on ${agentName(target)}.`
          : "Shows in Blocked."

  return (
    <form
      className="flex flex-col gap-2 border-b bg-muted/50 px-3 py-3"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      <div
        role="group"
        aria-label="Kind of card"
        className="inline-flex self-start rounded-lg bg-muted p-[3px]"
      >
        {(["task", "blocker"] as const).map((k) => (
          <button
            key={k}
            type="button"
            aria-pressed={kind === k}
            onClick={() => choose(k)}
            className="flex h-6 items-center rounded-md px-2.5 text-[12.5px] font-medium text-muted-foreground outline-none hover:text-ink focus-visible:ring-3 focus-visible:ring-ring/50 aria-pressed:bg-background aria-pressed:text-ink aria-pressed:shadow-sm"
          >
            {k === "task" ? "Task" : "Blocker"}
          </button>
        ))}
      </div>
      <input
        autoFocus
        aria-label="Title"
        value={title}
        maxLength={MaxTitle}
        placeholder={kind === "task" ? "What needs doing" : "What's in the way"}
        onChange={(e) => setTitle(e.target.value)}
        className={cn(fieldClass, "w-full")}
      />
      <textarea
        aria-label="Details"
        value={body}
        rows={2}
        maxLength={kind === "blocker" ? MaxReason : undefined}
        placeholder={
          kind === "task"
            ? "Objective (optional). Keep it short: plans and specs go in the project's tracker; link them."
            : "Why it's in the way (optional)"
        }
        onChange={(e) => setBody(e.target.value)}
        className={textareaClass}
      />
      <div className="flex flex-wrap gap-2">
        <select
          aria-label="Repo"
          value={repo}
          onChange={(e) => setRepo(e.target.value)}
          className={cn(fieldClass, "flex-1 basis-40")}
        >
          <option value="">No repo</option>
          {repos.map((r) => (
            <option key={r} value={r}>
              {shortRepo(r)}
            </option>
          ))}
        </select>
        <select
          aria-label={kind === "task" ? "For" : "Waiting on"}
          value={forWho}
          onChange={(e) => setForWho(e.target.value)}
          className={cn(fieldClass, "flex-1 basis-40")}
        >
          {kind === "task" ? (
            <>
              <option value="">Anyone (open to pick up)</option>
              <option value="shared">Anyone, shared</option>
              {agents.map((a) => (
                <option key={a.key} value={`agent:${a.key}`}>
                  Assign to {agentName(a)}
                </option>
              ))}
            </>
          ) : (
            <>
              <option value="person">Waiting on: you</option>
              {agents.map((a) => (
                <option key={a.key} value={`agent:${a.key}`}>
                  Waiting on: {agentName(a)}
                </option>
              ))}
              <option value="external">
                Waiting on: something outside Irori
              </option>
            </>
          )}
        </select>
      </div>
      <input
        aria-label="Link"
        value={link}
        placeholder="Link (optional): a Linear, GitHub or Notion URL, a repo path"
        onChange={(e) => setLink(e.target.value)}
        className={cn(fieldClass, "w-full")}
      />
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs text-muted-foreground">{hint}</span>
        {error && <span className="text-xs text-destructive">{error}</span>}
        <span className="ml-auto flex gap-1.5">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => onDone("")}
          >
            Cancel
          </Button>
          <Button type="submit" size="sm" disabled={!ready}>
            {busy ? "Posting…" : "Post"}
          </Button>
        </span>
      </div>
    </form>
  )
}
