// `bun run preview:look` (frontend/PREVIEW.md, card #610). Starts the browser
// preview on the first free port from 9246 up and writes its URL to
// frontend/.preview-url (gitignored), which phone-check.ts, loading-shots.ts
// and preview-shots.sh read. Several worktrees can each run one: every one
// gets its own port, and each worktree's shot scripts find its own preview.
//
// Vite runs as `node node_modules/vite/bin/vite.js`, not `bunx vite`, which
// can exit silently in a worktree. --strictPort makes it fail rather than
// move to a port this script didn't write down; if another preview took the
// port between the check and the start, it tries the next one.
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join } from "node:path"
import { spawn, type ChildProcess } from "node:child_process"
import { frontendDir, servedRoot } from "./preview-url.ts"

const dir = join(import.meta.dir, "..")
const urlFile = join(dir, ".preview-url")
const vite = join(dir, "node_modules", "vite", "bin", "vite.js")
const FIRST = 9246
const LAST = 9299

function free(port: number): Promise<boolean> {
  return new Promise((res) => {
    const s = createServer()
    s.once("error", () => res(false))
    s.listen({ port, host: "127.0.0.1", exclusive: true }, () =>
      s.close(() => res(true))
    )
  })
}

// Up once the port serves this folder (vite.config.ts's /__preview-root):
// another launcher's vite that won the port in a race doesn't count. Returns
// vite's exit code if it exited first, null when it's up.
async function up(child: ChildProcess, url: string): Promise<number | null> {
  let exit: number | null = null
  child.once("exit", (code) => (exit = code ?? 1))
  for (;;) {
    if (exit !== null) return exit
    if ((await servedRoot(url)) === frontendDir) return null
    await Bun.sleep(150)
  }
}

let child: ChildProcess | undefined
let url = ""

function forget() {
  // Only our own line: a newer preview:look in this folder may have replaced it.
  try {
    if (
      url &&
      existsSync(urlFile) &&
      readFileSync(urlFile, "utf8").trim() === url
    )
      rmSync(urlFile)
  } catch {
    // gone already
  }
}

for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"] as const)
  process.on(sig, () => {
    forget()
    if (child && child.exitCode === null) child.kill(sig)
    else process.exit(130)
  })

async function main() {
  if (!existsSync(vite))
    throw new Error(
      "no vite in node_modules: run `bun install --frozen-lockfile` first"
    )
  for (let port = FIRST; port <= LAST; port++) {
    if (!(await free(port))) continue
    const at = `http://127.0.0.1:${port}/`
    child = spawn(
      "node",
      [vite, "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
      {
        cwd: dir,
        stdio: "inherit",
      }
    )
    const failed = await up(child, at)
    if (failed !== null) {
      // Someone took the port after the check: try the next one.
      if (!(await free(port))) continue
      process.exit(failed)
    }
    url = at
    writeFileSync(urlFile, url + "\n")
    console.log(
      `\n  preview:look on ${url}  (written to frontend/.preview-url)\n`
    )
    const c = child
    const code =
      c.exitCode ??
      (await new Promise<number>((res) => c.once("exit", (n) => res(n ?? 0))))
    forget()
    process.exit(code)
  }
  throw new Error(`no free port from ${FIRST} to ${LAST}`)
}

main().catch((e) => {
  forget()
  console.error(String(e?.message ?? e))
  process.exit(2)
})
