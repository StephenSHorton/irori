import { describe, expect, test } from "bun:test"

import { isFormControlTarget } from "./hotkey-target"

// A tiny element stand-in: closest() walks parents and matches the simple
// selector forms FORM_CONTROL_SELECTOR uses (tag, [attr='value']).
type Fake = {
  tag: string
  attrs?: Record<string, string>
  parent?: Fake
  isContentEditable?: boolean
  closest: (selector: string) => Fake | null
}

function matches(el: Fake, sel: string): boolean {
  const attr = sel.match(/^\[([\w-]+)='([^']*)'\]$/)
  if (attr) {
    return el.attrs?.[attr[1]] === attr[2]
  }
  return el.tag === sel
}

function el(tag: string, attrs?: Record<string, string>, parent?: Fake): Fake {
  const node: Fake = {
    tag,
    attrs,
    parent,
    closest(selector) {
      const parts = selector.split(",").map((s) => s.trim())
      for (let cur: Fake | undefined = node; cur; cur = cur.parent) {
        if (parts.some((p) => matches(cur!, p))) {
          return cur
        }
      }
      return null
    },
  }
  return node
}

describe("isFormControlTarget", () => {
  const cases: { name: string; target: unknown; want: boolean }[] = [
    { name: "native input", target: el("input"), want: true },
    { name: "native textarea", target: el("textarea"), want: true },
    { name: "native select", target: el("select"), want: true },
    {
      name: "Select trigger button (role=combobox)",
      target: el("button", { role: "combobox", "data-slot": "select-trigger" }),
      want: true,
    },
    {
      name: "span inside a Select trigger",
      target: el("span", {}, el("button", { role: "combobox" })),
      want: true,
    },
    {
      name: "Select option",
      target: el("div", { role: "option" }),
      want: true,
    },
    {
      name: "Switch (role=switch)",
      target: el("span", { role: "switch" }),
      want: true,
    },
    {
      name: "Radio chip (role=radio)",
      target: el("span", { role: "radio" }),
      want: true,
    },
    { name: "Slider thumb", target: el("div", { role: "slider" }), want: true },
    {
      name: "contenteditable",
      target: { ...el("div"), isContentEditable: true },
      want: true,
    },
    { name: "plain button", target: el("button"), want: false },
    { name: "body", target: el("body"), want: false },
    { name: "null", target: null, want: false },
    { name: "window-like object", target: {}, want: false },
  ]
  for (const c of cases) {
    test(c.name, () => {
      expect(isFormControlTarget(c.target)).toBe(c.want)
    })
  }
})
