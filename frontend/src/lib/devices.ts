// Devices gateway (docs/design/v0.4-devices.md §3, §9): the person's
// devices, pairing (Pair a device, Join with a code, the words) and Remove.
// Like cards.ts, components import from here, never from src/gen. In a
// plain browser it runs on a small sample so the dialog can be tried: a
// pairing there moves on by itself, and the sample's words are "sample".

import { useCallback, useEffect, useState } from "react"

import { DevicesService, Events } from "./host"
import {
  DeviceView,
  HomeServerView,
  PairJoiner as Joiner,
  PairStatus,
} from "./models"
import { EventDevicesChanged, EventRosterChanged, inWails } from "./irori"
import { useLatest } from "./talk"

export { DeviceView, HomeServerView, PairStatus }

// EventPairingChanged mirrors the Go constant in devices_service.go.
export const EventPairingChanged = "devices:pairing"

// PairState mirrors internal/pair's states.
export const PairState = {
  Waiting: "waiting", // add: the code is shown; join: looking for the other device
  Confirm: "confirm", // add: type the words the new device shows
  Granted: "granted", // add: the words matched; type this device's key there
  Words: "words", // join: type these words on the other device, and its key here
  Done: "done",
  Failed: "failed",
  Canceled: "canceled",
  Expired: "expired",
  CodeUsed: "code_used",
} as const

// RefreshMs is how often the open dialog re-reads the devices: liveness
// changes without an event.
const RefreshMs = 5000

// isOpen reports whether a pairing is still in progress.
export function isOpen(s: PairStatus | null): boolean {
  return (
    s?.state === PairState.Waiting ||
    s?.state === PairState.Confirm ||
    s?.state === PairState.Granted ||
    s?.state === PairState.Words
  )
}

// ---- sample mode ------------------------------------------------------------

const SampleWords = ["sample", "words", "for", "this", "browser"]
const SampleCode = "K7QM-2XWD-9RTF-HB4P"
const SampleKey = "3f9a 0c21 77de 41b0"

let sampleDevices: DeviceView[] | null = null
let samplePair = PairStatus({ mode: "", state: "" })
let sampleTimer: number | undefined
let sampleHome = HomeServerView({
  can: true,
  publicRelays: true,
  suggest: true,
  on: false,
  outside: true,
  awake: true,
})
const sampleListeners = new Set<() => void>()

function sampleList(): DeviceView[] {
  sampleDevices ??= [
    DeviceView({
      deviceId: "sample-pc",
      name: "Stephen's PC",
      role: "desktop",
      this: true,
      state: "this",
      color: "teal",
      sides: ["Personal"],
    }),
    DeviceView({
      deviceId: "sample-mac",
      name: "MacBook",
      role: "desktop",
      color: "orange",
      state: "connected",
      lastSeen: new Date().toISOString(),
      reach: "direct",
      version: "0.12.1",
      sides: ["Work"],
    }),
    // Reached only through another device (card #544).
    DeviceView({
      deviceId: "sample-laptop",
      name: "Laptop",
      role: "desktop",
      color: "indigo",
      state: "recent",
      lastSeen: new Date(Date.now() - 2 * 60_000).toISOString(),
      via: "Phone",
      reach: "unreachable",
      version: "0.12.1",
    }),
    DeviceView({
      deviceId: "sample-node",
      name: "Node",
      role: "node",
      color: "pink",
      state: "offline",
      lastSeen: new Date(Date.now() - 3 * 3600_000).toISOString(),
      reach: "unreachable",
      version: "0.11.0",
      behind: "0.12.1",
      warning: "It runs an older Irori. Update it.",
    }),
  ]
  return sampleDevices
}

function setSamplePair(s: Partial<PairStatus>) {
  samplePair = PairStatus({ ...samplePair, ...s })
  for (const fn of sampleListeners) fn()
}

// sampleJoined is the new device typing this one's key: it joins the list.
function sampleJoined(name: string) {
  sampleList().push(
    DeviceView({
      deviceId: "sample-new",
      name,
      role: samplePair.role || "desktop",
      state: "connected",
      lastSeen: new Date().toISOString(),
    })
  )
  setSamplePair({ state: PairState.Done })
}

function sampleLater(fn: () => void) {
  window.clearTimeout(sampleTimer)
  sampleTimer = window.setTimeout(fn, 1500)
}

const sample = {
  add(role: string): PairStatus {
    if (isOpen(samplePair)) throw new Error("a pairing is already open")
    samplePair = PairStatus({
      mode: "add",
      state: PairState.Waiting,
      code: SampleCode,
      fingerprint: SampleKey,
      serverAddr: sampleHome.address,
      role,
      expires: new Date(Date.now() + 10 * 60_000).toISOString(),
    })
    sampleLater(() =>
      setSamplePair({
        state: PairState.Confirm,
        joiner: Joiner({
          deviceId: "sample-new",
          name: "New laptop",
          os: "darwin",
          fingerprint: "3f9a 0c21",
        }),
      })
    )
    return samplePair
  },
  join(server: string): PairStatus {
    if (isOpen(samplePair)) throw new Error("a pairing is already open")
    samplePair = PairStatus({
      mode: "join",
      state: PairState.Waiting,
      via: server || undefined,
    })
    sampleLater(() =>
      setSamplePair({
        state: PairState.Words,
        words: SampleWords,
        peer: "Stephen's PC",
      })
    )
    return samplePair
  },
  confirm(words: string) {
    if (samplePair.state !== PairState.Confirm)
      throw new Error("no pairing is waiting for words")
    if (words.trim().toLowerCase().split(/\s+/)[0] !== SampleWords[0]) {
      const misses = (samplePair.misses ?? 0) + 1
      if (misses >= 2)
        setSamplePair({
          state: PairState.Failed,
          misses,
          error: "Someone else tried to use this code. Make a new one.",
        })
      else setSamplePair({ misses })
      throw new Error("those aren't the words the other device shows")
    }
    const name = samplePair.joiner?.name ?? "New laptop"
    setSamplePair({ state: PairState.Granted })
    sampleLater(() => sampleJoined(name))
  },
  confirmKey(key: string) {
    if (samplePair.state !== PairState.Words || samplePair.keyOk)
      throw new Error("no pairing is waiting for a key")
    if (
      key.replace(/[^0-9a-f]/gi, "").toLowerCase() !==
      SampleKey.replace(/ /g, "")
    ) {
      const misses = (samplePair.misses ?? 0) + 1
      if (misses >= 2)
        setSamplePair({
          state: PairState.Failed,
          misses,
          error:
            "That isn't the key the other device shows: someone else may have the code. Make a new one.",
        })
      else setSamplePair({ misses })
      throw new Error("that isn't the key the other device shows")
    }
    setSamplePair({ keyOk: true })
    sampleLater(() => setSamplePair({ state: PairState.Done }))
  },
  cancel() {
    window.clearTimeout(sampleTimer)
    if (isOpen(samplePair)) setSamplePair({ state: PairState.Canceled })
  },
  color(id: string, color: string) {
    const d = sampleList().find((x) => x.deviceId === id)
    if (d) d.color = color || DeviceColors[0]
    for (const fn of sampleListeners) fn()
  },
  setSides(sides: string[]): string[] {
    const d = sampleList().find((x) => x.this)
    if (!d) throw new Error("no device here")
    const kept = cleanSides(sides)
    d.sides = kept.length ? kept : undefined
    for (const s of kept)
      if (!sampleSideNames.some((n) => sameSide(n, s))) sampleSideNames.push(s)
    for (const fn of sampleListeners) fn()
    return kept
  },
  rename(id: string, name: string): string {
    const d = sampleList().find((x) => x.deviceId === id)
    if (!d) throw new Error("that isn't one of your devices")
    sampleHostNames[id] ??= d.name
    const kept = cleanDeviceName(name)
    d.name = kept || sampleHostNames[id]
    for (const fn of sampleListeners) fn()
    return kept
  },
  remove(id: string) {
    const d = sampleList().find((x) => x.deviceId === id)
    if (!d || d.this) throw new Error("that device can't be removed here")
    d.revoked = true
    d.state = "offline"
    d.warning = "Removed."
    for (const fn of sampleListeners) fn()
  },
}

// ---- actions ----------------------------------------------------------------

// Every action is the person at this device, through Wails only: pairing
// and Remove are not on the loopback API.
export const devices = {
  // add opens a pairing window for a new device with role "desktop" or
  // "node": the code to type on it.
  async add(role: string): Promise<PairStatus> {
    if (inWails()) return DevicesService.AddDevice(role)
    return sample.add(role)
  },
  // join enters a code shown on another device; server is the home
  // server's address, typed only when joining from outside the home.
  async join(code: string, server = ""): Promise<PairStatus> {
    if (inWails()) return DevicesService.JoinWithCode(code, server)
    return sample.join(server)
  },
  // confirm takes the words the new device shows, typed here.
  async confirm(words: string): Promise<void> {
    if (inWails()) return DevicesService.ConfirmWords(words)
    sample.confirm(words)
  },
  // confirmKey takes, on the joining device, the key the other device
  // shows beside its code: the pairing is taken only from that device.
  async confirmKey(key: string): Promise<void> {
    if (inWails()) return DevicesService.ConfirmKey(key)
    sample.confirmKey(key)
  },
  async cancel(): Promise<void> {
    if (inWails()) return DevicesService.CancelPairing()
    sample.cancel()
  },
  // reset makes this device a new one (its log, id and keys move aside;
  // settings stay). Irori quits, and opening it again finishes it.
  async reset(): Promise<void> {
    if (inWails()) return DevicesService.ResetDevice()
    throw new Error("Reset needs the desktop app")
  },
  // homeServer is the Home server section: on this device, the switch
  // and how far it reaches; on the others, a line when it can't be reached.
  async homeServer(): Promise<HomeServerView> {
    if (inWails()) return DevicesService.HomeServer()
    return sampleHome
  },
  // setHomeServer switches this device's home server on or off.
  async setHomeServer(
    on: boolean,
    outside: boolean,
    publicAddr: string
  ): Promise<HomeServerView> {
    if (inWails()) return DevicesService.SetHomeServer(on, outside, publicAddr)
    sampleHome = HomeServerView({
      ...sampleHome,
      on,
      outside,
      public: publicAddr || undefined,
      running: on,
      port: 24817,
      reach: !on
        ? undefined
        : publicAddr
          ? "outside"
          : outside
            ? "forward"
            : "home",
      address: on && publicAddr ? `${publicAddr}:24817` : undefined,
      awakeLine: on && sampleHome.awake ? "Keeping this PC awake" : undefined,
      line:
        on && outside && !publicAddr
          ? "Away from home your devices can't reach this PC. Turn on UPnP on your router, or forward TCP and UDP port 24817 to 192.168.1.20 and enter your public address below."
          : undefined,
    })
    return sampleHome
  },
  // setHomeServerAwake switches "Keep this PC awake while it's the server".
  async setHomeServerAwake(on: boolean): Promise<HomeServerView> {
    if (inWails()) return DevicesService.SetHomeServerAwake(on)
    sampleHome = HomeServerView({
      ...sampleHome,
      awake: on,
      awakeLine: sampleHome.running && on ? "Keeping this PC awake" : undefined,
    })
    return sampleHome
  },
  // setPublicRelays switches "Meet through public relays" (card #328).
  async setPublicRelays(on: boolean): Promise<HomeServerView> {
    if (inWails()) return DevicesService.SetPublicRelays(on)
    sampleHome = HomeServerView({ ...sampleHome, publicRelays: on })
    return sampleHome
  },
  // remove revokes a device: its later rows are refused everywhere and the
  // person key rotates on the devices that remain (decision D8).
  async remove(deviceId: string): Promise<void> {
    if (inWails()) return DevicesService.RemoveDevice(deviceId)
    sample.remove(deviceId)
  },
  // color picks a device's colour (card #22); "" goes back to its default.
  async color(deviceId: string, color: string): Promise<void> {
    if (inWails()) return DevicesService.SetDeviceColor(deviceId, color)
    sample.color(deviceId, color)
  },
  // personColor is the person's own colour on shared boards (card #385),
  // "" for the default each friend's screen picks.
  async personColor(): Promise<string> {
    if (inWails()) return DevicesService.PersonColor()
    return samplePersonColor
  },
  // setPersonColor picks it; "" goes back to the default.
  async setPersonColor(color: string): Promise<void> {
    if (inWails()) return DevicesService.SetPersonColor(color)
    samplePersonColor = color
  },
  // personAvatar is the person's own picture on shared spaces (card
  // #393): a small image data URL, "" for none.
  async personAvatar(): Promise<string> {
    if (inWails()) return DevicesService.PersonAvatar()
    return samplePersonAvatar
  },
  // setPersonAvatar picks it; "" takes it off.
  async setPersonAvatar(avatar: string): Promise<void> {
    if (inWails()) return DevicesService.SetPersonAvatar(avatar)
    samplePersonAvatar = avatar
  },
  // assistantName is the name the person gave their Office Assistant (card
  // #1045), "" for the default.
  async assistantName(): Promise<string> {
    if (inWails()) return DevicesService.AssistantName()
    return sampleAssistantName
  },
  // assistantLabel is the name as shown for their own assistant: the chosen
  // name, else "<name>'s assistant".
  async assistantLabel(): Promise<string> {
    if (inWails()) return DevicesService.AssistantLabel()
    return sampleAssistantName || "Sam's assistant"
  },
  // setAssistantName sets it; "" goes back to the default.
  async setAssistantName(name: string): Promise<void> {
    if (inWails()) return DevicesService.SetAssistantName(name)
    sampleAssistantName = name.trim()
  },
  // sideNames is every work/personal side named on a device or a card
  // (card #494), for this device's pin to choose from.
  async sideNames(): Promise<string[]> {
    if (inWails()) return (await DevicesService.SideNames()) ?? []
    return [...sampleSideNames]
  },
  // setSides pins the sides this device runs, the first the one its new
  // cards take; [] runs every side again (card #494). Returns them as kept.
  async setSides(sides: string[]): Promise<string[]> {
    if (inWails()) return (await DevicesService.SetDeviceSides(sides)) ?? []
    return sample.setSides(sides)
  },
  // rename is the person's name for a device (card #184); "" goes back to
  // the computer's name. Returns the name as kept.
  async rename(deviceId: string, name: string): Promise<string> {
    if (inWails()) return DevicesService.RenameDevice(deviceId, name)
    return sample.rename(deviceId, name)
  },
}

let samplePersonColor = ""
let samplePersonAvatar = ""
let sampleAssistantName = ""

// MaxDeviceName mirrors coord::devname::MAX_LEN.
export const MaxDeviceName = 40

// cleanDeviceName is coord::devname::clean: one line, trimmed, at most
// MaxDeviceName characters.
export function cleanDeviceName(name: string): string {
  const one = name.replace(/\s+/g, " ").trim()
  return [...one].slice(0, MaxDeviceName).join("")
}

// MaxSide and MaxSides mirror coord::sides::MAX_LEN and MAX_SIDES.
export const MaxSide = 40
export const MaxSides = 8

// cleanSide is coord::sides::clean: one line, trimmed, at most MaxSide
// characters; "" is no side.
export function cleanSide(side: string): string {
  return cleanDeviceName(side)
}

// sameSide is coord::sides::same: case doesn't count.
export function sameSide(a: string, b: string): boolean {
  return cleanSide(a).toLowerCase() === cleanSide(b).toLowerCase()
}

// uniqueSides is each side cleaned, empty ones and repeats (any case)
// dropped, order kept: every side there is to offer.
export function uniqueSides(sides: string[]): string[] {
  const out: string[] = []
  for (const s of sides) {
    const c = cleanSide(s)
    if (c && !out.some((o) => sameSide(o, c))) out.push(c)
  }
  return out
}

// cleanSides is coord::sides::clean_list: uniqueSides, at most MaxSides:
// a device's pin.
export function cleanSides(sides: string[]): string[] {
  return uniqueSides(sides).slice(0, MaxSides)
}

const sampleSideNames: string[] = ["Personal", "Work"]

// The sample devices' computer names, for a rename back to "".
const sampleHostNames: Record<string, string> = {}

// DeviceColors is internal/devcolor.Palette: the colours a device can wear.
export const DeviceColors = [
  "teal",
  "orange",
  "pink",
  "lime",
  "cyan",
  "indigo",
  "sand",
]

// ---- hooks ------------------------------------------------------------------

// onPairing runs fn with every pairing change.
function onPairing(fn: (s: PairStatus) => void): () => void {
  if (!inWails()) {
    const l = () => fn(samplePair)
    sampleListeners.add(l)
    return () => sampleListeners.delete(l)
  }
  return Events.On<PairStatus>(EventPairingChanged, (ev) =>
    fn(PairStatus(ev.data ?? {}))
  )
}

export type DevicesState = {
  devices: DeviceView[]
  error: string | null
  // True until the first read lands, for the list's skeleton (card #598);
  // a reload never sets it again.
  loading: boolean
  reload: () => Promise<void>
}

// useDevices reads the person's devices while active (the dialog is open):
// at once, on every pairing or roster change, and every few seconds, since
// a device going quiet sends no event.
export function useDevices(active: boolean): DevicesState {
  const load = useCallback(
    async (): Promise<DeviceView[]> =>
      inWails()
        ? ((await DevicesService.Devices()) ?? [])
        : sampleList().map((d) => DeviceView(d)),
    []
  )
  const { value, error, loading, reload } = useLatest<DeviceView[]>(load, [])

  useEffect(() => {
    if (!active) return
    void reload()
    const id = window.setInterval(() => void reload(), RefreshMs)
    const offPair = onPairing(() => void reload())
    const offRoster = inWails()
      ? Events.OnShown(EventRosterChanged, () => void reload())
      : () => {}
    const offDevices = inWails()
      ? Events.OnShown(EventDevicesChanged, () => void reload())
      : () => {}
    return () => {
      window.clearInterval(id)
      offPair()
      offRoster()
      offDevices()
    }
  }, [active, reload])

  return { devices: value, error, loading, reload }
}

// usePairing follows the open (or last) pairing while active. A pairing
// that had already ended when the dialog opened isn't shown again.
export function usePairing(active: boolean): {
  status: PairStatus | null
  setStatus: (s: PairStatus | null) => void
} {
  const [status, setStatus] = useState<PairStatus | null>(null)

  useEffect(() => {
    if (!active) return
    let live = true
    const first = inWails()
      ? DevicesService.PairingStatus()
      : Promise.resolve(samplePair)
    void first
      .then((s) => live && setStatus(isOpen(s) ? s : null))
      .catch(() => live && setStatus(null))
    const off = onPairing((s) => live && setStatus(s))
    return () => {
      live = false
      off()
    }
  }, [active])

  return { status, setStatus }
}

// reachLine is how this device reaches another and what it is behind on
// (card #1119): "direct", "relay" or "unreachable", then "runs 0.11.0,
// behind 0.12.1" when its version is older than the newest. "" for this
// device and a removed one.
export function reachLine(d: DeviceView): string {
  if (d.this || d.revoked || !d.reach) return ""
  // Its rows still reach this device through another (card #544): no
  // session of its own, not out of reach.
  const parts: string[] = [
    d.reach === "relay"
      ? "via relay"
      : d.reach === "unreachable" && d.via
        ? "no direct link"
        : d.reach,
  ]
  if (d.behind)
    parts.push(`${d.version ? `runs ${d.version}, ` : ""}behind ${d.behind}`)
  return parts.join(" · ")
}

// stateDot is a device's one status dot.
export function stateDot(state: string): string {
  switch (state) {
    case "this":
    case "connected":
      return "bg-success"
    case "recent":
      return "bg-warning"
  }
  return "bg-muted-foreground/40"
}
