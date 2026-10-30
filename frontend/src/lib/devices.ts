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
  RelayView,
} from "./models"
import { EventRosterChanged, inWails } from "./irori"
import { useLatest } from "./talk"

export { DeviceView, HomeServerView, PairStatus, RelayView }

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
let sampleRelay = RelayView({ state: "none" })
let sampleHome = HomeServerView({
  can: true,
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
    }),
    DeviceView({
      deviceId: "sample-mac",
      name: "MacBook",
      role: "desktop",
      color: "orange",
      state: "connected",
      lastSeen: new Date().toISOString(),
    }),
    DeviceView({
      deviceId: "sample-node",
      name: "Node",
      role: "node",
      color: "pink",
      state: "offline",
      lastSeen: new Date(Date.now() - 3 * 3600_000).toISOString(),
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
          ? "Away from home your devices can't reach this PC. Turn on UPnP on your router, or forward TCP and UDP port 24817 to 192.168.1.20."
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
  // relay is the advanced TURN relay setting: state "none" is none set.
  async relay(): Promise<RelayView> {
    if (inWails()) return DevicesService.Relay()
    return sampleRelay
  },
  // setRelay sets the advanced relay's TURN REST credential URL ("" removes
  // it).
  async setRelay(url: string): Promise<RelayView> {
    if (inWails()) return DevicesService.SetRelay(url)
    sampleRelay = RelayView({
      credentialUrl: url.replace(/[?#].*$/, "?…") || undefined,
      state: url ? "pending" : "none",
    })
    return sampleRelay
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
}

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
  const { value, error, reload } = useLatest<DeviceView[]>(load, [])

  useEffect(() => {
    if (!active) return
    void reload()
    const id = window.setInterval(() => void reload(), RefreshMs)
    const offPair = onPairing(() => void reload())
    const offRoster = inWails()
      ? Events.On(EventRosterChanged, () => void reload())
      : () => {}
    return () => {
      window.clearInterval(id)
      offPair()
      offRoster()
    }
  }, [active, reload])

  return { devices: value, error, reload }
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
