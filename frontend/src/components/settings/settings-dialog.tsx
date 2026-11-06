import { useId, useRef, useState, type ReactNode } from "react"
import { useReducedMotion } from "@/lib/motion"
import {
  BellIcon,
  BotIcon,
  GitMergeIcon,
  InfoIcon,
  LaptopIcon,
  PaletteIcon,
  Trash2Icon,
  UsersIcon,
  type LucideIcon,
} from "lucide-react"

import { InvitePanel, JoinInvite } from "@/components/boards/parts"
import { ChoiceSelect } from "@/components/common/choice-select"
import { DevicesPanel } from "@/components/settings/devices-panel"
import { AgentsPane } from "@/components/settings/agents-pane"
import { DesksPane } from "@/components/settings/desks-pane"
import { AppearancePane } from "@/components/settings/appearance-pane"
import { MaintenancePane } from "@/components/settings/maintenance-pane"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog"
import { Slider } from "@/components/ui/slider"
import { Switch } from "@/components/ui/switch"
import { useBoards, useInvite } from "@/lib/boards"
import { plainLine } from "@/lib/irori"
import { go } from "@/lib/nav"
import { corners, useSettings, useYourName } from "@/lib/settings"
import { UpdateStatusLine, useKeepUpToDate } from "@/updater"
import { VoiceNote } from "@/components/settings/voice-note"

export type SettingsSection =
  | "notifications"
  | "appearance"
  | "devices"
  | "people"
  | "agents"
  | "desks"
  | "maintenance"
  | "app"

export const settingsSections: {
  id: SettingsSection
  label: string
  icon: LucideIcon
  hint: string
}[] = [
  {
    id: "notifications",
    label: "Notifications",
    icon: BellIcon,
    hint: "What Irori tells you about, and how.",
  },
  {
    id: "appearance",
    label: "Appearance",
    icon: PaletteIcon,
    hint: "The theme, your own colours, light or dark, and the glass.",
  },
  {
    id: "devices",
    label: "Devices and connections",
    icon: LaptopIcon,
    hint: "Your devices share one inbox and one Board.",
  },
  {
    id: "people",
    label: "People and invites",
    icon: UsersIcon,
    hint: "Who is in your spaces, and joining someone else's.",
  },
  {
    id: "agents",
    label: "Agents",
    icon: BotIcon,
    hint: "Agents Irori starts for your cards, and what they start with.",
  },
  {
    id: "desks",
    label: "Merge desks",
    icon: GitMergeIcon,
    hint: "One per repo: open PRs from GitHub, merged in turn by one agent.",
  },
  {
    id: "maintenance",
    label: "Maintenance",
    icon: Trash2Icon,
    hint: "Clear out done cards.",
  },
  {
    id: "app",
    label: "App",
    icon: InfoIcon,
    hint: "Starting Irori, voice input, updates and the version.",
  },
]

const cornerChoices = corners.map((c) => ({ value: c.id, label: c.label }))

// SettingsDialog is every setting in one place (card #132): a section list
// down the left, the chosen section on the right. The header gear opens
// it; ?settings=<section> opens it in the browser preview.
export function SettingsDialog({
  open,
  onOpenChange,
  section,
  onSection,
  version,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  section: SettingsSection
  onSection: (s: SettingsSection) => void
  version?: string
}) {
  const here =
    settingsSections.find((s) => s.id === section) ?? settingsSections[0]
  // Focus starts on the open section's button, not the first one.
  const currentRef = useRef<HTMLButtonElement>(null)
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        initialFocus={currentRef}
        // Under 768 px it takes the whole screen, inside the phone's insets
        // (card #479); from 768 a 640-high panel over the window.
        className="h-dvh max-w-none grid-cols-[minmax(0,1fr)] grid-rows-[auto_minmax(0,1fr)] gap-0 p-0 pt-[var(--safe-top)] pr-[var(--safe-right)] pb-[var(--safe-bottom)] pl-[var(--safe-left)] sm:max-w-none sm:grid-cols-[13rem_minmax(0,1fr)] sm:grid-rows-[minmax(0,1fr)] md:h-[min(640px,calc(100vh-4rem))] md:max-w-3xl md:p-0 max-md:[&>[data-slot=dialog-close]]:top-[calc(var(--safe-top)+0.75rem)] max-md:[&>[data-slot=dialog-close]]:right-[calc(var(--safe-right)+0.75rem)]"
      >
        <nav
          aria-label="Settings sections"
          className="flex flex-row flex-wrap gap-1 border-b border-border p-3 pr-12 sm:flex-col sm:flex-nowrap sm:gap-0.5 sm:border-r sm:border-b-0 sm:pr-3"
        >
          <DialogTitle className="w-full px-2 pt-1 pb-2 text-base sm:pb-3">
            Settings
          </DialogTitle>
          {settingsSections.map((s) => {
            const Icon = s.icon
            const current = s.id === here.id
            return (
              <Button
                key={s.id}
                ref={current ? currentRef : undefined}
                variant={current ? "secondary" : "ghost"}
                size="sm"
                aria-current={current ? "page" : undefined}
                onClick={() => onSection(s.id)}
                className="justify-start gap-2"
              >
                <Icon className="size-4" aria-hidden />
                {s.label}
              </Button>
            )
          })}
        </nav>
        <section className="flex min-h-0 flex-col gap-3 overflow-y-auto p-4 sm:p-5 sm:pr-12">
          <div className="grid gap-0.5">
            <h2 className="text-base font-semibold text-ink">{here.label}</h2>
            <DialogDescription>{here.hint}</DialogDescription>
          </div>
          {here.id === "notifications" && <NotificationsPane open={open} />}
          {here.id === "appearance" && <AppearancePane />}
          {here.id === "devices" && <DevicesPanel open={open} />}
          {here.id === "people" && (
            <PeoplePane onOpenBoard={() => onOpenChange(false)} />
          )}
          {here.id === "agents" && <AgentsPane open={open} />}
          {here.id === "desks" && <DesksPane open={open} />}
          {here.id === "maintenance" && <MaintenancePane />}
          {here.id === "app" && <AppPane open={open} version={version} />}
        </section>
      </DialogContent>
    </Dialog>
  )
}

function NotificationsPane({ open }: { open: boolean }) {
  const s = useSettings(open)
  const reducedMotion = useReducedMotion()
  const n = s.notify
  if (!n) {
    return (
      <p className="text-muted-foreground">
        {s.error ? `Could not load: ${s.error}` : "Loading…"}
      </p>
    )
  }
  const off = !n.enabled
  return (
    <div className="grid gap-3">
      <Toggle
        label="Show notifications"
        hint="Held messages, cards blocked on you, and agents asking you to choose."
        checked={n.enabled}
        onChange={(v) => s.setNotify({ enabled: v })}
      />
      <Toggle
        label="Sound"
        checked={n.sound}
        disabled={off}
        onChange={(v) => s.setNotify({ sound: v })}
      />
      <Toggle
        label="Confetti on good news"
        hint={
          reducedMotion
            ? "Off while Reduce Motion is on."
            : "A burst across the screen when a card is done."
        }
        checked={n.confetti ?? false}
        disabled={off}
        onChange={(v) => s.setNotify({ confetti: v })}
      />
      <Row label="Volume" disabled={off || !n.sound}>
        <Slider
          min={0}
          max={100}
          step={5}
          value={[Math.round(n.volume * 100)]}
          disabled={off || !n.sound}
          thumbLabel="Volume"
          className="w-40"
          onValueChange={(v) =>
            s.setNotify({ volume: (Array.isArray(v) ? v[0] : v) / 100 })
          }
        />
      </Row>
      <Row label="Corner" disabled={off}>
        <ChoiceSelect
          label="Corner"
          size="sm"
          value={n.corner}
          disabled={off}
          choices={cornerChoices}
          onValueChange={(v) => s.setNotify({ corner: v })}
          className="text-[13px]"
        />
      </Row>
      <div className="flex flex-col items-start gap-1">
        <Button
          variant="outline"
          size="sm"
          disabled={off}
          onClick={() => void s.test()}
        >
          Send a test notification
        </Button>
        {s.tested && (
          <p className="text-xs text-muted-foreground">{s.tested}</p>
        )}
      </div>
      {s.error && <p className="text-xs text-destructive">{s.error}</p>}
    </div>
  )
}

// PeoplePane is everyone on your boards, board by board, with Invite
// someone on the board's own page (an invite is always to one board), and
// Join with an invite someone sent you.
function PeoplePane({ onOpenBoard }: { onOpenBoard: () => void }) {
  const { boards, error, reload } = useBoards()
  const { invite, setInvite } = useInvite(true)
  const inviting = !!invite && invite.state !== ""
  return (
    <div className="grid gap-3">
      <YourName />
      {error && <p className="text-xs text-destructive">{error}</p>}
      {boards.length === 0 ? (
        <p className="text-[13px] text-muted-foreground">
          No spaces yet. Make one on the Spaces page, then invite people to it.
        </p>
      ) : (
        <ul className="grid gap-1.5" aria-label="People in your spaces">
          {boards.map((b) => (
            <li
              key={b.boardId}
              className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2"
            >
              <span className="flex min-w-0 flex-col">
                <span className="truncate font-medium text-ink">
                  {plainLine(b.name) || "(unnamed)"}
                  <span className="ml-1.5 font-mono text-xs font-normal text-muted-foreground">
                    {plainLine(b.tag)}#
                  </span>
                </span>
                <span className="truncate text-xs text-muted-foreground">
                  {b.people.map(plainLine).join(", ") || "Just you"}
                </span>
              </span>
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  onOpenBoard()
                  go({ kind: "board", id: b.boardId })
                }}
              >
                {b.owner ? "Invite…" : "Open"}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {inviting ? (
        <div className="border-t border-border pt-3">
          <InvitePanel
            s={invite}
            onClose={() => {
              setInvite(null)
              void reload()
            }}
          />
        </div>
      ) : (
        <JoinInvite onJoined={setInvite} />
      )}
    </div>
  )
}

function AppPane({ open, version }: { open: boolean; version?: string }) {
  const s = useSettings(open)
  const keep = useKeepUpToDate(open)
  return (
    <div className="grid gap-3">
      <Toggle
        label="Launch at login"
        hint="Irori starts in the tray when you sign in. The tray offers it too."
        checked={Boolean(s.launchAtLogin)}
        disabled={s.launchAtLogin === null}
        onChange={(v) => void s.setLaunchAtLogin(v)}
      />
      <VoiceNote open={open} />
      <div className="grid gap-1 border-t border-border pt-3">
        <span className="font-medium text-ink">Updates</span>
        <UpdateStatusLine />
        {keep.on !== null && (
          <div className="pt-2">
            <Toggle
              label="Keep this device up to date"
              hint="Checks every few hours and restarts Irori when no agent is working. If that hasn't happened within a day, it asks you."
              checked={keep.on}
              onChange={(v) => void keep.set(v)}
            />
          </div>
        )}
        {keep.error && (
          <p className="text-xs text-destructive">{keep.error}</p>
        )}
      </div>
      {version && (
        <div className="flex items-center justify-between gap-3 border-t border-border pt-3">
          <span className="text-ink">Version</span>
          <span className="font-mono text-xs text-muted-foreground">
            {version === "dev" || version === "sample"
              ? version
              : `v${version.replace(/^v/, "")}`}
          </span>
        </div>
      )}
    </div>
  )
}

function Toggle({
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  label: string
  hint?: string
  checked: boolean
  disabled?: boolean
  onChange: (v: boolean) => void
}) {
  const id = useId()
  return (
    <div
      className={`flex items-start justify-between gap-3 ${disabled ? "opacity-50" : ""}`}
    >
      <span className="flex flex-col">
        <label
          htmlFor={id}
          className={disabled ? "text-ink" : "cursor-pointer text-ink"}
        >
          {label}
        </label>
        {hint && (
          <span id={`${id}-hint`} className="text-xs text-muted-foreground">
            {hint}
          </span>
        )}
      </span>
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        aria-describedby={hint ? `${id}-hint` : undefined}
        onCheckedChange={(v) => onChange(v)}
        className="mt-0.5"
      />
    </div>
  )
}

function Row({
  label,
  disabled,
  children,
}: {
  label: string
  disabled?: boolean
  children: ReactNode
}) {
  return (
    <div
      className={`flex items-center justify-between gap-3 ${disabled ? "[&>span]:opacity-50" : ""}`}
    >
      <span className="text-ink">{label}</span>
      {children}
    </div>
  )
}

// YourName is the person's one name (card #183): what every board they're
// on shows them as, and their other devices too. Enter or Save keeps it.
function YourName() {
  const { name, error, save } = useYourName(true)
  const [draft, setDraft] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const id = useId()
  const value = draft ?? name ?? ""
  const changed = draft !== null && draft.trim() !== (name ?? "")
  const keep = async () => {
    if (!changed) return
    if (await save(value)) {
      setDraft(null)
      setSaved(true)
    }
  }
  return (
    <div className="grid gap-1.5">
      <label htmlFor={id} className="text-[13px] font-medium text-ink">
        Your name
      </label>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          value={value}
          maxLength={60}
          disabled={name === null}
          onChange={(e) => {
            setDraft(e.target.value)
            setSaved(false)
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") void keep()
            if (e.key === "Escape" && draft !== null) {
              e.stopPropagation()
              setDraft(null)
            }
          }}
          className="h-8 w-64 text-[13px]"
        />
        <Button
          variant="outline"
          size="sm"
          disabled={!changed || !value.trim()}
          onClick={() => void keep()}
        >
          Save
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {error
          ? error
          : saved
            ? "Saved. Your other devices and your spaces pick it up as they sync."
            : "What people in your spaces see you as, on all your devices."}
      </p>
    </div>
  )
}
