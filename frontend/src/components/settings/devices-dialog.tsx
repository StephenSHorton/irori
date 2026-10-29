import { useEffect, useState, type FormEvent, type ReactNode } from "react"

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
import { Switch } from "@/components/ui/switch"
import {
  DeviceColors,
  devices as actions,
  isOpen,
  PairState,
  stateDot,
  useDevices,
  usePairing,
  type DeviceView,
  type HomeServerView,
  type PairStatus,
  type RelayView,
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
// the words the new device shows, typed here, then this device's key typed
// there) and Join with a code (the words to type on the other device, and
// its key typed here; from outside the home, the server's address too).
// The Home server section sits above them (§7.1). Paths and counters stay
// in GET /v1/devices?detail=1.
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
        <HomeServer open={open} />
        <div className="border-t border-border pt-3">
          {status?.state ? (
            <Pairing status={status} onDone={() => setStatus(null)} />
          ) : (
            <Start onStarted={setStatus} />
          )}
        </div>
        <p className="text-xs text-muted-foreground">
          Devices find each other on your local network and through your home
          server, one of your own devices; nothing goes through a public server.
          Others on a network see that sealed messages pass, never their
          content, and a relay you add under Advanced sees IP addresses. What
          reaches another device stays there, so a secret an agent wrote that
          the scrubber missed is copied for good.
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
        : d.state === "recent" && seen
          ? `seen ${relativeTime(seen, now)}`
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
      {d.revoked ? (
        <span />
      ) : (
        <span className="flex items-center gap-2">
          <ColorPick device={d} onPicked={onRemoved} />
          {!d.this && (
            <ConfirmButton
              label="Remove"
              variant="destructive"
              confirmLabel="Remove device"
              question={
                <>
                  Remove <b>{plainLine(d.name) || "this device"}</b>? It keeps
                  everything it already has. Removing it stops it from syncing
                  and from writing anything new that your other devices will
                  accept, and gives your remaining devices a new person key.
                </>
              }
              onConfirm={async () => {
                await actions.remove(d.deviceId)
                await onRemoved()
              }}
            />
          )}
        </span>
      )}
      {d.warning && (
        <span className="col-start-2 col-end-4 flex items-center gap-2 text-xs text-warning">
          <span className="min-w-0">{plainLine(d.warning)}</span>
          {d.this && d.action === "reset" && (
            <ConfirmButton
              label="Reset"
              variant="destructive"
              confirmLabel="Reset and quit"
              question={
                <>
                  Reset this device? Its event log, device id and keys move
                  aside (nothing is deleted) and only the settings stay. Irori
                  quits; open it again to pair this device as a new one.
                </>
              }
              onConfirm={() => actions.reset()}
            />
          )}
        </span>
      )}
    </li>
  )
}

// HomeServer is the Home server section (§7.1, §9). On this device: the
// switch, "Reachable from outside home", a word on reach only when it
// falls short, and one line only when something is wrong (with the public
// address field when mapping failed). On every device: a line when this
// one can't be found on its network, and one when your home server can't
// be reached. Advanced holds the relay you run or rent elsewhere.
function HomeServer({ open }: { open: boolean }) {
  const [hs, setHs] = useState<HomeServerView | null>(null)
  const [publicAddr, setPublicAddr] = useState("")
  const [advanced, setAdvanced] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    let live = true
    const load = () =>
      void actions
        .homeServer()
        .then((v) => live && setHs(v))
        .catch(() => {})
    load()
    const id = window.setInterval(load, 5000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [open])

  const set = async (on: boolean, outside: boolean, pub: string) => {
    setError(null)
    try {
      setHs(await actions.setHomeServer(on, outside, pub))
    } catch (err) {
      setError(errorText(err))
    }
  }

  if (!hs) return null
  const reach =
    hs.reach === "forward"
      ? "reachable at home; outside needs a port forward"
      : hs.reach === "cgnat"
        ? "reachable at home only"
        : hs.on && !hs.running
          ? "starts once you have another device"
          : ""
  const askPublic =
    hs.on && hs.outside && (hs.reach === "forward" || !!hs.public)
  return (
    <div className="grid gap-1.5 text-[13px]">
      {hs.can && (hs.on || hs.suggest || !hs.elsewhere) && (
        <>
          <div className="flex items-center justify-between gap-2">
            <label htmlFor="home-server" className="min-w-0 cursor-pointer">
              <span className="text-ink">Home server: run it on this PC</span>
              {reach && (
                <small className="block text-xs text-muted-foreground">
                  {reach}
                </small>
              )}
            </label>
            <Switch
              id="home-server"
              checked={hs.on}
              onCheckedChange={(v) => void set(v, hs.outside, hs.public ?? "")}
            />
          </div>
          {!hs.on && (
            <p className="text-xs text-muted-foreground">
              {hs.suggest &&
                "This is your always-on device: run the server here so your devices reach each other from anywhere. "}
              Windows asks once to allow Irori on private networks; the server
              needs it.
            </p>
          )}
          {hs.on && (
            <div className="flex items-center justify-between gap-2">
              <label
                htmlFor="home-outside"
                className="cursor-pointer text-muted-foreground"
              >
                Reachable from outside home
              </label>
              <Switch
                id="home-outside"
                size="sm"
                checked={hs.outside}
                onCheckedChange={(v) => void set(true, v, hs.public ?? "")}
              />
            </div>
          )}
          {hs.line && (
            <p className="text-xs text-warning">{plainLine(hs.line)}</p>
          )}
          {askPublic && (
            <form
              className="flex gap-2"
              onSubmit={(e) => {
                e.preventDefault()
                void set(true, hs.outside, publicAddr.trim())
              }}
            >
              <Input
                aria-label="Public address"
                placeholder={
                  hs.public || "Public address (optional): IP or host name"
                }
                value={publicAddr}
                spellCheck={false}
                autoComplete="off"
                onChange={(e) => setPublicAddr(e.target.value)}
              />
              <Button type="submit" variant="outline">
                Save
              </Button>
            </form>
          )}
        </>
      )}
      {hs.lan && <p className="text-xs text-warning">{plainLine(hs.lan)}</p>}
      {hs.remote && (
        <p className="text-xs text-warning">{plainLine(hs.remote)}</p>
      )}
      {error && <span className="text-xs text-destructive">{error}</span>}
      <button
        type="button"
        aria-expanded={advanced}
        className="justify-self-start text-xs text-muted-foreground underline-offset-2 outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring"
        onClick={() => setAdvanced((v) => !v)}
      >
        {advanced ? "Hide advanced" : "Advanced"}
      </button>
      {advanced && <Relay open={open} />}
    </div>
  )
}

// Relay is the advanced relay line (§4.4, §9): a TURN relay you run or rent
// elsewhere, used beside the home server's. "None" with none set, else the
// credential URL (its query hidden); a warning only when it is failing.
// Change opens a field for the credential URL.
function Relay({ open }: { open: boolean }) {
  const [relay, setRelay] = useState<RelayView | null>(null)
  const [editing, setEditing] = useState(false)
  const [url, setUrl] = useState("")
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    let live = true
    void actions
      .relay()
      .then((r) => live && setRelay(r))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [open])

  const save = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      setRelay(await actions.setRelay(url.trim()))
      setEditing(false)
      setUrl("")
    } catch (err) {
      setError(errorText(err))
    }
  }

  const direct = !relay || relay.state === "none"
  return (
    <div className="grid gap-1 text-[13px]">
      {editing ? (
        <form className="flex gap-2" onSubmit={(e) => void save(e)}>
          <Input
            autoFocus
            aria-label="TURN credential URL"
            placeholder="https://… TURN credential URL (empty: none)"
            value={url}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setUrl(e.target.value)}
          />
          <Button type="submit">
            Save
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={() => setEditing(false)}
          >
            Back
          </Button>
        </form>
      ) : (
        <div className="flex items-center justify-between gap-2">
          <span className="min-w-0 truncate text-muted-foreground">
            Other relay:{" "}
            <span className="text-ink">
              {direct ? "none" : plainLine(relay?.credentialUrl ?? "")}
            </span>
          </span>
          <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>
            Change
          </Button>
        </div>
      )}
      {relay?.state === "error" && (
        <span className="text-xs text-warning">
          The relay is failing: {plainLine(relay.detail ?? "")}
        </span>
      )}
      {error && <span className="text-xs text-destructive">{error}</span>}
    </div>
  )
}

// Start offers Pair a device and Join with a code.
function Start({ onStarted }: { onStarted: (s: PairStatus) => void }) {
  const [role, setRole] = useState("desktop")
  const [joining, setJoining] = useState(false)
  const [code, setCode] = useState("")
  const [away, setAway] = useState(false)
  const [server, setServer] = useState("")
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
    if (code.trim())
      void run(() => actions.join(code.trim(), away ? server.trim() : ""))
  }

  return (
    <div className="grid gap-2">
      {joining ? (
        <form className="grid gap-2" onSubmit={submitCode}>
          <div className="flex gap-2">
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
          </div>
          {away ? (
            <Input
              aria-label="Home server address"
              placeholder="Home server address, like 203.0.113.7:24817"
              value={server}
              spellCheck={false}
              autoComplete="off"
              className="font-mono"
              onChange={(e) => setServer(e.target.value)}
            />
          ) : (
            <button
              type="button"
              className="justify-self-start text-xs text-muted-foreground underline-offset-2 outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring"
              onClick={() => setAway(true)}
            >
              Joining from outside your home?
            </button>
          )}
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
  if (s.mode === "add" && s.state === PairState.Granted) {
    const who = plainLine(s.joiner?.name ?? "") || "the new device"
    return (
      <div className="grid gap-2">
        <p className="text-[13px] text-ink">
          The words match. Now type this device's key on {who}:
        </p>
        <p className="rounded-lg bg-muted px-3 py-2 text-center font-mono text-base tracking-widest text-ink select-all">
          {s.fingerprint}
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
  if (s.state === PairState.Words)
    return <ReadWords status={s} cancel={cancel} />
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
        On the new device, open Settings → Devices → Join with a code, and type
        this code:
      </p>
      <p className="rounded-lg bg-muted px-3 py-2 text-center font-mono text-lg tracking-widest text-ink select-all">
        {s.code}
      </p>
      <p className="text-xs text-muted-foreground">
        Treat it like a one-use password: whoever has it can reach this device
        first.{expires ? ` It works for ${left} more min.` : ""}
      </p>
      {s.fingerprint && (
        <p className="text-xs text-muted-foreground">
          The new device will ask for this device's key:{" "}
          <span className="font-mono text-ink">{s.fingerprint}</span>
        </p>
      )}
      {s.serverAddr && (
        <p className="text-xs text-muted-foreground">
          Joining from outside your home? Server address{" "}
          <span className="font-mono text-ink select-all">{s.serverAddr}</span>
        </p>
      )}
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

// ReadWords is the joining device: the words to type on the other device,
// and the box for that device's key, which it shows beside its code. The
// pairing is taken only from the device whose connection proved that key,
// so someone else holding the code can't answer in its place.
function ReadWords({
  status: s,
  cancel,
}: {
  status: PairStatus
  cancel: ReactNode
}) {
  const [key, setKey] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const who = plainLine(s.peer ?? "") || "the other device"

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!key.trim()) return
    setBusy(true)
    setError(null)
    try {
      await actions.confirmKey(key.trim())
    } catch (err) {
      setError(errorText(err))
      setKey("")
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid gap-2">
      <p className="text-[13px] text-ink">Type these words on {who}:</p>
      <p className="rounded-lg bg-muted px-3 py-2 font-mono text-base tracking-wide text-ink select-all">
        {(s.words ?? []).join(" ")}
      </p>
      {s.keyOk ? (
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground">
            Waiting for {who}…
          </span>
          {cancel}
        </div>
      ) : (
        <form className="grid gap-2" onSubmit={(e) => void submit(e)}>
          <Input
            aria-label="The other device's key"
            placeholder={`Then type the key ${who} shows, then Enter`}
            value={key}
            disabled={busy}
            spellCheck={false}
            autoComplete="off"
            className="font-mono"
            aria-invalid={error ? true : undefined}
            onChange={(e) => setKey(e.target.value)}
          />
          {error && <p className="text-xs text-destructive">{error}</p>}
          <div className="flex justify-end">{cancel}</div>
        </form>
      )}
    </div>
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

// ColorPick is a device's colour (card #22): a swatch that opens the
// palette. Its agents' faces and its single-device cards wear it, on every
// device.
function ColorPick({
  device: d,
  onPicked,
}: {
  device: DeviceView
  onPicked: () => Promise<void>
}) {
  const [open, setOpen] = useState(false)
  const [error, setError] = useState("")
  const name = plainLine(d.name) || "this device"
  const pick = async (c: string) => {
    try {
      await actions.color(d.deviceId, c)
      setError("")
      setOpen(false)
      await onPicked()
    } catch (e) {
      setError(errorText(e))
    }
  }
  return (
    <span className="flex items-center gap-1">
      {open ? (
        DeviceColors.map((c) => (
          <button
            key={c}
            type="button"
            title={c}
            aria-label={`${c} for ${name}`}
            aria-pressed={d.color === c}
            className={cn(
              "size-4 rounded-full outline-none focus-visible:ring-1 focus-visible:ring-ring",
              d.color === c && "ring-1 ring-ink"
            )}
            style={{ background: `var(--device-${c})` }}
            onClick={() => void pick(c)}
          />
        ))
      ) : (
        <button
          type="button"
          title={`${name}'s colour: ${d.color || "default"}. Change it`}
          aria-label={`Change ${name}'s colour`}
          className="size-4 rounded-full ring-1 ring-border outline-none focus-visible:ring-ring"
          style={{
            background: d.color ? `var(--device-${d.color})` : undefined,
          }}
          onClick={() => setOpen(true)}
        />
      )}
      {error && <span className="text-xs text-destructive">{error}</span>}
    </span>
  )
}
