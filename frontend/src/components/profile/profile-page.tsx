// ProfilePage is you, as everyone you share with sees you (card #393,
// docs/design/talk-channels.md §5, mockup talk-6): your name (#183),
// colour (#385) and avatar, set once and carried on your member.profile
// into every space; the people you share spaces with; and your spaces,
// each with its boards and people. The sidebar's switcher picks among the
// same spaces.

import { useEffect, useId, useRef, useState, type ReactNode } from "react"
import { ImageIcon, LayoutGridIcon, Trash2Icon } from "lucide-react"

import { Face } from "@/components/profile/face"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { shrinkAvatar } from "@/lib/avatar"
import type { BoardSummary } from "@/lib/boards"
import { DeviceColors, devices, useDevices } from "@/lib/devices"
import { plainLine } from "@/lib/irori"
import { go } from "@/lib/nav"
import { meChanged } from "@/lib/profile"
import { useYourName } from "@/lib/settings"
import { Personal, withLine } from "@/lib/spaces"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// Friend is another person you share a space with, and the spaces.
export type Friend = {
  personId: string
  name: string
  color: string
  avatar: string
  spaces: string[]
}

// friendsOf is everyone else in your spaces, once each, by name: their
// newest name, colour and avatar as the spaces list them.
export function friendsOf(boards: BoardSummary[]): Friend[] {
  const by = new Map<string, Friend>()
  for (const b of boards) {
    if (b.leaving) continue
    for (const m of b.members) {
      if (m.me) continue
      const f = by.get(m.personId) ?? {
        personId: m.personId,
        name: m.name,
        color: m.color ?? "",
        avatar: m.avatar ?? "",
        spaces: [],
      }
      f.color ||= m.color ?? ""
      f.avatar ||= m.avatar ?? ""
      f.spaces.push(b.name || b.tag)
      by.set(m.personId, f)
    }
  }
  return [...by.values()].sort((a, b) => a.name.localeCompare(b.name))
}

export function ProfilePage({
  boards,
  agents,
  space,
  onSpace,
  onNewSpace,
}: {
  boards: BoardSummary[]
  // How many agents the person runs, for the "6 agents" line.
  agents: number
  // The switcher's pick, outlined among the spaces; a click picks one.
  space: string
  onSpace: (id: string) => void
  // New or join a space (card #689), for Invite a friend with no shared
  // space picked.
  onNewSpace: () => void
}) {
  const { name, error: nameError, save } = useYourName(true)
  const { devices: devs } = useDevices(true)
  const [color, setColor] = useState("")
  const [avatar, setAvatar] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const file = useRef<HTMLInputElement>(null)
  useEffect(() => {
    let live = true
    Promise.all([devices.personColor(), devices.personAvatar()])
      .then(([c, a]) => {
        if (!live) return
        setColor(c)
        setAvatar(a)
      })
      .catch((e: unknown) => live && setError(errorText(e)))
    return () => {
      live = false
    }
  }, [])
  const pickColor = async (c: string) => {
    try {
      await devices.setPersonColor(c)
      setColor(c)
      setError("")
      meChanged()
    } catch (e) {
      setError(errorText(e))
    }
  }
  const pickAvatar = async (f: File | undefined) => {
    if (!f) return
    setBusy(true)
    try {
      const a = await shrinkAvatar(f)
      await devices.setPersonAvatar(a)
      setAvatar(a)
      setError("")
      meChanged()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
      if (file.current) file.current.value = ""
    }
  }
  const dropAvatar = async () => {
    try {
      await devices.setPersonAvatar("")
      setAvatar("")
      setError("")
      meChanged()
    } catch (e) {
      setError(errorText(e))
    }
  }
  const me = name ?? ""
  const live = devs.filter((d) => !d.revoked).length
  const friends = friendsOf(boards)
  const shared = [...boards]
    .filter((b) => !b.leaving)
    .sort((a, b) => a.name.localeCompare(b.name))

  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-6 pb-8 lg:grid-cols-[minmax(0,20rem)_minmax(0,1fr)]">
      <div className="grid min-w-0 content-start gap-6">
        <Column title="You">
          <div className="grid gap-4 p-4 glass-panel">
            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={() => file.current?.click()}
                disabled={busy}
                className="group relative rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label={avatar ? "Change your picture" : "Add a picture"}
                title={avatar ? "Change your picture" : "Add a picture"}
              >
                <Face name={me} color={color} avatar={avatar} size="xl" />
                <span className="absolute inset-0 hidden items-center justify-center rounded-full bg-black/45 text-white group-hover:flex">
                  <ImageIcon className="size-5" aria-hidden />
                </span>
              </button>
              <input
                ref={file}
                type="file"
                accept="image/png,image/jpeg,image/webp,image/gif,image/heic"
                className="hidden"
                onChange={(e) => void pickAvatar(e.target.files?.[0])}
              />
              <div className="grid min-w-0 gap-0.5">
                <span className="truncate text-base font-semibold text-ink">
                  {me || "…"}
                </span>
                <span className="text-xs text-muted-foreground">
                  {live} {live === 1 ? "device" : "devices"} · {agents}{" "}
                  {agents === 1 ? "agent" : "agents"}
                </span>
                <span className="flex gap-1 pt-1">
                  <Button
                    size="xs"
                    variant="outline"
                    disabled={busy}
                    onClick={() => file.current?.click()}
                  >
                    {busy
                      ? "Shrinking…"
                      : avatar
                        ? "Change picture"
                        : "Add a picture"}
                  </Button>
                  {avatar && (
                    <Button
                      size="xs"
                      variant="ghost"
                      onClick={() => void dropAvatar()}
                      aria-label="Remove your picture"
                      title="Remove your picture"
                    >
                      <Trash2Icon aria-hidden />
                    </Button>
                  )}
                </span>
              </div>
            </div>
            <NameField name={name} error={nameError} save={save} />
            <div className="grid gap-1.5">
              <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
                Your colour
              </span>
              <span className="flex items-center gap-1.5">
                {DeviceColors.map((c) => (
                  <button
                    key={c}
                    type="button"
                    title={c}
                    aria-label={`${c} as your colour`}
                    aria-pressed={color === c}
                    className={cn(
                      "size-6 rounded-full ring-offset-2 ring-offset-background outline-none focus-visible:ring-2 focus-visible:ring-ring",
                      color === c && "ring-2 ring-ink"
                    )}
                    style={{ background: `var(--device-${c})` }}
                    onClick={() => void pickColor(color === c ? "" : c)}
                  />
                ))}
              </span>
            </div>
            <p className="text-xs text-muted-foreground">
              {error ? (
                <span className="text-destructive">{error}</span>
              ) : (
                <>
                  Others see your name, colour and picture everywhere you share:
                  spaces, channels, DMs. Your agents wear the colour as a
                  stripe.
                  {!color &&
                    " With no pick, each friend's screen picks one none of their devices wear."}
                </>
              )}
            </p>
          </div>
        </Column>

        <Column title="Friends">
          <div className="grid gap-1 p-3 glass-panel">
            {friends.length === 0 ? (
              <p className="px-1 text-xs text-muted-foreground">
                Nobody yet. Invite someone to a space and they show here.
              </p>
            ) : (
              friends.map((f) => (
                <div
                  key={f.personId}
                  className="flex items-center gap-2.5 px-1 py-1"
                >
                  <Face
                    name={f.name}
                    color={f.color}
                    avatar={f.avatar}
                    size="md"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-ink">
                    {plainLine(f.name)}
                  </span>
                  <span className="truncate text-xs text-muted-foreground">
                    in {f.spaces.map(plainLine).join(", ")}
                  </span>
                </div>
              ))
            )}
            <button
              type="button"
              // An invite is to one space: the picked shared space's
              // page has Invite; otherwise make one (card #689).
              onClick={() =>
                boards.some((b) => b.boardId === space)
                  ? go({ kind: "board", id: space })
                  : onNewSpace()
              }
              className="justify-self-start px-1 pt-1 text-xs text-primary hover:underline"
            >
              + Invite a friend
            </button>
          </div>
        </Column>
      </div>

      <Column title="Spaces: a group of people that holds many boards and channels">
        <div className="grid gap-4">
          <SpaceCard
            name="Personal"
            current={space === Personal}
            onPick={() => onSpace(Personal)}
            people={
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Face name={me} color={color} avatar={avatar} size="sm" />
                just you + your agents
              </span>
            }
            note="Everything not shared lives here: your own cards and threads."
          />
          {shared.map((b) => (
            <SpaceCard
              key={b.boardId}
              name={plainLine(b.name) || plainLine(b.tag)}
              current={space === b.boardId}
              onPick={() => onSpace(b.boardId)}
              people={
                <span className="flex -space-x-1">
                  {b.members.map((m) => (
                    <Face
                      key={m.personId}
                      name={m.me ? me || m.name : m.name}
                      color={m.me ? color : m.color}
                      avatar={m.me ? avatar : m.avatar}
                      size="sm"
                      className="ring-2 ring-background"
                    />
                  ))}
                </span>
              }
              boards={(b.boards?.length
                ? b.boards
                : [{ boardId: b.boardId, name: b.name, tag: b.tag }]
              ).map((x) => ({
                id: x.boardId,
                name: plainLine(x.name) || plainLine(x.tag),
              }))}
              onBoard={() => go({ kind: "board", id: b.boardId })}
              note={
                b.members.length > 1
                  ? `Shared ${withLine(b.members.filter((m) => !m.me).map((m) => plainLine(m.name)))}. Who's in, and what reaches your agents from them, is set once here, not per board.`
                  : "Only you so far. Invite someone from Space settings."
              }
            />
          ))}
        </div>
      </Column>
    </div>
  )
}

function Column({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="grid min-w-0 grid-cols-[minmax(0,1fr)] content-start gap-3">
      <h2 className="text-[15px] font-semibold text-ink">{title}</h2>
      {children}
    </section>
  )
}

function SpaceCard({
  name,
  current,
  onPick,
  people,
  boards,
  onBoard,
  note,
}: {
  name: string
  current: boolean
  onPick: () => void
  people: ReactNode
  boards?: { id: string; name: string }[]
  onBoard?: (id: string) => void
  note: string
}) {
  return (
    <div
      className={cn(
        "grid gap-3 p-4 glass-panel",
        current && "border-primary/70!"
      )}
    >
      <div className="flex items-center gap-3">
        <h3 className="flex-1 truncate text-[15px] font-semibold text-ink">
          {name}
        </h3>
        {people}
        <Button
          size="xs"
          variant={current ? "secondary" : "outline"}
          onClick={onPick}
          aria-pressed={current}
          title={
            current
              ? "The sidebar shows this space"
              : "Show this space in the sidebar"
          }
        >
          {current ? "Showing" : "Show"}
        </Button>
      </div>
      {boards && boards.length > 0 && (
        <div className="grid gap-1">
          <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
            Boards
          </span>
          {boards.map((x) => (
            <button
              key={x.id}
              type="button"
              onClick={() => onBoard?.(x.id)}
              className="flex items-center gap-2 justify-self-start text-sm text-ink-2 hover:text-ink"
            >
              <LayoutGridIcon
                className="size-3.5 text-muted-foreground"
                aria-hidden
              />
              {x.name}
            </button>
          ))}
        </div>
      )}
      <p className="border-t border-border pt-2.5 text-xs text-muted-foreground">
        {note}
      </p>
    </div>
  )
}

// NameField is your one name (card #183), as Settings has it: Enter or
// Save keeps it, Escape drops the edit.
function NameField({
  name,
  error,
  save,
}: {
  name: string | null
  error: string | null
  save: (n: string) => Promise<boolean>
}) {
  const [draft, setDraft] = useState<string | null>(null)
  const id = useId()
  const value = draft ?? name ?? ""
  const changed = draft !== null && draft.trim() !== (name ?? "")
  const keep = async () => {
    if (changed && (await save(value))) {
      setDraft(null)
      meChanged()
    }
  }
  return (
    <div className="grid gap-1">
      <label
        htmlFor={id}
        className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase"
      >
        Your name
      </label>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          value={value}
          maxLength={60}
          disabled={name === null}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void keep()
            if (e.key === "Escape" && draft !== null) {
              e.stopPropagation()
              setDraft(null)
            }
          }}
          className="h-8 text-[13px]"
        />
        <Button
          size="sm"
          variant="outline"
          disabled={!changed || !value.trim()}
          onClick={() => void keep()}
        >
          Save
        </Button>
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}
