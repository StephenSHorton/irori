// PermissionsSection is Settings › Agents' "What a new agent may do" (card
// #449, docs/design/spawned-agents.md §7 "Permission mode" and "Merge and
// push policy, per repo", decision 8): the board's choice, the extra
// allow and never-allow rules, and whether each repo's agents open a PR
// or push to main. Only you set these, here; no agent's tool can.

import { useEffect, useId, useState } from "react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import { Loading } from "@/components/ui/loading"
import { Textarea } from "@/components/ui/textarea"
import {
  BYPASS,
  choiceOf,
  LIMITED,
  MERGE_MAIN,
  MERGE_PR,
  permissionChoices,
  rulesOf,
  usePermissions,
} from "@/lib/agent-permissions"
import { cards, type AskRepo } from "@/lib/cards"
import { shortRepo } from "@/lib/irori"

const mergeChoices = [
  { value: MERGE_PR, label: "Opens a PR" },
  { value: MERGE_MAIN, label: "Pushes to main" },
]

export function PermissionsSection({
  open,
  boardId = "",
}: {
  open: boolean
  boardId?: string
}) {
  const { permissions: p, error, loading, save } = usePermissions(open, boardId)
  // Unsaved edits to the rule boxes; null shows what's saved.
  const [allowDraft, setAllow] = useState<string | null>(null)
  const [denyDraft, setDeny] = useState<string | null>(null)
  const [repos, setRepos] = useState<AskRepo[]>([])
  const id = useId()
  useEffect(() => {
    if (!open) return
    let live = true
    cards.askRepos().then(
      (r) => live && setRepos(r),
      () => live && setRepos([])
    )
    return () => {
      live = false
    }
  }, [open])
  // Until the first read lands, a skeleton in the section's shape rather
  // than nothing (card #598).
  if (!p) {
    return (
      <Loading
        key="section"
        loading={loading}
        label="Loading agent permissions"
        error={
          error && (
            <p className="text-[12.5px] text-muted-foreground">
              Could not load what a new agent may do: {error}
            </p>
          )
        }
      >
        <PermissionsShape />
      </Loading>
    )
  }
  const choice = choiceOf(p)
  const allow = allowDraft ?? p.allow.join("\n")
  const deny = denyDraft ?? p.deny.join("\n")
  const dirty = allow !== p.allow.join("\n") || deny !== p.deny.join("\n")
  const keys = new Set([
    ...repos.map((r) => r.repoKey),
    ...Object.keys(p.merge),
  ])
  const label = (k: string) =>
    repos.find((r) => r.repoKey === k)?.label || shortRepo(k)
  return (
    <Loading key="section" loading={false} label="Loading agent permissions">
      <section
        className="grid gap-2"
        aria-labelledby={`${id}-h`}
        data-setting="agents.permissions"
      >
        <h3 id={`${id}-h`} className="text-sm font-medium text-ink">
          What a new agent may do
        </h3>
        <ChoiceSelect
          label="Permissions"
          showLabel
          size="sm"
          value={choice}
          choices={permissionChoices}
          onValueChange={(v) => void save({ ...p, permission: v })}
          className="text-[13px]"
        />
        <p className="text-[12.5px] text-muted-foreground">
          In a shared space, or when a card quotes someone else, an agent never
          runs without asking: it gets the checks, git and Irori rules instead.
          {choice !== BYPASS && (
            <>
              {" "}
              Force pushes and <code>rm -rf</code> are never allowed.
            </>
          )}
        </p>
        {choice === LIMITED && (
          <div className="grid gap-2 sm:grid-cols-2">
            <label className="grid gap-1 text-[12.5px] text-muted-foreground">
              Also allow, one rule per line
              <Textarea
                rows={3}
                value={allow}
                placeholder="Bash(cargo test:*)"
                className="font-mono text-[12.5px]"
                onChange={(e) => setAllow(e.target.value)}
              />
            </label>
            <label className="grid gap-1 text-[12.5px] text-muted-foreground">
              Never allow, one rule per line
              <Textarea
                rows={3}
                value={deny}
                placeholder="Bash(cargo publish:*)"
                className="font-mono text-[12.5px]"
                onChange={(e) => setDeny(e.target.value)}
              />
            </label>
            {dirty && (
              <span className="flex gap-1.5 sm:col-span-2">
                <Button
                  size="sm"
                  onClick={() =>
                    void save({
                      ...p,
                      allow: rulesOf(allow),
                      deny: rulesOf(deny),
                    }).then((ok) => {
                      if (!ok) return
                      setAllow(null)
                      setDeny(null)
                    })
                  }
                >
                  Save rules
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setAllow(null)
                    setDeny(null)
                  }}
                >
                  Cancel
                </Button>
              </span>
            )}
          </div>
        )}
        {keys.size > 0 && (
          <div className="grid gap-1.5">
            <h4 className="text-[12.5px] font-medium text-ink">
              When its work is done
            </h4>
            <ul className="grid gap-1.5">
              {[...keys].map((k) => (
                <li
                  key={k}
                  className="flex min-w-0 items-center justify-between gap-3"
                >
                  <span
                    className="min-w-0 truncate font-mono text-[12.5px]"
                    title={k}
                  >
                    {label(k)}
                  </span>
                  <ChoiceSelect
                    label={`When an agent in ${label(k)} is done`}
                    size="sm"
                    value={p.merge[k] ?? MERGE_PR}
                    choices={mergeChoices}
                    onValueChange={(v) =>
                      void save({ ...p, merge: { ...p.merge, [k]: v } })
                    }
                    className="w-40 shrink-0 text-[13px]"
                  />
                </li>
              ))}
            </ul>
          </div>
        )}
        {error && <p className="text-[12.5px] text-destructive">{error}</p>}
      </section>
    </Loading>
  )
}

// PermissionsShape is the section's skeleton (card #598): plain markup in
// its shape, with no hooks, for Loading to measure (skullet-block: one
// block for a control).
function PermissionsShape() {
  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium text-ink">What a new agent may do</h3>
      <span className="text-sm font-medium text-ink">Permissions</span>
      <span skullet-block="" className="block h-7 w-[220px] rounded-lg" />
      <p className="text-[12.5px] text-muted-foreground">
        In a shared space, or when a card quotes someone else, an agent never
        runs without asking: it gets the checks, git and Irori rules instead.
      </p>
      <div className="grid gap-1.5">
        <h4 className="text-[12.5px] font-medium text-ink">
          When its work is done
        </h4>
        <div className="flex min-w-0 items-center justify-between gap-3">
          <span className="font-mono text-[12.5px]">sample-org/irori</span>
          <span
            skullet-block=""
            className="block h-7 w-40 shrink-0 rounded-lg"
          />
        </div>
      </div>
    </section>
  )
}
