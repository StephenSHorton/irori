import { useEffect, useId, useRef, useState, type ReactNode } from "react"
import { useReducedMotion } from "@/lib/motion"
import {
  BellIcon,
  BotIcon,
  GitMergeIcon,
  InfoIcon,
  LaptopIcon,
  PaletteIcon,
  UsersIcon,
  type LucideIcon,
} from "lucide-react"

import { InvitePanel, JoinInvite } from "@/components/boards/parts"
import { ChoiceSelect } from "@/components/common/choice-select"
import { DevicesPanel } from "@/components/settings/devices-panel"
import { AgentsPane } from "@/components/settings/agents-pane"
import { DesksPane } from "@/components/settings/desks-pane"
import { AppearancePane } from "@/components/settings/appearance-pane"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Loading } from "@/components/ui/loading"
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
import { useCallPart } from "@/components/talk/huddle"
import { loadTranscribe, setTranscribe } from "@/lib/call"
import { inWails } from "@/lib/irori"
import { errorText } from "@/lib/talk"

export type SettingsSection =
  | "notifications"
  | "appearance"
  | "devices"
  | "people"
  | "agents"
  | "desks"
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
    hint: "One per repo: your open PRs from GitHub, for review. It never merges.",
  },
  {
    id: "app",
    label: "App",
    icon: InfoIcon,
    hint: "Starting Irori, voice input, huddle transcripts, updates and the version.",
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
      <Loading
        key="pane"
        loading={s.loading}
        label="Loading notification settings"
        error={
          s.error && (
            <p className="text-muted-foreground">Could not load: {s.error}</p>
          )
        }
      >
        <NotificationsShape />
      </Loading>
    )
  }
  const off = !n.enabled
  return (
    <Loading key="pane" loading={false} label="Loading notification settings">
      <div className="grid gap-3">
        <Toggle
          spot="notifications.show"
          label="Show notifications"
          hint="Held messages, cards blocked on you, and agents asking you to choose."
          checked={n.enabled}
          onChange={(v) => s.setNotify({ enabled: v })}
        />
        <Toggle
          spot="notifications.sound"
          label="Sound"
          checked={n.sound}
          disabled={off}
          onChange={(v) => s.setNotify({ sound: v })}
        />
        <Toggle
          spot="notifications.confetti"
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
        <Row
          label="Volume"
          disabled={off || !n.sound}
          spot="notifications.volume"
        >
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
        <Row label="Corner" disabled={off} spot="notifications.corner">
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
    </Loading>
  )
}

// PeoplePane is everyone on your boards, board by board, with Invite
// someone on the board's own page (an invite is always to one board), and
// Join with an invite someone sent you.
function PeoplePane({ onOpenBoard }: { onOpenBoard: () => void }) {
  const { boards, error, loaded, reload } = useBoards()
  const { invite, setInvite } = useInvite(true)
  const inviting = !!invite && invite.state !== ""
  return (
    <div className="grid gap-3">
      <YourName />
      {error && <p className="text-xs text-destructive">{error}</p>}
      {/* "No spaces yet" only once the spaces are read (card #598). */}
      <Loading loading={!loaded} label="Loading your spaces">
        {!loaded ? (
          <BoardRowsShape />
        ) : boards.length === 0 ? (
          <p className="text-[13px] text-muted-foreground">
            No spaces yet. Make one with New or join a space, in the space
            picker at the top of the sidebar, then invite people to it.
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
      </Loading>
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

const launchHint =
  "Irori starts in the tray when you sign in. The tray offers it too."

function AppPane({ open, version }: { open: boolean; version?: string }) {
  const s = useSettings(open)
  const launchUnknown = s.launchAtLogin === null && s.loading
  const keep = useKeepUpToDate(open)
  return (
    <div className="grid gap-3">
      {/* Until it's read, a skeleton rather than a switch shown off (card
          #598); a failed read leaves the switch off and disabled. */}
      <Loading loading={launchUnknown} label="Loading Launch at login">
        {launchUnknown ? (
          <ToggleShape label="Launch at login" hint={launchHint} />
        ) : (
          <Toggle
            spot="app.login"
            label="Launch at login"
            hint={launchHint}
            checked={Boolean(s.launchAtLogin)}
            disabled={s.launchAtLogin === null}
            onChange={(v) => void s.setLaunchAtLogin(v)}
          />
        )}
      </Loading>
      <VoiceNote open={open} />
      <TranscribeSwitch open={open} />
      <div
        className="grid gap-1 border-t border-border pt-3"
        data-setting="app.updates"
      >
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
        {keep.error && <p className="text-xs text-destructive">{keep.error}</p>}
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

const transcribeLabel = "Transcribe what I say in calls"
const transcribeHint =
  "Huddles write down what each person says, on their own device. Turn this off and your voice isn't written down; you still hear and speak."

// TranscribeSwitch is "Transcribe what I say in calls" (#656): read when
// Settings opens, on all the person's devices.
function TranscribeSwitch({ open }: { open: boolean }) {
  const on = useCallPart((s) => s.transcribeOn)
  const [read, setRead] = useState(!inWails())
  const [error, setError] = useState("")
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    loadTranscribe()
      .then(() => live && setRead(true))
      .catch((err: unknown) => {
        if (!live) return
        setError(errorText(err))
        setRead(true)
      })
    return () => {
      live = false
    }
  }, [open])
  const set = async (v: boolean) => {
    setError("")
    try {
      await setTranscribe(v)
    } catch (err) {
      setError(errorText(err))
      await loadTranscribe().catch(() => {})
    }
  }
  return (
    <div
      className="grid gap-1 border-t border-border pt-3"
      data-setting="app.transcribe"
    >
      <Loading loading={!read} label="Loading huddle transcripts">
        {!read ? (
          <ToggleShape label={transcribeLabel} hint={transcribeHint} />
        ) : (
          <Toggle
            label={transcribeLabel}
            hint={transcribeHint}
            checked={on}
            onChange={(v) => void set(v)}
          />
        )}
      </Loading>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}

// spot is the row's key for Ask Irori's "take me there" (card #784,
// lib/goto.ts settingSpots): data-setting, which it highlights.
function Toggle({
  label,
  hint,
  checked,
  disabled,
  onChange,
  spot,
}: {
  label: string
  hint?: string
  checked: boolean
  disabled?: boolean
  onChange: (v: boolean) => void
  spot?: string
}) {
  const id = useId()
  return (
    <div
      data-setting={spot}
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
  spot,
}: {
  label: string
  disabled?: boolean
  children: ReactNode
  spot?: string
}) {
  return (
    <div
      data-setting={spot}
      className={`flex items-center justify-between gap-3 ${disabled ? "[&>span]:opacity-50" : ""}`}
    >
      <span className="text-ink">{label}</span>
      {children}
    </div>
  )
}

// ToggleShape, BoardRowsShape and NotificationsShape stand in while a
// pane's first read is on its way (card #598): plain markup in the shape of
// Toggle, the boards list and the Notifications pane, with no hooks, for
// Loading to measure. skullet-block draws a control's stand-in as one
// block, as the control would be.
function ToggleShape({ label, hint }: { label: string; hint?: string }) {
  return (
    <div className="flex items-start justify-between gap-3">
      <span className="flex flex-col">
        <span className="text-ink">{label}</span>
        {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
      </span>
      <span
        skullet-block=""
        className="mt-0.5 block h-5 w-9 shrink-0 rounded-full"
      />
    </div>
  )
}

function BoardRowsShape() {
  const rows = [
    ["Sample space", "sam", "Stephen, sample-heron, Casey"],
    ["Website refresh", "site", "Stephen"],
  ]
  return (
    <ul className="grid gap-1.5">
      {rows.map(([name, tag, people]) => (
        <li
          key={tag}
          className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2"
        >
          <span className="flex min-w-0 flex-col">
            <span className="font-medium text-ink">
              {name}
              <span className="ml-1.5 font-mono text-xs font-normal text-muted-foreground">
                {tag}#
              </span>
            </span>
            <span className="text-xs text-muted-foreground">{people}</span>
          </span>
          <span skullet-block="" className="block h-8 w-[66px] rounded-lg" />
        </li>
      ))}
    </ul>
  )
}

function NotificationsShape() {
  return (
    <div className="grid gap-3">
      <ToggleShape
        label="Show notifications"
        hint="Held messages, cards blocked on you, and agents asking you to choose."
      />
      <ToggleShape label="Sound" />
      <ToggleShape
        label="Confetti on good news"
        hint="A burst across the screen when a card is done."
      />
      <div className="flex items-center justify-between gap-3">
        <span className="text-ink">Volume</span>
        <span className="block h-3.5 w-full rounded-full" />
      </div>
      <div className="flex items-center justify-between gap-3">
        <span className="text-ink">Corner</span>
        <span skullet-block="" className="block h-7 w-[91px] rounded-lg" />
      </div>
      <span skullet-block="" className="block h-8 w-[152px] rounded-lg" />
    </div>
  )
}

// YourName is the person's one name (card #183): what every board they're
// on shows them as, and their other devices too. Enter or Save keeps it.
function YourName() {
  const { name, error, loading, save } = useYourName(true)
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
      <Loading loading={loading} label="Loading your name">
        {loading ? (
          <div className="flex items-center gap-2">
            <span skullet-block="" className="block h-8 w-64 rounded-lg" />
            <span skullet-block="" className="block h-8 w-14 rounded-lg" />
          </div>
        ) : (
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
        )}
      </Loading>
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
