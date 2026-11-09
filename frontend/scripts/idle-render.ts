// Idle render count (card #1137): opens a preview page in a headless Chrome,
// lets it settle, then counts for IDLE_MS (10 s) with nothing happening:
// React commits, function components rendered, and DOM mutations. A window
// that is idle should do (almost) none; a once-a-second whole-tree render
// shows up as ~IDLE_MS/1000 commits and thousands of components rendered.
//
//   bun run preview:look               # in another terminal
//   bun scripts/idle-render.ts home '?thread=sample-thread-1' '#talk/sample-ch-suzuri'
//
// Arguments are URL queries/hashes after the preview's base URL ("home" = ""),
// each run in its own tab. PREVIEW_URL, BROWSER_EXE, IDLE_MS as phone-check.ts.
/* eslint-disable @typescript-eslint/no-explicit-any -- DevTools replies are untyped JSON */
import { existsSync, mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawn } from "node:child_process"
import { checkPreview, devtoolsPort, previewUrl } from "./preview-url.ts"

const idleMs = Number(process.env.IDLE_MS ?? 10_000)

function findBrowser(): string {
  const c = [
    process.env.BROWSER_EXE,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].filter(Boolean) as string[]
  const hit = c.find((p) => existsSync(p))
  if (!hit) throw new Error("no Chrome or Edge found; set BROWSER_EXE")
  return hit
}

// Before React loads: count commits and the function components each rendered
// (PerformedWork), as src/perf/harness.ts does; plus a MutationObserver.
const HOOK = `(() => {
  const s = (window.__idle = { commits: 0, rendered: 0, mutations: 0, on: false, names: {}, tops: {} })
  const nameOf = (t) => !t ? '?' : typeof t === 'function' ? (t.displayName || t.name || '?') : (t.displayName || nameOf(t.render || t.type))
  // A subtree whose child is the one it had was bailed out: its old flags
  // are stale and don't count (as React's devtools do).
  const walk = (f, under) => {
    let n = 0
    for (; f; f = f.sibling) {
      const comp = f.tag === 0 || f.tag === 1 || f.tag === 11 || f.tag === 14 || f.tag === 15
      const did = comp && ((f.flags & 1) || !f.alternate)
      let here = under
      if (did) {
        n++
        const k = nameOf(f.type)
        s.names[k] = (s.names[k] || 0) + 1
        if (!under) s.tops[k] = (s.tops[k] || 0) + 1
        here = true
      }
      if (f.child && (!f.alternate || f.child !== f.alternate.child)) n += walk(f.child, here)
    }
    return n
  }
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true, renderers: new Map(), inject() { return 1 },
    onScheduleFiberRoot() {}, onCommitFiberUnmount() {}, onPostCommitFiberRoot() {},
    onCommitFiberRoot(_id, root) { if (s.on) { s.commits++; s.rendered += walk(root.current.child, false) } },
    checkDCE() {},
  }
  new MutationObserver((l) => { if (s.on) s.mutations += l.length })
    .observe(document, { subtree: true, childList: true, characterData: true, attributes: true })
})()`

async function main() {
  const base = previewUrl()
  await checkPreview(base)
  const pages = process.argv.slice(2).map((a) => (a === "home" ? "" : a))
  if (!pages.length) pages.push("")
  const profile = mkdtempSync(join(tmpdir(), "idle-render-"))
  const chrome = spawn(
    findBrowser(),
    [
      "--headless=new",
      "--disable-gpu",
      "--no-first-run",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { stdio: "ignore" }
  )
  try {
    const port = await devtoolsPort(profile)
    const ver: any = await fetch(`http://127.0.0.1:${port}/json/version`).then(
      (r) => r.json()
    )
    const ws = new WebSocket(ver.webSocketDebuggerUrl)
    await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)))
    let id = 0
    const waiting = new Map<number, (m: any) => void>()
    ws.onmessage = (e) => {
      const m = JSON.parse(String(e.data))
      const w = m.id && waiting.get(m.id)
      if (w) {
        waiting.delete(m.id)
        w(m)
      }
    }
    const send = (
      method: string,
      params: object = {},
      sessionId?: string
    ): Promise<any> => {
      const n = ++id
      ws.send(JSON.stringify({ id: n, method, params, sessionId }))
      return new Promise((res, rej) =>
        waiting.set(n, (m) =>
          m.error
            ? rej(new Error(`${method}: ${m.error.message}`))
            : res(m.result)
        )
      )
    }
    for (const q of pages) {
      const { targetId } = await send("Target.createTarget", {
        url: "about:blank",
      })
      const { sessionId: s } = await send("Target.attachToTarget", {
        targetId,
        flatten: true,
      })
      await send(
        "Emulation.setDeviceMetricsOverride",
        { width: 1300, height: 900, deviceScaleFactor: 1, mobile: false },
        s
      )
      await send("Page.enable", {}, s)
      await send(
        "Page.addScriptToEvaluateOnNewDocument",
        { source: HOOK, runImmediately: true },
        s
      )
      await send("Page.navigate", { url: base + q }, s)
      await Bun.sleep(3000)
      await send(
        "Runtime.evaluate",
        { expression: "window.__idle.on = true" },
        s
      )
      await Bun.sleep(idleMs)
      const { result } = await send(
        "Runtime.evaluate",
        { expression: "JSON.stringify(window.__idle)", returnByValue: true },
        s
      )
      if (result.value === undefined)
        throw new Error("probe: " + JSON.stringify(result))
      const v = JSON.parse(result.value)
      console.log(
        `${(q || "home").padEnd(34)} ${idleMs / 1000}s idle: ${v.commits} commits, ${v.rendered} components rendered, ${v.mutations} DOM mutations`
      )
      if (process.env.IDLE_NAMES)
        console.log("   tops: " + JSON.stringify(v.tops))
      if (process.env.IDLE_NAMES)
        console.log(
          "   " +
            Object.entries(v.names as Record<string, number>)
              .sort((a, b) => b[1] - a[1])
              .slice(0, 14)
              .map(([k, n]) => `${k}×${n}`)
              .join(" ")
        )
      if (process.env.IDLE_TEXT) {
        const t = await send(
          "Runtime.evaluate",
          { expression: "document.body.innerText", returnByValue: true },
          s
        )
        await Bun.write(
          `${process.env.IDLE_TEXT}-${(q || "home").replace(/\W+/g, "_")}.txt`,
          t.result.value
        )
      }
      await send("Target.closeTarget", { targetId })
    }
    ws.close()
  } finally {
    chrome.kill()
    rmSync(profile, { recursive: true, force: true })
  }
}
main().catch((e) => {
  console.error(e)
  process.exit(1)
})
