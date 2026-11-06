// Where this checkout's browser preview runs (card #610): PREVIEW_URL if set,
// else the URL `bun run preview:look` wrote to frontend/.preview-url. No
// default port: 9246 is often another worktree's preview, and shooting that
// shows someone else's code.
import { readFileSync, realpathSync } from "node:fs"
import { join } from "node:path"

export function previewUrl(): string {
  if (process.env.PREVIEW_URL) return process.env.PREVIEW_URL
  let url = ""
  try {
    url = readFileSync(
      join(import.meta.dir, "..", ".preview-url"),
      "utf8"
    ).trim()
  } catch {
    // not running
  }
  if (!url)
    throw new Error(
      "no preview running in this checkout: start `bun run preview:look` (it writes frontend/.preview-url) or set PREVIEW_URL"
    )
  return url.endsWith("/") ? url : url + "/"
}

// The folder a preview serves, from vite.config.ts's /__preview-root; null
// when nothing answers or it isn't one of ours.
export async function servedRoot(url: string): Promise<string | null> {
  try {
    const r = await fetch(new URL("__preview-root", url))
    return r.ok ? normal(await r.text()) : null
  } catch {
    return null
  }
}

export const frontendDir = normal(realpathSync(join(import.meta.dir, "..")))

function normal(p: string): string {
  return p.trim().replace(/\\/g, "/").replace(/\/$/, "").toLowerCase()
}

// Fails unless this checkout's own preview answers. A preview:look killed too
// hard to remove its .preview-url leaves the port free, and the next
// worktree's preview may take it: then the root it serves isn't ours.
// PREVIEW_URL set by hand skips the root check (any server you point it at).
export async function checkPreview(url: string): Promise<void> {
  const root = await servedRoot(url)
  if (process.env.PREVIEW_URL) {
    if (
      !(await fetch(url).then(
        () => true,
        () => false
      ))
    )
      throw new Error(
        `nothing at ${url}: start \`bun run preview:look\` or set PREVIEW_URL`
      )
    return
  }
  if (root === null)
    throw new Error(
      `no preview at ${url}: start \`bun run preview:look\` in this checkout`
    )
  if (root !== frontendDir)
    throw new Error(
      `${url} is another checkout's preview (${root}): start \`bun run preview:look\` in this one`
    )
}

// Start Chrome with --remote-debugging-port=0 so two runs never pick the same
// port; Chrome writes the one it took to DevToolsActivePort in its profile.
export async function devtoolsPort(profile: string): Promise<number> {
  for (let i = 0; i < 100; i++) {
    try {
      const port = Number(
        readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0]
      )
      if (port) return port
    } catch {
      // not yet
    }
    await Bun.sleep(100)
  }
  throw new Error("Chrome's DevTools port never answered")
}
