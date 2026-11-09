import { useEffect, useRef, useState, type ReactNode } from "react"
import { useReducedMotion } from "@/lib/motion"
import {
  ArrowLeftIcon,
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
import { Loading } from "@/components/ui/loading"
import { Slider } from "@/components/ui/slider"
import { useBoards, useInvite } from "@/lib/boards"
import { plainLine } from "@/lib/irori"
import { useMedia } from "@/lib/media"
import { go, openSettings, settingsBack, type SettingsSection } from "@/lib/nav"
import { isAndroid } from "@/updater/android"
import { corners, useSettings } from "@/lib/settings"
import { UpdateStatusLine, useKeepUpToDate } from "@/updater"
import { VoiceNote } from "@/components/settings/voice-note"
import { useCallPart } from "@/components/talk/huddle"
import { loadTranscribe, setTranscribe } from "@/lib/call"
import { inWails } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { AssistantService } from "@/lib/host"
import {
  AssistantSwitch,
  Toggle,
  ToggleShape,
} from "@/components/settings/settings-parts"

export const settingsSections: {
  id: SettingsSection
  label: string
  icon: LucideIcon
  hint: string
}[] = [
  {
    id: "agents",
    label: "Agents",
    icon: BotIcon,
    hint: "When Irori starts an agent for you, what it starts with, and what it may do.",
  },
  {
    id: "desks",
    label: "Merge desks",
    icon: GitMergeIcon,
    hint: "One per repo: open PRs from you, your space members and your agents, for review. It never merges.",
  },
  {
    id: "notifications",
    label: "Notifications",
    icon: BellIcon,
    hint: "On this device: whether notifications show, and how.",
  },
  {
    id: "appearance",
    label: "Appearance",
    icon: PaletteIcon,
    hint: "The theme, your own colours, light or dark, and the glass.",
  },
  {
    id: "people",
    label: "People and invites",
    icon: UsersIcon,
    hint: "Who is in your spaces, and joining someone else's.",
  },
  {
    id: "devices",
    label: "Devices and connections",
    icon: LaptopIcon,
    hint: "Your devices, how they connect, and which sides each one runs.",
  },
  {
    id: "app",
    label: "App",
    icon: InfoIcon,
    hint: "Starting and updating Irori on this device, voice and huddles, and thread names.",
  },
]

// Under this the sections and the pane don't both fit side by side.
const compactQuery = "(max-width: 1023.98px)"

const cornerChoices = corners.map((c) => ({ value: c.id, label: c.label }))

// SettingsPage is every setting in one place (cards #132, #1007): a page
// at #settings/<section>, so a section can be linked to and Back works. On
// a wide window the sections are a list down the left beside the chosen
// one. Where that list would squeeze the pane (under 1024 px, so a phone
// and a small window) #settings is the list, and a section opens on its
// own with a way back. A pane mounts only while its section shows, so each
// reads its settings when it's opened.
export function SettingsPage({ tab }: { tab: SettingsSection | "" }) {
  const compact = useMedia(compactQuery)
  // Where the list stands alone, opening a section or going back swaps
  // the whole page, so the focused button goes with it: focus the
  // section's heading, or on the way back the row of the section left.
  const heading = useRef<HTMLHeadingElement>(null)
  const list = useRef<HTMLUListElement>(null)
  const was = useRef(tab)
  useEffect(() => {
    const left = was.current
    was.current = tab
    if (!compact || left === tab) return
    if (tab) heading.current?.focus()
    else
      list.current
        ?.querySelector<HTMLButtonElement>(`[data-section="${left}"]`)
        ?.focus()
  }, [tab, compact])
  if (compact && !tab)
    return (
      <div className="grid min-h-0 flex-1 content-start overflow-y-auto pb-20 mac:overscroll-none">
        <ul
          ref={list}
          className="overflow-hidden p-0! glass-panel"
          aria-label="Settings sections"
        >
          {settingsSections.map((s) => {
            const Icon = s.icon
            return (
              <li key={s.id} className="border-b border-border last:border-b-0">
                <button
                  type="button"
                  data-section={s.id}
                  onClick={() =>
                    go({ kind: "settings", tab: s.id }, { fromList: true })
                  }
                  className="flex w-full items-center gap-3 px-4 py-3 text-left outline-none hover:bg-secondary/50 focus-visible:bg-secondary/50 focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset"
                >
                  <Icon
                    className="size-4 shrink-0 text-muted-foreground"
                    aria-hidden
                  />
                  <span className="grid min-w-0 flex-1 gap-0.5">
                    <span className="text-[13.5px] font-medium text-ink">
                      {s.label}
                    </span>
                    <span className="text-[12px] text-muted-foreground">
                      {s.hint}
                    </span>
                  </span>
                </button>
              </li>
            )
          })}
        </ul>
      </div>
    )
  const here = settingsSections.find((s) => s.id === tab) ?? settingsSections[0]
  return (
    <div className="flex min-h-0 flex-1 gap-4">
      {!compact && (
        <nav
          aria-label="Settings sections"
          className="-mx-1 grid w-[228px] shrink-0 content-start gap-0.5 overflow-y-auto px-1 pt-px pb-6"
        >
          {settingsSections.map((s) => {
            const Icon = s.icon
            const current = s.id === here.id
            return (
              <Button
                key={s.id}
                variant={current ? "secondary" : "ghost"}
                size="sm"
                aria-current={current ? "page" : undefined}
                onClick={() => !current && openSettings(s.id)}
                className="justify-start gap-2"
              >
                <Icon className="size-4" aria-hidden />
                {s.label}
              </Button>
            )
          })}
        </nav>
      )}
      {/* Keyed so the next section starts at its top. */}
      <div
        key={here.id}
        className="grid min-w-0 flex-1 grid-cols-[minmax(0,1fr)] content-start gap-3 overflow-y-auto pb-20 mac:overscroll-none"
      >
        {compact && (
          <div>
            <Button
              size="xs"
              variant="ghost"
              aria-label="Back to Settings sections"
              onClick={settingsBack}
            >
              <ArrowLeftIcon className="size-3.5" aria-hidden />
              Settings
            </Button>
          </div>
        )}
        <section className="grid max-w-3xl min-w-0 grid-cols-[minmax(0,1fr)] gap-3 p-4 glass-panel sm:p-5">
          <div className="grid gap-0.5">
            <h2
              ref={heading}
              tabIndex={-1}
              className="text-base font-semibold text-ink outline-none"
            >
              {here.label}
            </h2>
            <p className="text-[13px] text-muted-foreground">{here.hint}</p>
          </div>
          {here.id === "notifications" && <NotificationsPane />}
          {here.id === "appearance" && <AppearancePane />}
          {here.id === "devices" && <DevicesPanel open />}
          {here.id === "people" && <PeoplePane />}
          {here.id === "agents" && <AgentsPane open />}
          {here.id === "desks" && <DesksPane open />}
          {here.id === "app" && <AppPane />}
        </section>
      </div>
    </div>
  )
}

function NotificationsPane() {
  const s = useSettings(true)
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
  // Sound, volume, corner, confetti and the test are the desktop card's
  // (card #1068): a phone shows only the switch.
  const phone = isAndroid()
  return (
    <Loading key="pane" loading={false} label="Loading notification settings">
      <div className="grid gap-3">
        <Toggle
          spot="notifications.show"
          label="Show notifications"
          hint="Pop-up cards for messages, mentions, held messages, cards waiting on you, and agents asking you to choose. The Notifications page keeps its list either way."
          checked={n.enabled}
          onChange={(v) => s.setNotify({ enabled: v })}
        />
        {!phone && (
          <>
            <Toggle
              spot="notifications.sound"
              label="Sound"
              checked={n.sound}
              disabled={off}
              onChange={(v) => s.setNotify({ sound: v })}
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
            <Toggle
              spot="notifications.confetti"
              label="Confetti on good news"
              hint={
                reducedMotion
                  ? "Off while Reduce Motion is on."
                  : "A burst across the screen when an agent finishes a card."
              }
              checked={n.confetti ?? false}
              disabled={off}
              onChange={(v) => s.setNotify({ confetti: v })}
            />
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
          </>
        )}
        {s.error && <p className="text-xs text-destructive">{s.error}</p>}
      </div>
    </Loading>
  )
}

// PeoplePane is everyone on your boards, board by board, with Invite
// someone on the board's own page (an invite is always to one board), and
// Join with an invite someone sent you. Your own name, colour and picture
// are on your Profile (card #1007).
function PeoplePane() {
  const { boards, error, loaded, reload } = useBoards()
  const { invite, setInvite } = useInvite(true)
  const inviting = !!invite && invite.state !== ""
  return (
    <div className="grid gap-3">
      <p className="text-[13px] text-muted-foreground">
        Your name, colour and picture are on your{" "}
        <Button
          variant="link"
          size="xs"
          className="h-auto px-0 align-baseline"
          onClick={() => go({ kind: "profile" })}
        >
          Profile
        </Button>
        .
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
      {/* "No spaces yet" only once the spaces are read (card #598). */}
      <Loading loading={!loaded} label="Loading your spaces">
        {!loaded ? (
          <BoardRowsShape />
        ) : boards.length === 0 ? (
          <p className="text-[13px] text-muted-foreground">
            No spaces yet. Make one with New or join a space, in the space
            picker at the top of the sidebar, or paste an invite below.
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
                    {b.members
                      .filter((m) => !m.me)
                      .map((m) => plainLine(m.name))
                      .join(", ") || "Just you"}
                  </span>
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => go({ kind: "board", id: b.boardId })}
                >
                  Open
                </Button>
              </li>
            ))}
          </ul>
        )}
      </Loading>
      {loaded && boards.length > 0 && (
        <p className="text-xs text-muted-foreground">
          To invite someone, open the space, then Space settings. Only an owner
          can invite.
        </p>
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
        <div className="grid gap-1.5">
          <h3 className="text-sm font-medium text-ink">Join with an invite</h3>
          <JoinInvite onJoined={setInvite} autoFocus={false} />
        </div>
      )}
    </div>
  )
}

const launchHint = "Irori starts when you sign in to this computer."

// AppPane groups what's left by what it is (card #1007): this device's
// start and updates, voice and huddles, and thread names.
function AppPane() {
  const s = useSettings(true)
  const launchUnknown = s.launchAtLogin === null && s.loading
  const keep = useKeepUpToDate(true)
  return (
    <div className="grid gap-5">
      <section className="grid gap-3" aria-label="This device">
        <h3 className="text-sm font-medium text-ink">This device</h3>
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
        <div className="grid gap-1" data-setting="app.updates">
          <span className="text-ink">Updates</span>
          <UpdateStatusLine />
          {keep.on !== null && (
            <div className="pt-2">
              <Toggle
                label="Keep this device up to date"
                hint="Checks every few hours. When a new version is ready, restarts Irori once no agent is working. If that hasn't happened within a day, it asks you."
                checked={keep.on}
                onChange={(v) => void keep.set(v)}
              />
            </div>
          )}
          {keep.error && (
            <p className="text-xs text-destructive">{keep.error}</p>
          )}
        </div>
      </section>
      <section className="grid gap-3" aria-label="Voice and huddles">
        <h3 className="text-sm font-medium text-ink">Voice and huddles</h3>
        <VoiceNote open />
        <TranscribeSwitch />
        <HuddleNotesSwitch />
        <HuddleChairSwitch />
      </section>
      <section className="grid gap-3" aria-label="Threads">
        <h3 className="text-sm font-medium text-ink">Threads</h3>
        <NamesSwitch />
      </section>
    </div>
  )
}

const transcribeLabel = "Transcribe what I say in huddles"
const transcribeHint =
  "Huddles write down what each person says, each on their own device. Turn this off and your voice isn't written down on any of your devices; you still hear and speak."

// TranscribeSwitch is "Transcribe what I say in huddles" (#656): read when
// its pane opens, on all the person's devices.
function TranscribeSwitch() {
  const on = useCallPart((s) => s.transcribeOn)
  const [read, setRead] = useState(!inWails())
  const [error, setError] = useState("")
  useEffect(() => {
    if (!inWails()) return
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
  }, [])
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
    <div className="grid gap-1" data-setting="app.transcribe">
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

const namesLabel = "Name new threads"
const namesHint =
  "Irori gives a thread you post a short title, and may retitle it once as the talk moves on. Threads shared in a space aren't named. Applies on all your devices. Undo is on the thread; a name you give is never changed."

// NamesSwitch is "Name new threads" (#782): Irori's assistant naming the
// person's threads, on unless they turn it off, on all their devices.
function NamesSwitch() {
  return (
    <AssistantSwitch
      setting="app.names"
      label={namesLabel}
      hint={namesHint}
      loading="Loading thread names"
      read={AssistantService.Names}
      write={AssistantService.NamesSet}
    />
  )
}

// HuddleNotesSwitch and HuddleChairSwitch are the two huddle switches (#1053):
// on unless the person turns them off, on all their devices.
function HuddleNotesSwitch() {
  return (
    <AssistantSwitch
      setting="app.huddle-notes"
      label="Let my assistant add notes from what I say"
      hint="In a huddle, your assistant turns what you say into questions, decisions and tasks on the canvas. Off, it adds none of its own; it still answers when you or someone calls it, and when a question matches your work. Applies on all your devices."
      loading="Loading huddle notes"
      read={AssistantService.HuddleNotes}
      write={AssistantService.HuddleNotesSet}
    />
  )
}

function HuddleChairSwitch() {
  return (
    <AssistantSwitch
      setting="app.huddle-chair"
      label="Let my assistant chair huddles I start"
      hint="The chair writes the huddle's title and, at the end, its summary with the action items. Off, the next person's assistant chairs. Applies on all your devices."
      loading="Loading huddle chair"
      read={AssistantService.HuddleChair}
      write={AssistantService.HuddleChairSet}
    />
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

// BoardRowsShape and NotificationsShape stand in while a pane's first read
// is on its way (card #598): plain markup in the shape of the boards list
// and the Notifications pane, with no hooks, for Loading to measure.
// skullet-block draws a control's stand-in as one block, as the control
// would be.
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
        hint="Pop-up cards for messages, mentions, held messages, cards waiting on you, and agents asking you to choose. The Notifications page keeps its list either way."
      />
      <ToggleShape label="Sound" />
      <div className="flex items-center justify-between gap-3">
        <span className="text-ink">Volume</span>
        <span className="block h-3.5 w-full rounded-full" />
      </div>
      <div className="flex items-center justify-between gap-3">
        <span className="text-ink">Corner</span>
        <span skullet-block="" className="block h-7 w-[91px] rounded-lg" />
      </div>
      <ToggleShape
        label="Confetti on good news"
        hint="A burst across the screen when an agent finishes a card."
      />
      <span skullet-block="" className="block h-8 w-[152px] rounded-lg" />
    </div>
  )
}
