// PermissionPrompt is "Waiting for your permission" (cards #456, #682,
// docs/design/spawned-agents.md §7, mockup sa-7): the agent Irori started
// for a card stopped on a permission prompt nobody is attached to. It
// shows what is waiting (the tool, and the command, path or URL) and
// buttons instead of an answer box. Typed words don't reach a prompt.
//
// - Allow once / Deny answer the live prompt: the pick goes to the device
//   that runs the agent, where the agent's permission hook answers it.
// - Always allow does that and also adds the rule to the board's allow
//   list (Settings › Agents). It is offered only when a rule helps: a
//   bypass agent only stops on Claude Code's safety checks, which no rule
//   skips.
// - Attach answers at the prompt itself. It is the only way when Irori
//   can't answer from the card (an agent started before the hook, or an
//   ask Claude Code keeps for the terminal).

import { useState } from "react"

import { Button } from "@/components/ui/button"
import { SpawnedActions } from "@/components/cards/spawned-actions"
import {
  cards as cardsApi,
  cardStore,
  displayName,
  type AgentRef,
  type CardBlocker,
} from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { answerKey, answerPatch, instant } from "@/lib/optimistic"

export const ALLOW_ONCE = "Allow once"
export const ALWAYS_ALLOW = "Always allow"
export const DENY = "Deny"

// clock is an RFC 3339 time as the person's clock reads it, e.g. "6:47 PM".
function clock(at: string): string {
  const d = new Date(at)
  return Number.isNaN(d.getTime())
    ? "2 minutes after asking"
    : d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
}

// The order the buttons come in, whatever order the item's options are.
const ORDER = [ALLOW_ONCE, ALWAYS_ALLOW, DENY]

export function PermissionPrompt({
  cardId,
  b,
  who,
  onOpenCard,
  compact = false,
}: {
  cardId: string
  b: CardBlocker
  // The agent on the prompt, as the card lists it, for Attach and Logs;
  // its permission's agent when the card doesn't list it.
  who?: AgentRef
  onOpenCard?: (id: string) => void
  compact?: boolean
}) {
  const [said, setSaid] = useState("")
  const p = b.permission
  if (!p) return null
  const agent = who ?? (p.agent.key ? p.agent : undefined)
  const name = agent ? displayName(agent) : "Its agent"
  const open = b.state !== "resolved" && b.state !== "closed"
  const tool = plainLine(p.tool)
  const answers = ORDER.map(
    (a) => [a, b.options.findIndex((o) => o.answer === a)] as const
  ).filter(([a, i]) => i >= 0 && (a !== ALWAYS_ALLOW || !!p.rule))
  const send = (option: number, answer: string) => {
    const started = instant({
      key: answerKey(cardId, b.id),
      patch: answerPatch(
        cardId,
        b,
        option,
        cardStore.slot(cardId)?.detail?.openBlockers
      ),
      send: () => cardsApi.answer(cardId, b.id, option, ""),
      failed: "Your answer to the permission prompt wasn't saved",
      onFailed: () => setSaid(""),
    })
    if (!started) return
    setSaid(
      answer === DENY
        ? `Denied: ${name} goes on without it.`
        : answer === ALWAYS_ALLOW
          ? `Allowed, and ${plainLine(p.rule)} is on the board's allow list.`
          : `Allowed once: ${name} goes on.`
    )
  }
  return (
    <div className="grid min-w-0 gap-1.5" data-permission-prompt>
      <p className="text-sm break-words">
        {open ? (
          <>
            {name} is stopped on a permission prompt
            {tool ? (
              <>
                {" "}
                to use <span className="font-medium">{tool}</span>
              </>
            ) : null}
            {p.input ? ":" : "."}
          </>
        ) : (
          <>
            {name} asked to use {tool || "a tool"}
            {p.input ? ":" : "."}
          </>
        )}
      </p>
      {p.input && (
        <pre
          className={
            "max-w-full overflow-auto rounded-md border bg-muted/50 px-2 py-1.5 font-mono text-xs break-all whitespace-pre-wrap " +
            (compact ? "max-h-24" : "max-h-48")
          }
          aria-label={tool ? `The ${tool} input waiting` : "What is waiting"}
        >
          {p.input}
        </pre>
      )}
      {open && p.answerable && p.expires && (
        <p className="text-[13px] break-words text-warning">
          A safety check Claude Code asks even in bypass mode: it denies the
          call by itself at {clock(p.expires)} unless you answer.
        </p>
      )}
      {open && !p.answerable && (
        <p className="text-[13px] break-words text-muted-foreground">
          Irori can't answer this prompt from the card: attach to {name} to
          answer it.
        </p>
      )}
      {said ? (
        <p className="text-[13px] break-words text-muted-foreground">{said}</p>
      ) : (
        open && (
          <div
            className="flex min-w-0 flex-wrap items-start gap-1.5"
            role="group"
            aria-label={`Answers to ${name}'s permission prompt`}
          >
            {p.answerable &&
              answers.map(([a, i]) => (
                <Button
                  key={a}
                  size="sm"
                  variant={
                    a === ALLOW_ONCE
                      ? "default"
                      : a === DENY
                        ? "destructive"
                        : "outline"
                  }
                  title={
                    a === ALWAYS_ALLOW
                      ? `Also adds ${p.rule} to the board's allow list`
                      : b.options[i]?.why
                        ? plainLine(b.options[i].why)
                        : undefined
                  }
                  onClick={() => send(i, a)}
                >
                  {a}
                </Button>
              ))}
            {agent && (
              <SpawnedActions
                who={agent}
                cardId={cardId}
                onOpenCard={onOpenCard}
                only={["attach", "logs"]}
              />
            )}
          </div>
        )
      )}
      {open &&
        !said &&
        p.answerable &&
        !!p.rule &&
        answers.some(([a]) => a === ALWAYS_ALLOW) && (
          <p className="text-xs break-words text-muted-foreground">
            Always allow also adds{" "}
            <code className="font-mono break-all">{plainLine(p.rule)}</code> to
            the board's allow list (Settings › Agents).
          </p>
        )}
    </div>
  )
}
