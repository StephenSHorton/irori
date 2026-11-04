// SpawnedActions is what the person can do to an agent Irori started for a
// card (card #456, docs/design/spawned-agents.md §10, mockup sa-2): Attach
// opens it in the card's terminal tab, Logs shows its output so far, Stop
// cuts its turn and End hands off and removes it. The agent hover, Who's
// on it and the "Waiting for your permission" row in Needs you share it.
// Attach, Logs and End work on the device the agent runs on; elsewhere the
// row says where.

import { useEffect, useState } from "react"
import {
  ScrollTextIcon,
  SquareIcon,
  SquareTerminalIcon,
  XIcon,
} from "lucide-react"

import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { agentLogs, attachAgent, endAgent } from "@/lib/agents"
import { displayName, type AgentRef } from "@/lib/cards"
import { inWails, plainLine } from "@/lib/irori"
import { errorText, talk } from "@/lib/talk"
import { cn } from "@/lib/utils"

// here says the agent runs on this device, where Attach, Logs and End work.
export function spawnedHere(who: AgentRef): boolean {
  return who.where === "This PC"
}

export function SpawnedActions({
  who,
  cardId,
  onOpenCard,
  only,
  className,
}: {
  who: AgentRef
  cardId: string
  // Attach from outside the card's page opens the card, where its
  // terminal tab is.
  onOpenCard?: (id: string) => void
  // A subset, in this order (Needs you shows Attach and Logs).
  only?: ("attach" | "logs" | "stop" | "end")[]
  className?: string
}) {
  const [line, setLine] = useState<{ ok: boolean; text: string } | null>(null)
  const [ended, setEnded] = useState(false)
  const [logsOpen, setLogsOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const show = (k: "attach" | "logs" | "stop" | "end") => !only || only.includes(k)
  const name = displayName(who)
  if (ended) return <p className="text-xs text-muted-foreground">Ending: it writes where it left off first.</p>
  if (!spawnedHere(who))
    return (
      <p className={cn("text-xs text-muted-foreground", className)}>
        Runs on {plainLine(who.where)}: Attach, Logs and End are there.
      </p>
    )
  const attach = async () => {
    setBusy(true)
    setLine(null)
    try {
      await attachAgent(who.key, cardId)
      onOpenCard?.(cardId)
      setLine({
        ok: true,
        text: inWails()
          ? `${name} is open in this card's terminal. Typing there talks to it; it stays spawned.`
          : `In the app this opens ${name} in this card's terminal tab.`,
      })
    } catch (e) {
      setLine({ ok: false, text: errorText(e) })
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className={cn("grid gap-1", className)}>
      <div className="flex flex-wrap items-center gap-1.5">
        {show("attach") && (
          <Button
            size="xs"
            variant="outline"
            disabled={busy}
            onClick={() => void attach()}
            title="Open it in this card's terminal tab. Typing there talks to it; it stays spawned."
          >
            <SquareTerminalIcon aria-hidden />
            Attach
          </Button>
        )}
        {show("logs") && (
          <Button
            size="xs"
            variant="outline"
            onClick={() => setLogsOpen(true)}
            title="Its output so far, read only"
          >
            <ScrollTextIcon aria-hidden />
            Logs
          </Button>
        )}
        {show("stop") && (
          <ConfirmButton
            label={
              <>
                <SquareIcon aria-hidden />
                Stop
              </>
            }
            question={`Stop ${name}'s turn? It stays on the card and waits for you.`}
            confirmLabel="Stop"
            onConfirm={async () => {
              await talk.stop(who.key, "")
              setLine({ ok: true, text: `${name} is stopped and waits for you.` })
            }}
          />
        )}
        {show("end") && (
          <ConfirmButton
            label={
              <>
                <XIcon aria-hidden />
                End
              </>
            }
            variant="destructive"
            question={
              <>
                End {name}? It writes where it left off on this card first,
                then Irori stops it and removes its session. The card stays
                open and unheld.
              </>
            }
            confirmLabel="End"
            onConfirm={async () => {
              await endAgent(who.key)
              setEnded(true)
            }}
          />
        )}
      </div>
      {line && (
        <p className={line.ok ? "text-xs text-muted-foreground" : "text-xs text-destructive"}>
          {line.text}
        </p>
      )}
      {logsOpen && (
        <LogsDialog who={who} onClose={() => setLogsOpen(false)} />
      )}
    </div>
  )
}

// LogsDialog is Logs: `claude logs` for the agent, read once on open, with
// Refresh.
function LogsDialog({ who, onClose }: { who: AgentRef; onClose: () => void }) {
  const [text, setText] = useState<string | null>(null)
  const [error, setError] = useState("")
  const [tick, setTick] = useState(0)
  useEffect(() => {
    let live = true
    agentLogs(who.key).then(
      (t) => live && (setText(t), setError("")),
      (e) => live && setError(errorText(e)),
    )
    return () => {
      live = false
    }
  }, [who.key, tick])
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Logs · {displayName(who)}</DialogTitle>
          <DialogDescription>
            Its terminal output so far, read only. Attach to talk to it.
          </DialogDescription>
        </DialogHeader>
        <pre className="max-h-[60vh] min-h-32 overflow-auto rounded-md border border-border bg-background/60 p-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink-2">
          {error ? (
            <span className="text-destructive">{error}</span>
          ) : text === null ? (
            "Loading…"
          ) : (
            text || "Nothing yet."
          )}
        </pre>
        <div className="flex justify-end">
          <Button size="sm" variant="outline" onClick={() => setTick((n) => n + 1)}>
            Refresh
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
