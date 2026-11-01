import { useState } from "react"
import { SettingsIcon } from "lucide-react"

import {
  SettingsDialog,
  settingsSections,
  type SettingsSection,
} from "@/components/settings/settings-dialog"
import { Button } from "@/components/ui/button"
import { useCommands } from "@/lib/commands"
import { previewFlag } from "@/lib/preview"

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

  // Settings and each of its sections in the palette (card #213).
  const show = (s: SettingsSection) => {
    setSection(s)
    setOpen(true)
  }
  useCommands([
    {
      id: "settings.open",
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
