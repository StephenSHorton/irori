// Screenshots of the loading skeletons (card #598). Opens each loading spot
// with ?slow (frontend/PREVIEW.md: it keeps every Loading on its skeleton)
// in a headless Chrome, in dark and light, with and without Reduce Motion,
// and saves PNGs to frontend/preview-shots/loading/.
//
//   bun run preview:look                       # in another terminal
//   bun scripts/loading-shots.ts               # or name pages: card talk
//
// PREVIEW_URL (default: frontend/.preview-url, which preview:look writes), BROWSER_EXE, SHOT_WIDTH
// (1280), SHOT_HEIGHT (800), SHOT_OUT (the folder), SHOT_FRAMES=n also saves
// n frames 120 ms apart of each page in dark, with and without Reduce
// Motion (for a GIF, and to see the reduced one hold still).
//
// Only Chrome's DevTools protocol over a WebSocket, as phone-check.ts.
/* eslint-disable @typescript-eslint/no-explicit-any -- DevTools replies are untyped JSON */
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawn } from "node:child_process"
import { checkPreview, devtoolsPort, previewUrl } from "./preview-url.ts"

const width = Number(process.env.SHOT_WIDTH ?? 1280)
const height = Number(process.env.SHOT_HEIGHT ?? 800)
// A phone width (SHOT_WIDTH=390) shoots as a phone: the width comes from
// DevTools' device emulation, so it isn't held at headless Chrome's 500 px
// minimum and cropped. SHOT_MOBILE=0 or 1 overrides.
const mobile = process.env.SHOT_MOBILE
  ? process.env.SHOT_MOBILE === "1"
  : width < 768
const frames = Number(process.env.SHOT_FRAMES ?? 0)
// SHOT_LOADED=1 shoots the same pages without ?slow: the loaded page, to
// compare with its skeleton.
const loaded = process.env.SHOT_LOADED === "1"
const out =
  process.env.SHOT_OUT ??
  join(import.meta.dir, "..", "preview-shots", "loading")

// The loading spots moved onto Loading, each held on its skeleton by ?slow.
const PAGES: Array<[name: string, query: string]> = [
  ["card", "?slow#card/sample-card-1"],
  ["card-cold", "?slow#card/sample-card-gone"],
  ["home", "?slow"],
  ["done", "?slow&filter=done"],
  ["notifications", "?slow#notifications"],
  ["notes", "?slow&notes"],
  ["talk", "?slow#talk"],
  ["thread", "?slow&thread=sample-thread-1"],
  ["channel", "?slow#talk/sample-ch-suzuri"],
  ["settings", "?slow&settings"],
  ["settings-agents", "?slow&settings=agents"],
  ["settings-devices", "?slow&settings=devices"],
  ["settings-people", "?slow&settings=people"],
  ["settings-app", "?slow&settings=app"],
  ["settings-desks", "?slow&settings=desks"],
  ["personal", "?slow#board/personal"],
  ["board", "?slow#board/sample-board-mlg"],
]

function findBrowser(): string {
  const c = [
    process.env.BROWSER_EXE,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].filter(Boolean) as string[]
  const hit = c.find((p) => existsSync(p))
  if (!hit) throw new Error("no Chrome or Edge found; set BROWSER_EXE")
  return hit
}

class Cdp {
  private id = 0
  private waiting = new Map<number, (m: any) => void>()
  constructor(private ws: WebSocket) {
    ws.onmessage = (e) => {
      const m = JSON.parse(String(e.data))
      const w = m.id && this.waiting.get(m.id)
      if (w) {
        this.waiting.delete(m.id)
        w(m)
      }
    }
  }
  send(method: string, params: object = {}, sessionId?: string): Promise<any> {
    const id = ++this.id
    this.ws.send(JSON.stringify({ id, method, params, sessionId }))
    return new Promise((res, rej) =>
      this.waiting.set(id, (m) =>
        m.error
          ? rej(new Error(`${method}: ${m.error.message}`))
          : res(m.result)
      )
    )
  }
}

// Runs in the page: how many skeletons are up, and whether any animates.
const PROBE = `(() => {
  const busy = document.querySelectorAll('[skullet-loading]').length
  const moving = document.getAnimations().filter((a) => a.playState === 'running').length
  return { busy, moving }
})()`

async function main() {
  mkdirSync(out, { recursive: true })
  const only = process.argv.slice(2)
  const pages = only.length ? PAGES.filter(([n]) => only.includes(n)) : PAGES
  if (!pages.length)
    throw new Error(
      `no page named ${only.join(", ")}; pages: ${PAGES.map(([n]) => n).join(" ")}`
    )
  const base = previewUrl()
  await checkPreview(base)
  const profile = mkdtempSync(join(tmpdir(), "loading-shots-"))
  const chrome = spawn(
    findBrowser(),
    [
      "--headless=new",
      "--disable-gpu",
      "--hide-scrollbars",
      "--no-first-run",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { stdio: "ignore" }
  )
  let empty = 0
  try {
    const port = await devtoolsPort(profile)
    const ver: any = await fetch(`http://127.0.0.1:${port}/json/version`).then(
      (r) => r.json()
    )
    const ws = new WebSocket(ver.webSocketDebuggerUrl)
    await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)))
    const cdp = new Cdp(ws)
    for (const [name, query] of pages)
      for (const mode of ["dark", "light"])
        for (const reduce of [false, true]) {
          const { targetId } = await cdp.send("Target.createTarget", {
            url: "about:blank",
          })
          const { sessionId: s } = await cdp.send("Target.attachToTarget", {
            targetId,
            flatten: true,
          })
          await cdp.send(
            "Emulation.setDeviceMetricsOverride",
            { width, height, deviceScaleFactor: 1, mobile },
            s
          )
          await cdp.send(
            "Emulation.setEmulatedMedia",
            {
              features: [
                {
                  name: "prefers-reduced-motion",
                  value: reduce ? "reduce" : "no-preference",
                },
              ],
            },
            s
          )
          const [raw, hash = ""] = query.split("#")
          const path = loaded
            ? raw.replace(/^\?slow&?/, "?").replace(/^\?$/, "")
            : raw
          const url = `${base}${path}${path ? "&" : "?"}mode=${mode}${hash ? `#${hash}` : ""}`
          await cdp.send("Page.navigate", { url }, s)
          await Bun.sleep(1800)
          const { result } = await cdp.send(
            "Runtime.evaluate",
            { expression: PROBE, returnByValue: true },
            s
          )
          const v = result.value
          const tag = `${name}-${mode}${reduce ? "-reduced" : ""}${loaded ? "-loaded" : ""}`
          const shot = await cdp.send(
            "Page.captureScreenshot",
            { format: "png" },
            s
          )
          writeFileSync(
            join(out, `${tag}.png`),
            Buffer.from(shot.data, "base64")
          )
          if (frames && mode === "dark")
            for (let f = 0; f < frames; f++) {
              const fr = await cdp.send(
                "Page.captureScreenshot",
                { format: "png" },
                s
              )
              writeFileSync(
                join(
                  out,
                  `${name}${reduce ? "-reduced" : ""}-frame-${String(f).padStart(3, "0")}.png`
                ),
                Buffer.from(fr.data, "base64")
              )
              await Bun.sleep(120)
            }
          await cdp.send("Target.closeTarget", { targetId })
          if (!v.busy && !loaded) empty++
          console.log(
            `${v.busy ? "ok  " : "NONE"} ${tag.padEnd(34)} ${v.busy} skeleton(s), ${v.moving} animation(s)`
          )
        }
    ws.close()
  } finally {
    chrome.kill()
    // Chrome can hold its profile a moment after the kill (Windows).
    await Bun.sleep(500)
    try {
      rmSync(profile, { recursive: true, force: true })
    } catch {
      // left in the temp folder
    }
  }
  console.log(
    empty
      ? `${empty} shot(s) showed no skeleton; PNGs in ${out}`
      : `PNGs in ${out}`
  )
  process.exit(empty ? 1 : 0)
}

main().catch((e) => {
  console.error(String(e?.message ?? e))
  process.exit(2)
})
