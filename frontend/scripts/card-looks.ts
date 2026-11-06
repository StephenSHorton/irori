// Writes the notification cards' looks (card #527) as JSON, one file per
// theme and mode, for the screenshot loop:
//
//   bun scripts/card-looks.ts <outdir>
//
// <theme>-<mode>.json for every theme (and a custom one) in light and dark
// under Frost, and <theme>-<mode>-<glass>.json for Blue dark under the
// other glass styles. Each is the CardLook settings_set_card_look takes.
import { mkdirSync } from "node:fs"
import { join } from "node:path"

import { themes, type Appearance, type Mode } from "../src/lib/appearance"
import { cardLook } from "../src/lib/card-look"

const out = process.argv[2]
if (!out) {
  console.error("usage: bun scripts/card-looks.ts <outdir>")
  process.exit(2)
}
mkdirSync(out, { recursive: true })

const custom = { base: "blue", accent: "#e2683c", tint: "#5a6b7c" }
const modes: Mode[] = ["light", "dark"]

async function write(name: string, a: Appearance) {
  await Bun.write(join(out, `${name}.json`), JSON.stringify(cardLook(a), null, 2) + "\n")
  console.log(`wrote ${join(out, name)}.json`)
}

for (const theme of [...themes.map((t) => t.id), "custom"])
  for (const mode of modes)
    await write(`${theme}-${mode}`, {
      theme,
      mode,
      glass: "frost",
      custom: theme === "custom" ? custom : undefined,
    })

for (const glass of ["clear", "milk", "rim", "ember", "liquid"])
  await write(`blue-dark-${glass}`, { theme: "blue", mode: "dark", glass })
