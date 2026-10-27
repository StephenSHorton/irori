import { useState } from "react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
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
      <ToggleGroup
        aria-label="Kind of card"
        size="sm"
        value={[kind]}
        onValueChange={([k]) => k && choose(k)}
        className="self-start"
      >
        {(["task", "blocker"] as const).map((k) => (
          <ToggleGroupItem
            key={k}
            value={k}
            className="px-2.5 text-[12.5px] dark:data-pressed:border-transparent dark:data-pressed:bg-background"
          >
            {k === "task" ? "Task" : "Blocker"}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      <input
        autoFocus
        aria-label="Title"
        value={title}
        maxLength={MaxTitle}
        placeholder={kind === "task" ? "What needs doing" : "What's in the way"}
        onChange={(e) => setTitle(e.target.value)}
        className={cn(fieldClass, "w-full")}
      />
      <Textarea
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
        <ChoiceSelect
          label="Repo"
          value={repo}
          onValueChange={setRepo}
          choices={[
            { value: "", label: "No repo" },
            ...repos.map((r) => ({ value: r, label: shortRepo(r) })),
          ]}
          className="flex-1 basis-40"
        />
        <ChoiceSelect
          label={kind === "task" ? "For" : "Waiting on"}
          value={forWho}
          onValueChange={setForWho}
          choices={
            kind === "task"
              ? [
                  { value: "", label: "Anyone (open to pick up)" },
                  { value: "shared", label: "Anyone, shared" },
                  ...agents.map((a) => ({
                    value: `agent:${a.key}`,
                    label: `Assign to ${agentName(a)}`,
                  })),
                ]
              : [
                  { value: "person", label: "Waiting on: you" },
                  ...agents.map((a) => ({
                    value: `agent:${a.key}`,
                    label: `Waiting on: ${agentName(a)}`,
                  })),
                  {
                    value: "external",
                    label: "Waiting on: something outside Irori",
                  },
                ]
          }
          className="flex-1 basis-40"
        />
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
