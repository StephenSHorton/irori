import { previewFlag } from "@/lib/preview"

// Card #270: on Android the backend never downloads on its own. A newer
// release shows as "available"; the person's tap (the same RestartToUpdate
// call desktop uses) downloads the APK, checks it against SHA256SUMS and
// opens Android's install screen.

// uaIsAndroid: a user agent of Android's WebView, which Irori's phone build
// runs in.
export function uaIsAndroid(ua: string): boolean {
  return /\bAndroid\b/i.test(ua)
}

// isAndroid: this page is Irori on a phone, or the browser preview's
// ?android.
export function isAndroid(): boolean {
  return (
    previewFlag("android") !== null ||
    uaIsAndroid(globalThis.navigator?.userAgent ?? "")
  )
}

// The backend's line when installing unknown apps had to be allowed first
// (crates/update NEEDS_INSTALL_PERMISSION).
export const NEEDS_INSTALL_PERMISSION =
  "Allow Irori to install apps on the page that just opened, then tap Install again."

// updateErrorLine is the banner's words for an update that failed on the
// phone: a bad hash says nothing was installed; anything else gives the
// reason in short.
export function updateErrorLine(error: string): string {
  const e = error.trim()
  if (/checksum mismatch/i.test(e))
    return "The download didn't match the release's checksum, so it was deleted. Nothing was installed."
  if (/has no .*APK asset/i.test(e))
    return "This release has no Android app yet."
  if (/no SHA256SUMS/i.test(e))
    return "This release can't be checked (no checksum), so it won't be installed."
  if (!e) return "The update didn't finish."
  return `The update didn't finish: ${e.length > 140 ? `${e.slice(0, 140)}…` : e}`
}
