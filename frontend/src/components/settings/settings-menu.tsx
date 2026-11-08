import { useEffect, useState } from "react"
import { SettingsIcon } from "lucide-react"

import {
  SettingsDialog,
  settingsSections,
  type SettingsSection,
} from "@/components/settings/settings-dialog"
import { Button } from "@/components/ui/button"
import { PaletteJump } from "@/components/command-palette"
import { useCommands } from "@/lib/commands"
import { SettingsShortcut } from "@/lib/app-shortcuts"
import { previewFlag } from "@/lib/preview"

// OpenSettingsEvent opens Settings at a section from elsewhere in the
// window (the merge desk's gear, card #417): dispatch it as a CustomEvent
// whose detail is the section.
export const OpenSettingsEvent = "irori:open-settings"

export function openSettings(section: SettingsSection) {
  window.dispatchEvent(new CustomEvent(OpenSettingsEvent, { detail: section }))
}

// SettingsMenu is the header's gear: it opens Settings (card #132), every
// setting in one dialog with its sections down the left.
export function SettingsMenu({ version }: { version?: string }) {
  // ?settings[=<section>] and ?devices open it in the browser preview.
  const [section, setSection] = useState<SettingsSection>(() => {
    if (previewFlag("devices") !== null) return "devices"
    const want = previewFlag("settings")
    return settingsSections.find((s) => s.id === want)?.id ?? "notifications"
  })
  const [open, setOpen] = useState(
    () => previewFlag("settings") !== null || previewFlag("devices") !== null
  )

  // A jump from the palette closes Settings so the place shows (#218).
  useEffect(() => {
    const close = () => setOpen(false)
    window.addEventListener(PaletteJump, close)
    return () => window.removeEventListener(PaletteJump, close)
  }, [])
  useEffect(() => {
    const open = (e: Event) => {
      const want = (e as CustomEvent<SettingsSection>).detail
      if (settingsSections.some((s) => s.id === want)) setSection(want)
      setOpen(true)
    }
    window.addEventListener(OpenSettingsEvent, open)
    return () => window.removeEventListener(OpenSettingsEvent, open)
  }, [])
  // Settings and each of its sections in the palette (card #213).
  const show = (s: SettingsSection) => {
    setSection(s)
    setOpen(true)
  }
  useCommands([
    {
      id: "settings.open",
      shortcut: SettingsShortcut,
      title: "Settings",
      group: "Go to",
      order: 4,
      keywords: ["preferences", "options", "open settings"],
      run: () => setOpen(true),
    },
    ...settingsSections.map((s) => ({
      id: `settings.${s.id}`,
      title: `Settings: ${s.label}`,
      group: "Go to",
      quiet: true,
      keywords: ["settings", "preferences"],
      run: () => show(s.id),
    })),
  ])

  return (
    <>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Settings"
        title="Settings"
        onClick={() => setOpen(true)}
      >
        <SettingsIcon />
      </Button>
      <SettingsDialog
        open={open}
        onOpenChange={setOpen}
        section={section}
        onSection={setSection}
        version={version}
      />
    </>
  )
}
