// The phone's system bars (card #395). Irori draws edge to edge on Android,
// under the status and navigation bars; a WebView before 136 reads
// env(safe-area-inset-*) as 0, so MainActivity also hands the insets over
// (window.IroriInsets, in CSS px). index.css's --safe-top and friends take
// the larger of the two; pages pad with those, never env() directly.

type Insets = {
  top(): number
  bottom(): number
  left(): number
  right(): number
}

export function followInsets(): void {
  const src = (window as unknown as { IroriInsets?: Insets }).IroriInsets
  if (!src) return
  const apply = () => {
    const s = document.documentElement.style
    s.setProperty("--android-inset-top", `${src.top()}px`)
    s.setProperty("--android-inset-bottom", `${src.bottom()}px`)
    s.setProperty("--android-inset-left", `${src.left()}px`)
    s.setProperty("--android-inset-right", `${src.right()}px`)
  }
  apply()
  window.addEventListener("irori:insets", apply)
  window.addEventListener("resize", apply)
}
