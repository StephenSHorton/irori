import { describe, expect, test } from "bun:test"

import { uaIsAndroid, updateErrorLine } from "./android"

describe("uaIsAndroid (card #270)", () => {
  test("the Android WebView's user agent", () => {
    expect(
      uaIsAndroid(
        "Mozilla/5.0 (Linux; Android 15; Pixel 9 Build/AP4A; wv) AppleWebKit/537.36 Chrome/131.0 Mobile Safari/537.36"
      )
    ).toBe(true)
  })
  test("desktop webviews are not", () => {
    expect(
      uaIsAndroid(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15"
      )
    ).toBe(false)
    expect(
      uaIsAndroid("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/131.0")
    ).toBe(false)
  })
})

describe("updateErrorLine (card #270)", () => {
  test("a bad hash says the APK was deleted and nothing installed", () => {
    expect(
      updateErrorLine("update checksum mismatch: got ab, want cd")
    ).toContain("Nothing was installed")
  })
  test("a release with no APK or no checksum", () => {
    expect(
      updateErrorLine("release v0.7.0 has no Android arm64 APK asset")
    ).toBe("This release has no Android app yet.")
    expect(updateErrorLine("release v0.7.0 has no SHA256SUMS asset")).toContain(
      "no checksum"
    )
  })
  test("anything else keeps its reason, shortened", () => {
    expect(updateErrorLine("download x: 503")).toBe(
      "The update didn't finish: download x: 503"
    )
    expect(updateErrorLine("x".repeat(300)).length).toBeLessThan(180)
    expect(updateErrorLine("")).toBe("The update didn't finish.")
  })
})
