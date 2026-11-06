// Phone-width check of the browser preview (card #479). Opens each main page
// at 390 x 844 as a phone (touch, no hover) in a headless Chrome, fails when
// the page or anything on it reaches past the right edge, warns about text
// that gets cut or ends in "…", and saves a screenshot of each page to
// frontend/preview-shots/phone/.
//
//   bun run preview:look                 # in another terminal
//   bun run check:phone                  # or: bun scripts/phone-check.ts home card-1
//
// PREVIEW_URL (default: frontend/.preview-url, which preview:look writes), BROWSER_EXE, PHONE_WIDTH
// (390), PHONE_HEIGHT (844; taller shows more of a long page), PHONE_DPR (1),
// PHONE_OUT (the folder), PHONE_TRACE=1 (where a cut sits), PHONE_MOBILE=0 for a narrow desktop window instead.
//
// Why not `preview-shots.sh` with --window-size=390,…: Chrome won't make a
// headless window narrower than 500 px, so the page lays out at 500 and the
// PNG is cropped to 390. Boards and the right-hand buttons then look cut off
// when they aren't (the 11-04 audit hit this). Here the width comes from
// DevTools' device emulation, and the run fails if the page doesn't see it.
//
// Only Chrome's DevTools protocol over a WebSocket: no extra packages.
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

const width = Number(process.env.PHONE_WIDTH ?? 390)
const height = Number(process.env.PHONE_HEIGHT ?? 844)
const dpr = Number(process.env.PHONE_DPR ?? 1)
const mobile = process.env.PHONE_MOBILE !== "0"
const out =
  process.env.PHONE_OUT ?? join(import.meta.dir, "..", "preview-shots", "phone")

// The main pages a phone reaches (rows of PREVIEW.md).
const PAGES: Array<[name: string, query: string]> = [
  ["home", ""],
  ["home-spawn", "?spawn=states"],
  ["card-1", "?open=board:task:sample-card-1"],
  ["card-2", "?open=board:task:sample-card-2"],
  ["card-4", "?open=board:task:sample-card-4"],
  ["card-7", "?open=board:task:sample-card-7"],
  ["card-12", "?open=board:task:sample-card-12"],
  ["card-shared", "?open=board:task:sample-board-1"],
  ["notes", "?notes"],
  ["notifications", "#notifications"],
  ["held", "?open=held"],
  ["held-msg", "?open=inbox:sample-delivery-1"],
  ["agent-msg", "?open=inbox:sample-delivery-6"],
  ["settings", "?settings"],
  ["settings-agents", "?settings=agents"],
  ["settings-devices", "?settings=devices"],
  ["settings-appearance", "?settings=appearance"],
  ["talk", "#talk"],
  ["channel", "#talk/sample-ch-suzuri"],
  ["dm", "#talk/sample-dm-heron"],
  ["thread", "?thread=sample-thread-1"],
  ["boards", "?boards"],
  ["board-mlg", "#board/sample-board-mlg"],
  ["board-net", "#board/sample-board-mlg/sample-board-mlg-net"],
  ["personal", "?space=personal#boards"],
  // Freshness lines (card #580): a space's Home, Personal's, every log a few
  // days behind, and the space menu open with each one's.
  ["home-space", "?space=sample-board-mlg"],
  ["home-personal", "?space=personal"],
  ["home-stale", "?fresh=stale"],
  ["board-stale", "?fresh=stale#board/sample-board-mlg"],
  ["spaces-menu", "?spaces=open"],
  ["new-card", "?newcard"],
  ["done", "?filter=done"],
  // The voice-call mock-up (card #571).
  ["call-live", "?call=live#talk/sample-ch-suzuri"],
  ["call-transcript", "?call=transcript#talk/sample-ch-suzuri"],
  ["later-home", "?later=only"],
  ["later-open", "?later=only&notyet"],
  ["later-board", "?later=board#board/sample-board-mlg"],
  ["later-card", "?later=1&open=board:task:sample-card-later-90"],
  ["later-order", "?later=1&open=board:task:sample-card-order-82"],
  ["later-hold", "?later=hold&open=board:task:sample-card-1"],
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

// Runs in the page. `off`: boxes past the right edge (outermost only), which
// fail the page; a box inside a sideways scroller that itself fits is fine,
// you can scroll to it. `cut`: text or a control cut short by an
// overflow-hidden box, and `ellipsis`: text that ends in "…"; both warn.
const PROBE = `(() => {
  const vw = innerWidth
  const press = 'button,a,[role=button],[role=tab],[role=switch],input,textarea,select'
  const label = (el) => {
    const t = (el.innerText || el.getAttribute('aria-label') || el.getAttribute('placeholder') || '').trim().replace(/\\s+/g, ' ').slice(0, 60)
    return el.tagName.toLowerCase() + (t ? ' "' + t + '"' : ' .' + (el.getAttribute('class') || '').slice(0, 80))
  }
  const shown = (el, r) => {
    if (r.width === 0 || r.height === 0) return false
    const s = getComputedStyle(el)
    return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0
  }
  // The nearest ancestor that clips sideways, and whether it scrolls.
  const clipper = (el) => {
    for (let p = el.parentElement; p && p !== document.documentElement; p = p.parentElement) {
      const o = getComputedStyle(p).overflowX
      if (o !== 'visible') return { p, scrolls: o === 'auto' || o === 'scroll' }
    }
    return null
  }
  const ownText = (el) => [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())
  const off = [], cut = [], ellipsis = []
  for (const el of document.querySelectorAll('body *')) {
    const r = el.getBoundingClientRect()
    if (!shown(el, r)) continue
    const c = clipper(el)
    const cr = c && c.p.getBoundingClientRect()
    if (r.right > vw + 0.5 && !(c && cr.right <= vw + 0.5)) {
      const pr = el.parentElement && el.parentElement !== document.body && el.parentElement.getBoundingClientRect()
      if (!(pr && pr.right > vw + 0.5)) off.push({ el: label(el), right: Math.round(r.right), top: Math.round(r.top) })
      continue
    }
    if (c && !c.scrolls && r.right > cr.right + 1 && (el.matches(press) || ownText(el)))
      cut.push({ el: label(el), right: Math.round(r.right), box: Math.round(cr.right), top: Math.round(r.top),
        path: [el.parentElement, el.parentElement?.parentElement, c.p].map((x) => x && x.tagName.toLowerCase() + '.' + (x.getAttribute('class') || '').slice(0, 70)).join(' < ') })
    const s = getComputedStyle(el)
    if (s.textOverflow === 'ellipsis' && el.scrollWidth > el.clientWidth + 1)
      ellipsis.push({ el: label(el), top: Math.round(r.top) })
  }
  return { vw, scrollWidth: document.documentElement.scrollWidth, off, cut, ellipsis }
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
  const profile = mkdtempSync(join(tmpdir(), "phone-check-"))
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
  let failed = 0
  try {
    const port = await devtoolsPort(profile)
    const ver: any = await fetch(`http://127.0.0.1:${port}/json/version`).then(
      (r) => r.json()
    )
    const ws = new WebSocket(ver.webSocketDebuggerUrl)
    await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)))
    const cdp = new Cdp(ws)
    for (const [name, query] of pages) {
      const { targetId } = await cdp.send("Target.createTarget", {
        url: "about:blank",
      })
      const { sessionId: s } = await cdp.send("Target.attachToTarget", {
        targetId,
        flatten: true,
      })
      await cdp.send(
        "Emulation.setDeviceMetricsOverride",
        { width, height, deviceScaleFactor: dpr, mobile },
        s
      )
      if (mobile) {
        await cdp.send(
          "Emulation.setTouchEmulationEnabled",
          { enabled: true, maxTouchPoints: 5 },
          s
        )
        await cdp.send(
          "Emulation.setEmulatedMedia",
          {
            features: [
              { name: "hover", value: "none" },
              { name: "pointer", value: "coarse" },
              { name: "any-hover", value: "none" },
              { name: "any-pointer", value: "coarse" },
            ],
          },
          s
        )
      }
      await cdp.send("Page.navigate", { url: base + query }, s)
      await Bun.sleep(1800)
      const { result, exceptionDetails } = await cdp.send(
        "Runtime.evaluate",
        { expression: PROBE, returnByValue: true },
        s
      )
      if (exceptionDetails)
        throw new Error(`probe failed on ${name}: ${exceptionDetails.text}`)
      const v = result.value
      const shot = await cdp.send(
        "Page.captureScreenshot",
        { format: "png" },
        s
      )
      writeFileSync(join(out, `${name}.png`), Buffer.from(shot.data, "base64"))
      await cdp.send("Target.closeTarget", { targetId })
      const problems: string[] = []
      if (v.vw !== width)
        problems.push(
          `the page saw a ${v.vw}px window, not ${width}px: the check itself is wrong`
        )
      if (v.scrollWidth > width)
        problems.push(`the page is ${v.scrollWidth}px wide`)
      for (const o of v.off)
        problems.push(`past the edge (right ${o.right}, y ${o.top}): ${o.el}`)
      const ok = problems.length === 0
      if (!ok) failed++
      const warn = v.cut.length + v.ellipsis.length
      console.log(
        `${ok ? "ok  " : "FAIL"} ${name.padEnd(20)}${warn ? ` ${warn} warning(s)` : ""}`
      )
      for (const p of problems.slice(0, 20)) console.log(`       ${p}`)
      for (const o of v.cut.slice(0, 12))
        console.log(
          `       warn cut at ${o.box} (right ${o.right}, y ${o.top}): ${o.el}${process.env.PHONE_TRACE ? `\n         in ${o.path}` : ""}`
        )
      for (const o of v.ellipsis.slice(0, 12))
        console.log(`       warn ends in … (y ${o.top}): ${o.el}`)
    }
    ws.close()
  } finally {
    chrome.kill()
    rmSync(profile, { recursive: true, force: true })
  }
  console.log(
    failed
      ? `${failed} of ${pages.length} page(s) don't fit ${width}px; screenshots in ${out}`
      : `all ${pages.length} pages fit ${width}px; screenshots in ${out}`
  )
  process.exit(failed ? 1 : 0)
}

main().catch((e) => {
  console.error(String(e?.message ?? e))
  process.exit(2)
})
