import { describe, expect, test } from "bun:test"

import {
  clampWidth,
  defaultWidth,
  dragTo,
  foldBelow,
  maxWidth,
  minWidth,
  refoldMargin,
  unfoldAbove,
  type Drag,
} from "./sidebar-width"

const open = (width: number): Drag => ({
  folded: false,
  width,
  catching: false,
})
const shut = (width: number): Drag => ({ folded: true, width, catching: false })

describe("clampWidth", () => {
  test("keeps it between the ends", () => {
    expect(clampWidth(100)).toBe(minWidth)
    expect(clampWidth(9000)).toBe(maxWidth)
    expect(clampWidth(300.4)).toBe(300)
  })
  test("a bad stored value is the default", () => {
    expect(clampWidth(Number("abc"))).toBe(defaultWidth)
  })
})

describe("dragTo", () => {
  test("an open drag sets the width live", () => {
    expect(dragTo(320, open(240))).toEqual(open(320))
  })
  test("between the fold threshold and the min it holds at the min", () => {
    expect(dragTo(foldBelow + 1, open(300))).toEqual(open(minWidth))
  })
  test("past the threshold it folds and keeps the last width", () => {
    expect(dragTo(foldBelow - 1, open(300))).toMatchObject(shut(300))
  })
  test("folding keeps the width the drag started from", () => {
    let d: Drag = { ...open(298), start: 298 }
    for (const x of [250, 200, 180, 160, 150, 60]) d = dragTo(x, d)
    expect(d).toMatchObject({ folded: true, width: 298 })
  })
  test("once a drag folds it, moving on left keeps it folded", () => {
    // The pointer passes the fold line, then keeps going toward the icons.
    let d = open(240)
    for (const x of [200, 160, 150, 140, 120, 100, 60]) d = dragTo(x, d)
    expect(d.folded).toBe(true)
    // Back past the fold line (plus the margin) it opens at the last width.
    d = dragTo(foldBelow + refoldMargin, d)
    expect(d.folded).toBe(true)
    d = dragTo(foldBelow + refoldMargin + 1, d)
    expect(d).toMatchObject({ folded: false, width: 192, catching: true })
  })
  test("a folded drag stays folded until it clears the unfold line", () => {
    expect(dragTo(unfoldAbove, shut(300))).toEqual(shut(300))
  })
  test("dragging out of folded opens it at the last width", () => {
    const d = dragTo(unfoldAbove + 1, shut(300))
    expect(d).toEqual({ folded: false, width: 300, catching: true })
    // It waits for the pointer, then follows it.
    expect(dragTo(250, d)).toEqual(d)
    expect(dragTo(340, d)).toEqual(open(340))
    // Back over the icons folds it again.
    expect(dragTo(40, d).folded).toBe(true)
    expect(dragTo(80, dragTo(40, d)).folded).toBe(true)
  })
  test("past the last width it opens straight to the pointer", () => {
    expect(dragTo(400, shut(300))).toEqual(open(400))
  })
})
