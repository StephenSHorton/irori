// Where this checkout's browser preview runs (card #610): PREVIEW_URL if set,
// else the URL `bun run preview:look` wrote to frontend/.preview-url. No
// default port: 9246 is often another worktree's preview, and shooting that
// shows someone else's code.
import { readFileSync } from "node:fs"
import { join } from "node:path"

export function previewUrl(): string {
  if (process.env.PREVIEW_URL) return process.env.PREVIEW_URL
  let url = ""
  try {
    url = readFileSync(join(import.meta.dir, "..", ".preview-url"), "utf8").trim()
  } catch {
    // not running
  }
  if (!url)
    throw new Error(
      "no preview running in this checkout: start `bun run preview:look` (it writes frontend/.preview-url) or set PREVIEW_URL",
    )
  return url.endsWith("/") ? url : url + "/"
}

// Fails when nothing answers, as after a preview:look killed too hard to
// remove its .preview-url.
export async function checkPreview(url: string): Promise<void> {
  const ok = await fetch(url).then(() => true, () => false)
  if (!ok) throw new Error(`nothing at ${url}: start \`bun run preview:look\` or set PREVIEW_URL`)
}

// Start Chrome with --remote-debugging-port=0 so two runs never pick the same
// port; Chrome writes the one it took to DevToolsActivePort in its profile.
export async function devtoolsPort(profile: string): Promise<number> {
  for (let i = 0; i < 100; i++) {
    try {
      const port = Number(readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0])
      if (port) return port
    } catch {
      // not yet
    }
    await Bun.sleep(100)
  }
  throw new Error("Chrome's DevTools port never answered")
}
