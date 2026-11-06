// Writes src/design/themes.css from src/lib/appearance.ts (card #275).
// appearance.test.ts fails when the file is stale.
import { themesCSS } from "../src/lib/appearance"

await Bun.write(
  new URL("../src/design/themes.css", import.meta.url),
  themesCSS()
)
console.log("wrote src/design/themes.css")
