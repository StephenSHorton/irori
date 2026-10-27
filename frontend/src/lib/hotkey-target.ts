// Single-key app hotkeys (the theme's "d") must not fire while focus sits in
// a form control. Native inputs, textareas and selects are covered by tag;
// the Base UI controls that replaced them are not native elements, so they
// are matched by role: the Select trigger is a <button role=combobox>, its
// popup a listbox of options, a Switch a <span role=switch>, a RadioGroup
// item a <span role=radio>, and a Slider thumb carries role=slider.
export const FORM_CONTROL_SELECTOR = [
  "input",
  "textarea",
  "select",
  "[contenteditable='true']",
  "[role='combobox']",
  "[role='listbox']",
  "[role='option']",
  "[role='switch']",
  "[role='radio']",
  "[role='radiogroup']",
  "[role='slider']",
].join(", ")

type ClosestTarget = {
  isContentEditable?: boolean
  closest: (selector: string) => unknown
}

export function isFormControlTarget(target: unknown): boolean {
  if (
    typeof target !== "object" ||
    target === null ||
    typeof (target as ClosestTarget).closest !== "function"
  ) {
    return false
  }
  const el = target as ClosestTarget
  if (el.isContentEditable) {
    return true
  }
  return el.closest(FORM_CONTROL_SELECTOR) != null
}
