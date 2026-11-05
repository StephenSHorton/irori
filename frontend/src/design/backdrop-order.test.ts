// Card #506: the glass blur was missing from the built app. Vite's CSS
// minifier (lightningcss) folds `backdrop-filter` followed by
// `-webkit-backdrop-filter` into the -webkit- line alone, and Chromium
// (WebView2) ignores that one, so nothing behind a panel or a menu blurred.
// The dev server doesn't minify, so the browser preview looked fine.
// With -webkit- first and the standard line last, both survive.
import { describe, expect, test } from "bun:test"
import { readFileSync, readdirSync, statSync } from "node:fs"
import { join } from "node:path"
import { transform } from "lightningcss"

const src = join(import.meta.dir, "..")

function cssFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n)
    if (statSync(p).isDirectory()) return cssFiles(p)
    return n.endsWith(".css") ? [p] : []
  })
}

// Each innermost { ... } block that sets a backdrop filter.
function blocks(css: string): string[] {
  const out: string[] = []
  const re = /\{([^{}]*)\}/g
  for (let m = re.exec(css); m; m = re.exec(css))
    if (m[1].includes("backdrop-filter")) out.push(m[1])
  return out
}

const glass = cssFiles(src).flatMap((f) =>
  blocks(readFileSync(f, "utf8")).map((b) => ({ file: f.slice(src.length + 1), b }))
)

// Vite's default build.cssTarget ("baseline-widely-available").
const targets = { chrome: 111 << 16, edge: 111 << 16, firefox: 114 << 16, safari: (16 << 16) | (4 << 8) }

describe("backdrop-filter survives the production build", () => {
  test("there are glass rules to check", () => {
    expect(glass.length).toBeGreaterThan(1)
  })

  for (const { file, b } of glass) {
    const body = b.replace(/\/\*[\s\S]*?\*\//g, "")
    if (!/(^|[^-])backdrop-filter\s*:/.test(body)) continue
    test(`${file}: ${body.trim().split("\n")[0].slice(0, 60)}`, () => {
      const min = transform({ filename: "x.css", code: Buffer.from(`.x{${body}}`), minify: true, targets })
        .code.toString()
      expect(min).toMatch(/(^|[;{])backdrop-filter:/)
    })
  }
})
