import { SettingsIcon } from "lucide-react"

import { settingsSections } from "@/components/settings/settings-page"
import { Button } from "@/components/ui/button"
import { SettingsShortcut } from "@/lib/app-shortcuts"
import { useCommands } from "@/lib/commands"
import { openSettings } from "@/lib/nav"

// SettingsMenu is the sidebar's gear: it goes to Settings (card #132; a
// page since card #1007). onGo closes a phone's drawer, which going to
// where the window already is doesn't do by itself.
export function SettingsMenu({
  current,
  onGo,
}: {
  current: boolean
  onGo?: () => void
}) {
  // Settings and each of its sections in the palette (card #213).
  useCommands([
    {
      id: "settings.open",
      title: "Settings",
      group: "Go to",
      order: 4,
      keywords: ["preferences", "options", "open settings"],
      shortcut: SettingsShortcut,
      run: () => openSettings(),
    },
    ...settingsSections.map((s) => ({
      id: `settings.${s.id}`,
      title: `Settings: ${s.label}`,
      group: "Go to",
      quiet: true,
      keywords: ["settings", "preferences"],
      run: () => openSettings(s.id),
    })),
  ])

  return (
    <Button
      variant={current ? "secondary" : "ghost"}
      size="icon-sm"
      aria-label="Settings"
      aria-current={current ? "page" : undefined}
      title="Settings"
      onClick={() => {
        onGo?.()
        openSettings()
      }}
    >
      <SettingsIcon />
    </Button>
  )
}
