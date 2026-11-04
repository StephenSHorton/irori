// DesksPane is Settings › Merge desks (card #417, docs/design/
// merge-desk.md §7, mockup desk-6): one repo.desk row per repo, set once.
// Each desk names its holder, the device that polls GitHub, the logins it
// trusts besides your agents, its check lines and merge method. Agents
// can't write any of this: only you, here.

import { useMemo, useState } from "react"
import { GitMergeIcon, PlusIcon, Trash2Icon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  deskActions,
  newDesk,
  ownerName,
  useDesks,
  type CheckLine,
  type MergeMethod,
  type RepoDesk,
} from "@/lib/desk"
import { useDevices } from "@/lib/devices"
import { useRoster } from "@/lib/irori"
import { nameOr, unknownAgent } from "@/lib/names"
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
  const t = s.trim().replace(/\.git$/, "").replace(/\/$/, "")
  const m =
    t.match(/^remote:github\.com\/([^/\s]+\/[^/\s]+)$/) ||
    t.match(/^(?:https?:\/\/)?(?:www\.)?github\.com\/([^/\s]+\/[^/\s]+)$/) ||
    t.match(/^git@github\.com:([^/\s]+\/[^/\s]+)$/) ||
    t.match(/^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)$/)
  return m ? `remote:github.com/${m[1]}` : ""
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1 py-1.5 sm:grid-cols-[130px_1fr] sm:gap-3">
      <span className="pt-1.5 text-[12.5px] text-muted-foreground">{label}</span>
      <div className="grid min-w-0 gap-1">
        {children}
        {hint && <span className="text-[11.5px] text-muted-foreground">{hint}</span>}
      </div>
    </div>
  )
}

type Choice = { value: string; label: string }

function DeskEditor({
  start,
  agents,
  devices,
  onDone,
}: {
  start: RepoDesk
  agents: Choice[]
  devices: Choice[]
  onDone: () => void
}) {
  const [d, setD] = useState<RepoDesk>(start)
  const [trusted, setTrusted] = useState(start.trusted.join(", "))
  const [error, setError] = useState("")
  const [saving, setSaving] = useState(false)
  const set = (p: Partial<RepoDesk>) => setD((x) => ({ ...x, ...p }))
  const setCheck = (i: number, p: Partial<CheckLine>) =>
    set({ checks: d.checks.map((c, j) => (j === i ? { ...c, ...p } : c)) })
  const save = async () => {
    setSaving(true)
    setError("")
    try {
      const checks = d.checks
        .map((c) => ({ ...c, command: c.command.trim() }))
        .filter((c) => c.command)
      await deskActions.setDesk({ ...d, trusted: list(trusted), checks })
      onDone()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setSaving(false)
    }
  }
  return (
    <div className="grid gap-0.5 border-t border-border pt-2" data-desk-editor={d.repoKey}>
      <Field label="Takes PRs" hint="Off for a repo whose work merges straight to main: it gets no desk.">
        <Switch checked={d.takesPrs} onCheckedChange={(v) => set({ takesPrs: v })} aria-label="Takes PRs" />
      </Field>
      <Field label="Holder" hint="One of your agents merges for this repo. If it goes, the desk shows unheld and nothing merges.">
        <ChoiceSelect
          label="Holder"
          size="sm"
          value={d.holder || NONE}
          onValueChange={(v) => set({ holder: v === NONE ? undefined : v })}
          choices={[{ value: NONE, label: "Nobody (unheld)" }, ...agents]}
        />
      </Field>
      <Field label="Polls GitHub from" hint="One device asks GitHub for the open PRs every 3 minutes; it stays put when the holder changes.">
        <ChoiceSelect
          label="Poll device"
          size="sm"
          value={d.pollDevice || NONE}
          onValueChange={(v) => set({ pollDevice: v === NONE ? "" : v })}
          choices={[{ value: NONE, label: "Pick a device" }, ...devices]}
        />
      </Field>
      <Field label="Check lines" hint="The first line with no paths always runs. A line with paths runs only when the diff touches them; a runner or device says where.">
        {d.checks.map((c, i) => (
          <div key={i} className="grid gap-1 rounded-md border border-border p-2" data-check={i + 1}>
            <div className="flex items-center gap-2">
              <span className="w-4 text-[12px] text-muted-foreground">{i + 1}</span>
              <Input
                className="h-8 font-mono text-[12px]"
                placeholder="cargo test --workspace"
                value={c.command}
                onChange={(e) => setCheck(i, { command: e.target.value })}
                aria-label={`Check line ${i + 1}`}
              />
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={`Remove check line ${i + 1}`}
                onClick={() => set({ checks: d.checks.filter((_, j) => j !== i) })}
              >
                <Trash2Icon />
              </Button>
            </div>
            <div className="grid gap-1 pl-6 sm:grid-cols-[1fr_150px_120px] sm:gap-2">
              <Input
                className="h-7 font-mono text-[11.5px]"
                placeholder="runs when the diff touches (e.g. app/macos/**), else always"
                value={(c.paths ?? []).join(", ")}
                onChange={(e) => setCheck(i, { paths: list(e.target.value) })}
                aria-label={`Paths for line ${i + 1}`}
              />
              <ChoiceSelect
                label={`Runner for line ${i + 1}`}
                size="sm"
                value={c.runner || NONE}
                onValueChange={(v) => setCheck(i, { runner: v === NONE ? undefined : v })}
                choices={[{ value: NONE, label: "the holder" }, ...agents]}
              />
              <ChoiceSelect
                label={`Device for line ${i + 1}`}
                size="sm"
                value={c.device || NONE}
                onValueChange={(v) => setCheck(i, { device: v === NONE ? undefined : (v as CheckLine["device"]) })}
                choices={[
                  { value: NONE, label: "any OS" },
                  { value: "mac", label: "Mac" },
                  { value: "windows", label: "Windows" },
                  { value: "linux", label: "Linux" },
                ]}
              />
            </div>
          </div>
        ))}
        <Button
          size="xs"
          variant="ghost"
          className="w-fit"
          onClick={() => set({ checks: [...d.checks, { command: "" }] })}
        >
          <PlusIcon /> Add a line
        </Button>
      </Field>
      <Field label="Merge method">
        <ChoiceSelect
          label="Merge method"
          size="sm"
          value={d.mergeMethod}
          onValueChange={(v) => set({ mergeMethod: v as MergeMethod })}
          choices={[
            { value: "squash", label: "Squash" },
            { value: "merge", label: "Merge" },
            { value: "rebase", label: "Rebase" },
          ]}
        />
      </Field>
      <Field label="Trusted authors" hint="GitHub logins whose PRs the desk acts on, besides your agents'. Anyone else's PR waits for your OK on its queue line.">
        <Input
          className="h-8 text-[12.5px]"
          placeholder="your-login, a-collaborator"
          value={trusted}
          onChange={(e) => setTrusted(e.target.value)}
          aria-label="Trusted GitHub logins"
        />
      </Field>
      <Field label="OK each PR" hint="Off: the holder merges a trusted PR once its checks pass. On: it asks you on the queue line first.">
        <Switch checked={d.perPrOk} onCheckedChange={(v) => set({ perPrOk: v })} aria-label="OK each PR" />
      </Field>
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
  const { desks } = useDesks()
  const roster = useRoster()
  const { devices } = useDevices(open)
  const [editing, setEditing] = useState("")
  const [adding, setAdding] = useState("")
  const [error, setError] = useState("")
  const agents = useMemo(
    () =>
      roster.agents
        .filter((a) => a.status !== "gone")
        .map((a) => ({ value: a.key, label: nameOr(a.name) })),
    [roster.agents]
  )
  const deviceChoices = useMemo(
    () => devices.filter((v) => !v.revoked).map((v) => ({ value: v.deviceId, label: v.name || v.deviceId })),
    [devices]
  )
  const nameOf = (key?: string) => (key ? (agents.find((a) => a.value === key)?.label ?? unknownAgent) : "")
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
    shown.unshift({ settings: newDesk(editing, here), holds: {}, oks: {} })
  }
  return (
    <div className="grid gap-3">
      <p className="text-[12.5px] text-muted-foreground">
        One desk per repo, set once. Open PRs come from GitHub; one agent merges them in turn under the
        repo&apos;s checks. Only you change these: agents can&apos;t. Tags and releases are never the
        desk&apos;s.
      </p>
      {shown.map((d) => {
        const s = d.settings
        const on = s.enabled && s.takesPrs
        const open = editing === s.repoKey
        return (
          <section
            key={s.repoKey}
            className={cn(
              "grid gap-1 rounded-xl border px-4 py-2.5",
              on ? "border-border bg-card" : "border-dashed border-border opacity-85"
            )}
          >
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <GitMergeIcon className="size-4 text-ink" />
              <span className="font-mono text-[13.5px] text-ink">{ownerName(s.repoKey)}</span>
              <span className="text-[12px] text-muted-foreground">
                {!s.takesPrs
                  ? "No desk: this repo takes no PRs."
                  : [
                      s.holder ? `held by ${nameOf(s.holder)}` : "unheld",
                      `${s.checks.length} check line${s.checks.length === 1 ? "" : "s"}`,
                      s.mergeMethod,
                    ].join(" · ")}
              </span>
              <span className="ml-auto flex items-center gap-2 text-[12.5px] text-ink-2">
                {!open && (
                  <Button size="xs" variant="ghost" onClick={() => setEditing(s.repoKey)}>
                    Edit
                  </Button>
                )}
                Desk {s.enabled ? "on" : "off"}
                <Switch
                  checked={s.enabled}
                  disabled={!s.takesPrs && !s.enabled}
                  aria-label={`Desk for ${ownerName(s.repoKey)}`}
                  onCheckedChange={(v) =>
                    deskActions
                      .setDesk({ ...s, enabled: v, pollDevice: s.pollDevice || here })
                      .catch((e: unknown) => setError(errorText(e)))
                  }
                />
              </span>
            </div>
            {open && (
              <DeskEditor key={s.at ?? "new"} start={s} agents={agents} devices={deviceChoices} onDone={() => setEditing("")} />
            )}
          </section>
        )
      })}
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
