// DesksPane is Settings › Merge desks (card #417, docs/design/
// merge-desk.md §7, mockup desk-6). A GitHub repo the person or their
// space members open PRs in has a desk by itself (card #521, #625): on,
// polled from the device that saw the repo last. The desk never merges
// (card #418): it lists PRs for review, and each agent asks the person
// "Can I merge PR #N?" on its own card. Here the person turns a desk off,
// picks the device that polls GitHub, and names the logins whose PRs also
// count as ours. Agents can't write any of this: only you, here.

import { useMemo, useState } from "react"
import { GitMergeIcon, PlusIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Loading } from "@/components/ui/loading"
import { Switch } from "@/components/ui/switch"
import {
  deskActions,
  newDesk,
  ownerName,
  useDesks,
  type RepoDesk,
} from "@/lib/desk"
import { useDevices } from "@/lib/devices"
import { useRoster } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

const NONE = "-"

function list(s: string): string[] {
  return s
    .split(/[,\s]+/)
    .map((x) => x.trim())
    .filter(Boolean)
}

// githubKey makes "owner/name", a GitHub URL or a repo key into the repo
// key a desk is stored under, or "" when it isn't a GitHub repo.
export function githubKey(s: string): string {
  const t = s
    .trim()
    .replace(/\.git$/, "")
    .replace(/\/$/, "")
  const m =
    t.match(/^remote:github\.com\/([^/\s]+\/[^/\s]+)$/) ||
    t.match(/^(?:https?:\/\/)?(?:www\.)?github\.com\/([^/\s]+\/[^/\s]+)$/) ||
    t.match(/^git@github\.com:([^/\s]+\/[^/\s]+)$/) ||
    t.match(/^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)$/)
  return m ? `remote:github.com/${m[1]}` : ""
}

function Field({
  label,
  hint,
  children,
}: {
  label: string
  hint?: string
  children: React.ReactNode
}) {
  return (
    <div className="grid gap-1 py-1.5 sm:grid-cols-[130px_1fr] sm:gap-3">
      <span className="pt-1.5 text-[12.5px] text-muted-foreground">
        {label}
      </span>
      <div className="grid min-w-0 gap-1">
        {children}
        {hint && (
          <span className="text-[11.5px] text-muted-foreground">{hint}</span>
        )}
      </div>
    </div>
  )
}

type Choice = { value: string; label: string }

function DeskEditor({
  start,
  devices,
  follow,
  onDone,
}: {
  start: RepoDesk
  devices: Choice[]
  // The device that saw the repo last, by name: a blank poll device
  // follows it (card #521). "" when no device has seen the repo.
  follow: string
  onDone: () => void
}) {
  const [d, setD] = useState<RepoDesk>(start)
  const [trusted, setTrusted] = useState(start.trusted.join(", "))
  const [error, setError] = useState("")
  const [saving, setSaving] = useState(false)
  const set = (p: Partial<RepoDesk>) => setD((x) => ({ ...x, ...p }))
  const save = async () => {
    setSaving(true)
    setError("")
    try {
      await deskActions.setDesk({ ...d, trusted: list(trusted) })
      onDone()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setSaving(false)
    }
  }
  return (
    <div
      className="grid gap-0.5 border-t border-border pt-2"
      data-desk-editor={d.repoKey}
    >
      <Field
        label="Takes PRs"
        hint="Off for a repo whose work merges straight to main: it gets no desk."
      >
        <Switch
          checked={d.takesPrs}
          onCheckedChange={(v) => set({ takesPrs: v })}
          aria-label="Takes PRs"
        />
      </Field>
      <Field
        label="Polls GitHub from"
        hint="One device asks GitHub for the open PRs every 3 minutes."
      >
        <ChoiceSelect
          label="Poll device"
          size="sm"
          value={d.pollDevice || NONE}
          onValueChange={(v) => set({ pollDevice: v === NONE ? "" : v })}
          choices={[
            {
              value: NONE,
              label: follow
                ? `Last device to start work in it (${follow})`
                : "Pick a device",
            },
            ...devices,
          ]}
        />
      </Field>
      <Field
        label="Trusted authors"
        hint="GitHub logins whose PRs also show on this desk, beside yours, your space members' and your agents'. Anyone else's PR isn't listed."
      >
        <Input
          className="h-8 text-[12.5px]"
          placeholder="a-collaborator, another-login"
          value={trusted}
          onChange={(e) => setTrusted(e.target.value)}
          aria-label="Trusted GitHub logins"
        />
      </Field>
      <p className="text-xs text-muted-foreground">
        A desk you add stays only if you or your space members own the repo or
        open PRs in it.
      </p>
      {error && (
        <p className="text-[12px] text-destructive" role="alert">
          {error}
        </p>
      )}
      <div className="flex gap-2 pt-1">
        <Button size="sm" onClick={() => void save()} disabled={saving}>
          Save
        </Button>
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </div>
  )
}

export function DesksPane({ open }: { open: boolean }) {
  const { desks, loaded } = useDesks()
  const roster = useRoster()
  const { devices } = useDevices(open)
  const [editing, setEditing] = useState("")
  const [adding, setAdding] = useState("")
  const [error, setError] = useState("")
  const deviceChoices = useMemo(
    () =>
      devices
        .filter((v) => !v.revoked)
        .map((v) => ({ value: v.deviceId, label: v.name || v.deviceId })),
    [devices]
  )
  const deviceName = (id: string) =>
    deviceChoices.find((c) => c.value === id)?.label ?? "another device"
  const here = roster.device?.id ?? ""
  const add = () => {
    const key = githubKey(adding)
    if (!key) {
      setError("Give a GitHub repo as owner/name or its URL.")
      return
    }
    setError("")
    setAdding("")
    setEditing(key)
  }
  const shown = [...desks]
  if (editing && !shown.some((d) => d.settings.repoKey === editing)) {
    shown.unshift({ settings: newDesk(editing, here) })
  }
  return (
    <div className="grid gap-3">
      <p className="text-[12.5px] text-muted-foreground">
        A GitHub repo your agents work in gets a desk by itself, if you or your
        space members own it or open PRs in it. It lists open PRs from you, your
        space members and your agents, and shows while one is open, for review.
        The desk never merges: each agent asks you &ldquo;Can I merge PR
        #N?&rdquo; on its own card and merges on your yes. Your desks are your
        own, on all your devices: your space members have their own, and agents
        can&apos;t change yours.
      </p>
      {/* The desks wait on the first read (card #598), rather than popping
          in above the form. With no desks it takes no row, as before. */}
      <Loading
        loading={!loaded}
        label="Loading merge desks"
        className="not-has-[section]:hidden"
      >
        {!loaded ? (
          <DesksShape />
        ) : (
          <div className="grid gap-3">
            {shown.map((d) => {
              const s = d.settings
              const on = s.enabled && s.takesPrs
              const open = editing === s.repoKey
              // A poll device that follows where the repo was seen last (the
              // default's, or a saved blank one) is saved blank so it keeps
              // following.
              const start =
                d.isDefault || d.followsSeen ? { ...s, pollDevice: "" } : s
              const follow = d.seenOn ? deviceName(d.seenOn) : ""
              return (
                <section
                  key={s.repoKey}
                  className={cn(
                    "grid gap-1 rounded-xl border px-4 py-2.5",
                    on
                      ? "border-border bg-card"
                      : "border-dashed border-border opacity-85"
                  )}
                >
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <GitMergeIcon className="size-4 text-ink" />
                    <span className="font-mono text-[13.5px] text-ink">
                      {ownerName(s.repoKey)}
                    </span>
                    <span className="text-[12px] text-muted-foreground">
                      {!s.takesPrs
                        ? "No desk: this repo takes no PRs."
                        : d.isDefault
                          ? "on by itself"
                          : "your settings"}
                    </span>
                    <span className="ml-auto flex items-center gap-2 text-[12.5px] text-ink-2">
                      {!open && (
                        <Button
                          size="xs"
                          variant="ghost"
                          onClick={() => setEditing(s.repoKey)}
                        >
                          Edit
                        </Button>
                      )}
                      Desk {on ? "on" : "off"}
                      <Switch
                        checked={on}
                        disabled={!s.takesPrs}
                        aria-label={`Desk for ${ownerName(s.repoKey)}`}
                        onCheckedChange={(v) =>
                          deskActions
                            .setDesk({
                              ...start,
                              enabled: v,
                              pollDevice:
                                start.pollDevice || (d.seenOn ? "" : here),
                            })
                            .catch((e: unknown) => setError(errorText(e)))
                        }
                      />
                    </span>
                  </div>
                  {open && (
                    <DeskEditor
                      key={s.at ?? "new"}
                      start={start}
                      devices={deviceChoices}
                      follow={follow}
                      onDone={() => setEditing("")}
                    />
                  )}
                </section>
              )
            })}
          </div>
        )}
      </Loading>
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          add()
        }}
      >
        <Input
          className="h-8 max-w-[320px] text-[12.5px]"
          placeholder="owner/name or a GitHub URL"
          value={adding}
          onChange={(e) => setAdding(e.target.value)}
          aria-label="Repo for a new desk"
        />
        <Button size="sm" variant="outline" type="submit">
          <PlusIcon /> Add a desk
        </Button>
      </form>
      {error && (
        <p className="text-[12px] text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}

// DesksShape is the desks' skeleton (card #598): plain markup in the shape
// of two desk rows, with no hooks, for Loading to measure (skullet-block:
// one block for a control).
function DesksShape() {
  const desks = [
    ["sample-org/app-repo", "your settings"],
    ["sample-org/game-repo", "on by itself"],
  ]
  return (
    <div className="grid gap-3">
      {desks.map(([repo, line]) => (
        <section
          key={repo}
          className="grid gap-1 rounded-xl border border-border px-4 py-2.5"
        >
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span skullet-block="" className="block size-4 rounded-sm" />
            <span className="font-mono text-[13.5px] text-ink">{repo}</span>
            <span className="text-[12px] text-muted-foreground">{line}</span>
            <span className="ml-auto flex items-center gap-2 text-[12.5px] text-ink-2">
              <span className="block h-6 w-9 rounded-lg" />
              Desk on
              <span skullet-block="" className="block h-5 w-9 rounded-full" />
            </span>
          </div>
        </section>
      ))}
    </div>
  )
}
