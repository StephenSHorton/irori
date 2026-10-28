import { useState, type FormEvent, type ReactNode } from "react"

import { ChoiceSelect } from "@/components/common/choice-select"
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
  devices as actions,
  isOpen,
  PairState,
  stateDot,
  useDevices,
  usePairing,
  type DeviceView,
  type PairStatus,
} from "@/lib/devices"
import { parseTime, plainLine, relativeTime, useNow } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

const roleChoices = [
  { value: "desktop", label: "Desktop" },
  { value: "node", label: "Always-on node" },
]

// DevicesDialog is Settings → Devices (docs/design/v0.4-devices.md §3, §9):
// each device, this one first, with one status dot, when it was last seen,
// at most one warning line and Remove; then Pair a device (the code, then
// the words the new device shows, typed here) and Join with a code (the
// words to type on the other device). Fingerprints, paths and counters
// stay in GET /v1/devices?detail=1.
export function DevicesDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { devices, error, reload } = useDevices(open)
  const { status, setStatus } = usePairing(open)
  const now = useNow(5000)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="gap-3 sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Devices</DialogTitle>
          <DialogDescription>
            Your devices share one inbox and one Board.
          </DialogDescription>
        </DialogHeader>
        <ul className="grid gap-1.5" aria-label="Your devices">
          {devices.map((d) => (
            <DeviceRow
              key={d.deviceId}
              device={d}
              now={now}
              onRemoved={reload}
            />
          ))}
        </ul>
        {error && <p className="text-xs text-destructive">{error}</p>}
        <div className="border-t border-border pt-3">
          {status?.state ? (
            <Pairing status={status} onDone={() => setStatus(null)} />
          ) : (
            <Start onStarted={setStatus} />
          )}
        </div>
        <p className="text-xs text-muted-foreground">
          Devices find each other through sealed messages on public MQTT
          brokers, which see IP addresses and when messages pass, never their
          content; STUN servers see IP addresses. Everything else goes encrypted
          between your own devices. What reaches another device stays there, so
          a secret an agent wrote that the scrubber missed is copied for good.
        </p>
      </DialogContent>
    </Dialog>
  )
}

function DeviceRow({
  device: d,
  now,
  onRemoved,
}: {
  device: DeviceView
  now: Date
  onRemoved: () => Promise<void>
}) {
  const seen = parseTime(d.lastSeen)
  const state = d.this
    ? "this device"
    : d.revoked
      ? "removed"
      : d.state === "connected"
        ? "online"
        : seen
          ? `offline · seen ${relativeTime(seen, now)}`
          : "offline · never seen"
  return (
    <li className="grid grid-cols-[8px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 text-[13px]">
      <span
        className={cn(
          "size-2 rounded-full",
          stateDot(d.revoked ? "offline" : d.state)
        )}
        aria-hidden
      />
      <span className="min-w-0 truncate">
        <span className={cn("text-ink", d.revoked && "line-through")}>
          {plainLine(d.name) || d.deviceId}
        </span>{" "}
        <small className="text-xs text-muted-foreground">
          {d.role === "node" ? "node · " : ""}
          {state}
        </small>
      </span>
      {d.this || d.revoked ? (
        <span />
      ) : (
        <ConfirmButton
          label="Remove"
          variant="destructive"
          confirmLabel="Remove device"
          question={
            <>
              Remove <b>{plainLine(d.name) || "this device"}</b>? It keeps
              everything it already has. Removing it stops it from syncing and
              from writing anything new that your other devices will accept, and
              gives your remaining devices a new person key.
            </>
          }
          onConfirm={async () => {
            await actions.remove(d.deviceId)
            await onRemoved()
          }}
        />
      )}
      {d.warning && (
        <span className="col-start-2 col-end-4 text-xs text-warning">
          {plainLine(d.warning)}
        </span>
      )}
    </li>
  )
}

// Start offers Pair a device and Join with a code.
function Start({ onStarted }: { onStarted: (s: PairStatus) => void }) {
  const [role, setRole] = useState("desktop")
  const [joining, setJoining] = useState(false)
  const [code, setCode] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async (fn: () => Promise<PairStatus>) => {
    setBusy(true)
    setError(null)
    try {
      onStarted(await fn())
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const submitCode = (e: FormEvent) => {
    e.preventDefault()
    if (code.trim()) void run(() => actions.join(code.trim()))
  }

  return (
    <div className="grid gap-2">
      {joining ? (
        <form className="flex gap-2" onSubmit={submitCode}>
          <Input
            autoFocus
            aria-label="Pairing code"
            placeholder="XXXX-XXXX-XXXX-XXXX"
            value={code}
            spellCheck={false}
            autoComplete="off"
            className="font-mono uppercase"
            onChange={(e) => setCode(e.target.value)}
          />
          <Button type="submit" size="sm" disabled={busy || !code.trim()}>
            Join
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => setJoining(false)}
          >
            Back
          </Button>
        </form>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            disabled={busy}
            onClick={() => void run(() => actions.add(role))}
          >
            Pair a device
          </Button>
          <ChoiceSelect
            label="The new device's role"
            size="sm"
            value={role}
            choices={roleChoices}
            onValueChange={setRole}
            className="text-[13px]"
          />
          <Button
            size="sm"
            variant="outline"
            className="ml-auto"
            onClick={() => setJoining(true)}
          >
            Join with a code
          </Button>
        </div>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}

// Pairing is the open (or just ended) pairing, either side.
function Pairing({
  status: s,
  onDone,
}: {
  status: PairStatus
  onDone: () => void
}) {
  const cancel = (
    <Button size="sm" variant="ghost" onClick={() => void actions.cancel()}>
      Cancel
    </Button>
  )
  if (s.state === PairState.Done)
    return <Ended text="Paired. The devices are syncing now." onDone={onDone} />
  if (!isOpen(s))
    return (
      <Ended
        text={
          s.error ||
          (s.state === PairState.Expired
            ? "Nobody used the code in time. Make a new one."
            : "Pairing canceled.")
        }
        onDone={onDone}
        failed={s.state !== PairState.Canceled}
      />
    )
  if (s.mode === "add" && s.state === PairState.Waiting)
    return <ShowCode status={s} cancel={cancel} />
  if (s.mode === "add" && s.state === PairState.Confirm)
    return <TypeWords status={s} cancel={cancel} />
  if (s.state === PairState.Words) {
    const who = plainLine(s.peer ?? "") || "the other device"
    return (
      <div className="grid gap-2">
        <p className="text-[13px] text-ink">Type these words on {who}:</p>
        <p className="rounded-lg bg-muted px-3 py-2 font-mono text-base tracking-wide text-ink select-all">
          {(s.words ?? []).join(" ")}
        </p>
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground">
            Waiting for {who}…
          </span>
          {cancel}
        </div>
      </div>
    )
  }
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-[13px] text-muted-foreground">
        Looking for the other device…
      </span>
      {cancel}
    </div>
  )
}

// ShowCode is step 1 on the device already in your mesh: the code to type
// on the new one.
function ShowCode({
  status: s,
  cancel,
}: {
  status: PairStatus
  cancel: ReactNode
}) {
  const [copied, setCopied] = useState(false)
  const now = useNow(15000)
  const expires = parseTime(s.expires)
  const left = expires
    ? Math.max(0, Math.ceil((expires.getTime() - now.getTime()) / 60_000))
    : 0
  return (
    <div className="grid gap-2">
      <p className="text-[13px] text-ink">
        On the new device, open Settings → Devices → Join with a code and type:
      </p>
      <p className="rounded-lg bg-muted px-3 py-2 text-center font-mono text-lg tracking-widest text-ink select-all">
        {s.code}
      </p>
      <p className="text-xs text-muted-foreground">
        Treat it like a one-use password: whoever has it can reach this device
        first.{expires ? ` It works for ${left} more min.` : ""}
      </p>
      {copied && (
        <p className="text-xs text-warning">
          Copied. Any agent on this machine can read the clipboard.
        </p>
      )}
      <div className="flex items-center justify-between gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            void navigator.clipboard
              ?.writeText(s.code ?? "")
              .then(() => setCopied(true))
          }
        >
          Copy
        </Button>
        {cancel}
      </div>
    </div>
  )
}

// TypeWords is step 4 on the device already in your mesh: who wants to
// join, and the box for the words it shows. There is no Confirm button:
// pairing goes on only when the typed words match.
function TypeWords({
  status: s,
  cancel,
}: {
  status: PairStatus
  cancel: ReactNode
}) {
  const [words, setWords] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const j = s.joiner

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!words.trim()) return
    setBusy(true)
    setError(null)
    try {
      await actions.confirm(words.trim())
    } catch (err) {
      setError(errorText(err))
      setWords("")
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="grid gap-2" onSubmit={(e) => void submit(e)}>
      <p className="text-[13px] text-ink">
        <b>{plainLine(j?.name ?? "") || "A device"}</b>
        {j?.os ? ` (${plainLine(j.os)})` : ""} wants to join.
        {j?.fingerprint && (
          <small className="ml-1 font-mono text-xs text-muted-foreground">
            key {plainLine(j.fingerprint)}
          </small>
        )}
      </p>
      <Input
        autoFocus
        aria-label="The words the new device shows"
        placeholder="Type the words it shows, then Enter"
        value={words}
        disabled={busy}
        spellCheck={false}
        autoComplete="off"
        aria-invalid={error ? true : undefined}
        onChange={(e) => setWords(e.target.value)}
      />
      {error && <p className="text-xs text-destructive">{error}</p>}
      <div className="flex justify-end">{cancel}</div>
    </form>
  )
}

function Ended({
  text,
  failed,
  onDone,
}: {
  text: string
  failed?: boolean
  onDone: () => void
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span
        className={cn(
          "text-[13px]",
          failed ? "text-destructive" : "text-muted-foreground"
        )}
      >
        {plainLine(text)}
      </span>
      <Button size="sm" variant="outline" onClick={onDone}>
        OK
      </Button>
    </div>
  )
}
