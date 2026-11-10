import { throughLine } from "@/lib/agent-status"
import { useEffect, useState, type FormEvent, type ReactNode } from "react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Loading } from "@/components/ui/loading"
import { Switch } from "@/components/ui/switch"
import {
  DeviceColors,
  MaxDeviceName,
  MaxSide,
  MaxSides,
  cleanSide,
  sameSide,
  uniqueSides,
  devices as actions,
  isOpen,
  PairState,
  reachLine,
  stateDot,
  useDevices,
  usePairing,
  type DeviceView,
  type HomeServerView,
  type PairStatus,
} from "@/lib/devices"
import { parseTime, plainLine, relativeTime, useNow } from "@/lib/irori"
import { isMac } from "@/lib/commands"
import { isAndroid } from "@/updater/android"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

const roleChoices = [
  { value: "desktop", label: "Desktop" },
  { value: "node", label: "Often-on PC" },
]

// DevicesPanel is Settings → Devices and connections (docs/design/v0.4-devices.md §3, §9):
// each device, this one first, with one status dot, when it was last seen,
// at most one warning line and Remove; then Pair a device (the code, then
// the words the new device shows, typed here, then this device's key typed
// there) and Join with a code (the words to type on the other device, and
// its key typed here; from outside the home, the server's address too).
// The Home server section sits above them (§7.1). Paths and counters stay
// in GET /v1/devices?detail=1.
export function DevicesPanel({ open }: { open: boolean }) {
  const { devices, error, loading, reload } = useDevices(open, true)
  const { status, setStatus } = usePairing(open)
  const now = useNow(5000)

  return (
    <div className="grid gap-3">
      {/* The list and the sides wait on the first read of the devices
          (card #598); a reload never shows the skeleton again. */}
      <Loading loading={loading} label="Loading your devices">
        {loading ? (
          <DevicesShape />
        ) : (
          <div className="grid gap-3">
            <span className="text-sm">Your devices</span>
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
            {status?.state ? (
              <Pairing status={status} onDone={() => setStatus(null)} />
            ) : (
              <Start onStarted={setStatus} />
            )}
            <Sides
              open={open}
              here={devices.find((d) => d.this)}
              onSaved={reload}
            />
          </div>
        )}
      </Loading>
      <div className="grid gap-1.5 border-t border-border pt-3">
        <span className="text-sm">How this device connects</span>
        <HomeServer open={open} />
      </div>
      <p className="text-xs text-muted-foreground">
        Devices find each other on your local network, through your home server,
        and with public relays on, through iroh&apos;s free relays, which see
        your IP address and which devices connect but never what they say.
        Others on a network see that sealed messages pass, never their content.
        What reaches another device stays there, so a secret an agent wrote that
        the scrubber missed is copied for good.
      </p>
    </div>
  )
}

// DevicesShape and HomeServerShape stand in while the first reads are on
// their way (card #598): plain markup in the shape of the device list, the
// sides and the home server section, with no hooks, for Loading to
// measure. The words only set each line's length; skullet-block draws a
// control's stand-in as one block.
function DevicesShape() {
  const rows = [
    ["Sample PC", "this device · runs Personal"],
    ["Sample laptop", "online · runs Work"],
  ]
  return (
    <div className="grid gap-3">
      <span className="text-sm">Your devices</span>
      <ul className="grid gap-1.5">
        {rows.map(([name, state]) => (
          <li
            key={name}
            className="grid grid-cols-[8px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 text-[13px]"
          >
            <span skullet-block="" className="block size-2 rounded-full" />
            <span className="min-w-0">
              <span className="text-ink">{name}</span>{" "}
              <small className="text-xs text-muted-foreground">{state}</small>
            </span>
            <span className="flex items-center gap-2">
              <span className="block h-6 w-[60px] rounded-lg" />
              <span skullet-block="" className="block size-4 rounded-full" />
            </span>
          </li>
        ))}
      </ul>
      <div className="flex flex-wrap items-center gap-2">
        <span skullet-block="" className="block h-8 w-[108px] rounded-lg" />
        <span skullet-block="" className="block h-7 w-[120px] rounded-lg" />
        <span
          skullet-block=""
          className="ml-auto block h-8 w-[120px] rounded-lg"
        />
      </div>
      <div className="grid gap-1.5 border-t border-border pt-3">
        <span className="text-sm">Sides this device runs</span>
        <span className="flex flex-wrap items-center gap-1.5">
          <span skullet-block="" className="block h-6 w-[66px] rounded-lg" />
          <span skullet-block="" className="block h-6 w-[50px] rounded-lg" />
        </span>
        <span skullet-block="" className="block h-7 w-56 rounded-lg" />
        <small className="text-xs text-muted-foreground">
          This device runs Personal, and cards with no side. New cards typed
          here go on Personal. Every device still shows every card.
        </small>
      </div>
    </div>
  )
}

function HomeServerShape() {
  return (
    <div className="grid gap-1.5 text-[13px]">
      <div className="flex items-center justify-between gap-2">
        <span className="text-ink">Home server: run it on this PC</span>
        <span
          skullet-block=""
          className="block h-5 w-9 shrink-0 rounded-full"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Windows asks once to allow Irori on private networks; the server needs
        it.
      </p>
      <div className="flex items-center justify-between gap-2">
        <span className="min-w-0 text-muted-foreground">
          Meet through public relays
          <small className="block text-xs">
            Reach your devices and invited people from any network, no port
            forwarding.
          </small>
        </span>
        <span
          skullet-block=""
          className="block h-4 w-7 shrink-0 rounded-full"
        />
      </div>
    </div>
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
  const [renaming, setRenaming] = useState(false)
  const seen = parseTime(d.lastSeen)
  // The only removed device listed is one still being removed: a settled
  // removal has no row (card #1240).
  const state = d.this
    ? "this device"
    : d.revoked
      ? "removal pending"
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
      {renaming ? (
        <RenameField
          device={d}
          onDone={async (saved) => {
            setRenaming(false)
            if (saved) await onRemoved()
          }}
        />
      ) : (
        // Wraps rather than ending in "…": on a phone the buttons beside it
        // left no room for "seen 3h ago" (card #479).
        <span className="min-w-0 [overflow-wrap:anywhere]">
          <span className="text-ink">{plainLine(d.name) || d.deviceId}</span>{" "}
          <small className="text-xs text-muted-foreground">
            {d.role === "node" ? "often-on · " : ""}
            {state}
            {d.via ? ` · ${throughLine(d.via)}` : ""}
            {reachLine(d) ? ` · ${reachLine(d)}` : ""}
            {d.sides?.length ? ` · runs ${d.sides.join(", ")}` : ""}
          </small>
        </span>
      )}
      {d.revoked || renaming ? (
        <span />
      ) : (
        <span className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="xs"
            aria-label={`Rename ${plainLine(d.name) || "this device"}`}
            onClick={() => setRenaming(true)}
          >
            Rename
          </Button>
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
          {d.revoked &&
            d.holdouts?.map((h) => (
              <ConfirmButton
                key={h.deviceId}
                label={`Remove ${plainLine(h.name) || "device"}`}
                variant="destructive"
                confirmLabel="Remove device"
                question={
                  <>
                    Remove <b>{plainLine(h.name) || "this device"}</b>? Do this
                    if it is gone for good and will never sync again: the
                    removal then no longer waits on it. It keeps everything it
                    already has.
                  </>
                }
                onConfirm={async () => {
                  await actions.remove(h.deviceId)
                  await onRemoved()
                }}
              />
            ))}
          {d.this && d.action === "reset" && (
            <ConfirmButton
              label="Reset"
              variant="destructive"
              confirmLabel={onPhone() ? "Reset and quit" : "Reset and restart"}
              question={
                <>
                  Reset this device? Its event log, device id and keys move
                  aside (nothing is deleted) and only the settings stay.{" "}
                  {onPhone()
                    ? "Irori quits; open it again to pair this device as a new one."
                    : "Irori restarts as a new device, ready to pair."}
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

// onPhone: a phone can't start Irori again after a Reset, so there it only
// quits (card #1095); a desktop restarts as the new device.
function onPhone(): boolean {
  return isAndroid() || /\b(iPhone|iPad)\b/.test(globalThis.navigator?.userAgent ?? "")
}

// HomeServer is the Home server section (§7.1, §9). On this device: the
// switch, "Reachable from outside home", a word on reach only when it
// falls short, and one line only when something is wrong (with the public
// address field when mapping failed). On every device: a line when this
// one can't be found on its network, and one when your home server can't
// be reached. "Meet through public relays" (card #328) on every device.
// homeLine is the warning under the home server. Its "can't be reached
// from outside" lines (CGNAT, port forward needed) tell the truth about the
// home server, but with public relays on (decision 43) devices and invites
// still reach this PC from any network, so the line says that instead of
// "can't reach this PC" (issue 9). Other lines (couldn't start, port moved,
// an entered address) are shown as they are.
export function homeLine(hs: HomeServerView): string {
  const line = plainLine(hs.line ?? "")
  if (!hs.publicRelays) return line
  const relays =
    "Your home server can't be reached from outside your home, but public relays connect your devices and invited people from any network."
  if (hs.reach === "cgnat") return relays
  if (hs.reach === "forward") {
    // Keep the router steps, for anyone who wants a direct connection too.
    const steps = line.replace(
      /^Away from home your devices can't reach this PC\.\s*/,
      ""
    )
    return steps && steps !== line
      ? `${relays} For a direct connection: ${steps}`
      : relays
  }
  return line
}

function HomeServer({ open }: { open: boolean }) {
  const [hs, setHs] = useState<HomeServerView | null>(null)
  const [publicAddr, setPublicAddr] = useState("")
  const [error, setError] = useState<string | null>(null)
  // A failed first read ends the skeleton; the section stays away, as it
  // always has, until a later read answers.
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    if (!open) return
    let live = true
    const load = () =>
      void actions
        .homeServer()
        .then((v) => live && setHs(v))
        .catch(() => live && setFailed(true))
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
  const setPublicRelays = async (on: boolean) => {
    setError(null)
    try {
      setHs(await actions.setPublicRelays(on))
    } catch (err) {
      setError(errorText(err))
    }
  }
  const setAwake = async (on: boolean) => {
    setError(null)
    try {
      setHs(await actions.setHomeServerAwake(on))
    } catch (err) {
      setError(errorText(err))
    }
  }

  if (!hs) {
    if (failed) return null
    return (
      <Loading key="hs" loading label="Loading the home server">
        <HomeServerShape />
      </Loading>
    )
  }
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
    <Loading key="hs" loading={false} label="Loading the home server">
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
                onCheckedChange={(v) =>
                  void set(v, hs.outside, hs.public ?? "")
                }
              />
            </div>
            {!hs.on && (
              <p className="text-xs text-muted-foreground">
                {hs.suggest &&
                  "This device is on most of the time, so it suits the server: away from home your devices can connect through it if your router allows it, even with public relays off. It's optional: any device that's on often keeps the others up to date. "}
                {!isMac() &&
                  "Windows asks once to allow Irori on private networks; the server needs it."}
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
            {hs.on && (
              <div className="flex items-center justify-between gap-2">
                <label
                  htmlFor="home-awake"
                  className="min-w-0 cursor-pointer text-muted-foreground"
                >
                  Keep this PC awake while it&apos;s the server
                  {hs.awakeLine && (
                    <small
                      className={
                        hs.awake && hs.awakeLine === "Keeping this PC awake"
                          ? "block text-xs"
                          : "block text-xs text-warning"
                      }
                    >
                      {hs.awakeLine}
                    </small>
                  )}
                </label>
                <Switch
                  id="home-awake"
                  size="sm"
                  checked={hs.awake}
                  onCheckedChange={(v) => void setAwake(v)}
                />
              </div>
            )}
            {homeLine(hs) && (
              <p className="text-xs text-warning">{homeLine(hs)}</p>
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
        <div className="flex items-center justify-between gap-2">
          <label
            htmlFor="public-relays"
            className="min-w-0 cursor-pointer text-muted-foreground"
          >
            This device meets through public relays
            <small className="block text-xs">
              {hs.publicRelays
                ? "This device can reach your devices and invited people from any network, no port forwarding. iroh's free relays see your IP address and which devices connect, never what they say."
                : "Away from home, this device connects only through a home server, if you run one. Your other devices have their own switch."}
            </small>
          </label>
          <Switch
            id="public-relays"
            size="sm"
            checked={hs.publicRelays}
            onCheckedChange={(v) => void setPublicRelays(v)}
          />
        </div>
        {hs.lan && <p className="text-xs text-warning">{plainLine(hs.lan)}</p>}
        {hs.remote && (
          <p className="text-xs text-warning">{plainLine(hs.remote)}</p>
        )}
        {error && <span className="text-xs text-destructive">{error}</span>}
      </div>
    </Loading>
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
          {role === "node" && (
            <p className="w-full text-xs text-muted-foreground">
              An often-on PC keeps your devices up to date and suits the home
              server. Irori won&apos;t send it Personal cards made on your other
              devices; a card you post on it can still start there.
            </p>
          )}
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
        On the new device, open Settings → Devices and connections → Join with a
        code, and type this code:
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

// RenameField renames one of the person's devices (card #184): the name
// shows in the header, the Devices list and agents' "on <device>" lines on
// all their devices, and never on a shared board. Empty goes back to the
// computer's name. Enter saves, Esc cancels.
function RenameField({
  device: d,
  onDone,
}: {
  device: DeviceView
  onDone: (saved: boolean) => Promise<void>
}) {
  const [name, setName] = useState(d.name)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const save = async (e?: FormEvent) => {
    e?.preventDefault()
    if (busy) return
    if (name.trim() === d.name.trim()) {
      await onDone(false)
      return
    }
    setBusy(true)
    try {
      await actions.rename(d.deviceId, name)
      setError("")
      await onDone(true)
    } catch (err) {
      setError(errorText(err))
      setBusy(false)
    }
  }
  return (
    <form
      className="col-start-2 col-end-4 grid gap-1"
      onSubmit={(e) => void save(e)}
    >
      <span className="flex items-center gap-2">
        <Input
          autoFocus
          value={name}
          maxLength={MaxDeviceName}
          aria-label="Device name"
          placeholder="The computer's name"
          className="h-7 text-[13px]"
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault()
              e.stopPropagation()
              void onDone(false)
            }
          }}
        />
        <Button type="submit" size="xs" disabled={busy}>
          Save
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="xs"
          onClick={() => void onDone(false)}
        >
          Cancel
        </Button>
      </span>
      <small className="text-xs text-muted-foreground">
        {error ? (
          <span className="text-destructive">{error}</span>
        ) : (
          "Your devices show this name. Leave it empty for the computer's name."
        )}
      </small>
    </form>
  )
}

// Sides is work and personal separation (card #494): the sides this device
// runs, picked here and only here (a device can't pin another), the first
// the one cards typed on it go on. With none picked it runs every side.
// Every device still shows every card: a side says which devices run it.
// A side is named by picking it: the list is every side a device runs or
// a card is on.
export function Sides({
  open,
  here,
  onSaved,
}: {
  open: boolean
  here: DeviceView | undefined
  onSaved: () => Promise<void>
}) {
  const [names, setNames] = useState<string[]>([])
  const [adding, setAdding] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const pinned = here?.sides ?? []
  const key = pinned.join("\n")
  useEffect(() => {
    if (!open) return
    actions
      .sideNames()
      .then(setNames)
      .catch((e: unknown) => setError(errorText(e)))
  }, [open, key])
  if (!here) return null
  const all = uniqueSides([...pinned, ...names])
  const runs = (s: string) => pinned.some((p) => sameSide(p, s))
  const full = pinned.length >= MaxSides
  const save = async (next: string[]) => {
    setBusy(true)
    try {
      await actions.setSides(next)
      setError("")
      await onSaved()
      setNames(await actions.sideNames())
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  const toggle = (s: string) =>
    void save(runs(s) ? pinned.filter((p) => !sameSide(p, s)) : [...pinned, s])
  const add = (e: FormEvent) => {
    e.preventDefault()
    const c = cleanSide(adding)
    if (!c) return
    setAdding("")
    if (!runs(c)) void save([...pinned, c])
  }
  return (
    <div className="grid gap-1.5 border-t border-border pt-3">
      <span className="text-sm">Sides this device runs</span>
      {all.length > 0 && (
        <span
          className="flex flex-wrap items-center gap-1.5"
          role="group"
          aria-label="Sides this device runs"
        >
          {all.map((s) => (
            <Button
              key={s}
              type="button"
              size="xs"
              variant={runs(s) ? "default" : "outline"}
              aria-pressed={runs(s)}
              // A device runs at most MaxSides: a ninth would be cut.
              disabled={busy || (full && !runs(s))}
              onClick={() => toggle(s)}
            >
              {s}
            </Button>
          ))}
        </span>
      )}
      {pinned.length > 1 && (
        <ChoiceSelect
          label="New cards typed here go on"
          showLabel
          size="sm"
          labelClassName="text-xs font-normal text-muted-foreground"
          value={pinned[0]}
          disabled={busy}
          onValueChange={(s) =>
            void save([s, ...pinned.filter((p) => !sameSide(p, s))])
          }
          choices={pinned.map((s) => ({ value: s, label: s }))}
        />
      )}
      <form className="flex items-center gap-2" onSubmit={add}>
        <Input
          value={adding}
          maxLength={MaxSide}
          placeholder={all.length ? "Another side" : "Name a side, like Work"}
          aria-label="Name a side this device runs"
          disabled={busy || full}
          onChange={(e) => setAdding(e.target.value)}
          className="h-7 max-w-56 text-[13px]"
        />
        <Button
          type="submit"
          size="xs"
          variant="ghost"
          disabled={busy || full || !cleanSide(adding)}
        >
          Add
        </Button>
      </form>
      <small className="text-xs text-muted-foreground">
        {error ? (
          <span className="text-destructive">{error}</span>
        ) : (
          sidesLine(pinned)
        )}
      </small>
    </div>
  )
}

// sidesLine says what this device's pin means.
export function sidesLine(pinned: string[]): string {
  if (!pinned.length)
    return "This device runs every side. Pick the sides it runs to keep work and personal cards apart. Cards in shared spaces have no side. Every device still shows every card."
  const list =
    pinned.length === 1
      ? pinned[0]
      : `${pinned.slice(0, -1).join(", ")} and ${pinned[pinned.length - 1]}`
  return `This device runs ${list}, and cards with no side. New cards typed here go on ${pinned[0]}. Cards in shared spaces have no side. Every device still shows every card.`
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
