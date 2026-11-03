// Card #388: exploration mockups for Notes (shared and private context
// that agents read and people review). Browser preview only
// (?mockup=notes&view=1..3); static JSX on made-up data, no lib/ calls,
// no state. Lives on a mockup branch, never main.

import type { ReactNode } from "react"
import {
  AlertTriangleIcon,
  BookOpenIcon,
  CheckIcon,
  HomeIcon,
  InboxIcon,
  LayoutGridIcon,
  LinkIcon,
  LockIcon,
  MessagesSquareIcon,
  NotebookPenIcon,
  PanelLeftIcon,
  PencilIcon,
  SearchIcon,
  SettingsIcon,
  SparklesIcon,
  UsersIcon,
  XIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { cn } from "@/lib/utils"

// ---------- small pieces (as talk-mockups.tsx) ----------

type Who = { name: string; kind: "person" | "agent"; tone?: string; of?: string }
const ST: Who = { name: "Stephen", kind: "person" }
const BL: Who = { name: "Blake", kind: "person", tone: "bg-chart-1" }
const KE: Who = { name: "kestrel", kind: "agent", tone: "bg-chart-3" }
const MJ: Who = { name: "marjoram", kind: "agent", tone: "bg-chart-2" }
const KI: Who = { name: "kite", kind: "agent", tone: "bg-chart-5", of: "Blake" }

function Av({ w, size = 6 }: { w: Who; size?: number }) {
  return (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center font-semibold text-background",
        w.kind === "person" ? cn("rounded-full", w.tone ?? "bg-primary") : cn("rounded-md", w.tone),
        size === 7 ? "size-7 text-[11px]" : size === 5 ? "size-5 text-[9px]" : "size-6 text-[10px]"
      )}
    >
      {w.name[0].toUpperCase()}
    </span>
  )
}

function Ref({ n, t }: { n: string; t?: string }) {
  return (
    <span className="inline-flex max-w-full items-center gap-1 rounded-md bg-secondary px-1.5 py-0.5 align-middle text-[11px] text-ink-2">
      <span className="size-1.5 rounded-full bg-success" />
      <span className="font-mono text-primary">{n}</span>
      {t && <span className="truncate">{t}</span>}
    </span>
  )
}

function Where({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center rounded-md border border-border px-1.5 py-0.5 text-[10.5px] text-muted-foreground">
      {children}
    </span>
  )
}

type Kind = "rule" | "gotcha" | "decision" | "pointer" | "preference"
const kindTone: Record<Kind, "destructive" | "warning" | "success" | "info" | "slate"> = {
  rule: "destructive",
  gotcha: "warning",
  decision: "success",
  pointer: "info",
  preference: "slate",
}
function KindBadge({ k }: { k: Kind }) {
  return (
    <Badge variant={kindTone[k]} className="h-5 px-1.5 text-[10px] capitalize">
      {k}
    </Badge>
  )
}

function Callout({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "rounded-lg border border-dashed border-warning/60 bg-warning-foreground px-3 py-2 text-[12px] leading-snug text-warning",
        className
      )}
    >
      {children}
    </div>
  )
}

// ---------- the app's frame ----------

function NavRow({
  icon: I,
  label,
  current,
  count,
}: {
  icon: typeof HomeIcon
  label: string
  current?: boolean
  count?: number
}) {
  return (
    <div
      className={cn(
        "flex h-9 items-center gap-3 rounded-md px-2.5 text-sm text-ink-2",
        current && "bg-secondary font-medium text-ink"
      )}
    >
      <I className="size-4 shrink-0" />
      <span className="flex-1 truncate">{label}</span>
      {!!count && (
        <Badge variant="info" className="h-5 px-1.5 tabular-nums">
          {count}
        </Badge>
      )}
    </div>
  )
}

function Sidebar({ current = "notes" }: { current?: string }) {
  return (
    <aside className="flex h-full w-[232px] shrink-0 flex-col border-r border-border bg-sidebar/60">
      <div className="flex h-14 items-center gap-2 px-4">
        <span className="text-lg font-semibold tracking-tight text-ink">Irori</span>
        <span className="text-sm text-muted-foreground">囲炉裏</span>
        <PanelLeftIcon className="ml-auto size-4 text-muted-foreground" />
      </div>
      <nav className="flex flex-1 flex-col gap-0.5 px-2">
        <NavRow icon={HomeIcon} label="Home" current={current === "home"} />
        <NavRow icon={MessagesSquareIcon} label="Talk" current={current === "talk"} />
        <NavRow icon={LayoutGridIcon} label="Boards" current={current === "boards"} />
        <NavRow icon={NotebookPenIcon} label="Notes" current={current === "notes"} count={2} />
        <NavRow icon={InboxIcon} label="Held" current={current === "held"} />
        <div className="px-2.5 pt-4 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          Pinned boards
        </div>
        <div className="flex h-8 items-center gap-2.5 px-2.5 text-sm text-ink-2">
          <span className="rounded bg-secondary px-1 font-mono text-[10px] text-muted-foreground">ml</span>
          MLG
        </div>
      </nav>
      <div className="px-4 pb-3">
        <div className="pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          Agents
        </div>
        <div className="flex gap-1">
          <Badge variant="info" className="normal-case">3 working</Badge>
          <Badge variant="slate" className="normal-case">2 free</Badge>
        </div>
      </div>
      <div className="flex items-center border-t border-border px-4 py-3 text-xs text-ink-2">
        Stephen&apos;s MacBook
        <SettingsIcon className="ml-auto size-4 text-muted-foreground" />
      </div>
    </aside>
  )
}

function Header({ title, right, sub }: { title: ReactNode; right?: ReactNode; sub?: ReactNode }) {
  return (
    <header className="flex h-14 shrink-0 items-center gap-3 border-b border-border px-6">
      <h1 className="text-xl font-semibold text-ink">{title}</h1>
      {sub}
      <div className="ml-auto flex items-center gap-2">{right}</div>
    </header>
  )
}

function Window({ children, current }: { children: ReactNode; current?: string }) {
  return (
    <div className="flex h-full overflow-hidden">
      <Sidebar current={current} />
      <div className="flex min-w-0 flex-1 flex-col">{children}</div>
    </div>
  )
}

function Frame({ title, pick, children, caption }: { title: string; pick?: boolean; children: ReactNode; caption: ReactNode }) {
  return (
    <div className="grid min-w-0 grid-rows-[auto_1fr_auto] gap-2">
      <div className="flex items-center gap-2">
        <span className="text-[15px] font-semibold text-ink">{title}</span>
        {pick && <Badge variant="success">My pick</Badge>}
      </div>
      <div
        className={cn(
          "h-[640px] overflow-hidden rounded-xl border bg-background",
          pick ? "border-success/70 ring-2 ring-success/30" : "border-border"
        )}
      >
        <div className="h-[900px] w-[1100px] origin-top-left scale-[0.581]">{children}</div>
      </div>
      <div className="text-[12.5px] leading-snug text-ink-2">{caption}</div>
    </div>
  )
}

function Tabs({ items, on }: { items: string[]; on: number }) {
  return (
    <div className="flex gap-1 rounded-lg bg-secondary/60 p-0.5 text-[12.5px]">
      {items.map((t, i) => (
        <span
          key={t}
          className={cn("rounded-md px-2.5 py-1 text-ink-2", i === on && "bg-background font-medium text-ink shadow-sm")}
        >
          {t}
        </span>
      ))}
    </div>
  )
}

// ---------- notes (made-up, but drawn from this board's real lessons) ----------

type N = {
  k: Kind
  text: string
  where: string[]
  from?: [string, string?]
  by: Who
  ok?: Who
  read?: number
  stale?: string
}

const mine: N[] = [
  {
    k: "rule",
    text: "Never drive Stephen's mouse or keyboard. UI checks use passive screenshots or the browser preview.",
    where: ["everywhere"],
    from: ["#327", "Stephen's comment"],
    by: ST,
    read: 41,
  },
  {
    k: "gotcha",
    text: "A scratch Irori dev build still hands asks to live agents. Never send from one.",
    where: ["repo irori"],
    from: ["#321"],
    by: MJ,
    ok: ST,
    read: 9,
  },
  {
    k: "gotcha",
    text: "PC release builds need CMAKE and LIBCLANG_PATH set before cargo.",
    where: ["repo irori", "on the PC"],
    by: KE,
    ok: ST,
    read: 6,
  },
  {
    k: "decision",
    text: "Devices meet through n0's public relays, on by default (decision 43).",
    where: ["repo irori"],
    from: ["#328", "b1 answer"],
    by: ST,
    read: 12,
  },
  {
    k: "gotcha",
    text: "Each worktree builds into its own target/. A shared target mixes agents' crates.",
    where: ["repo irori"],
    by: KE,
    ok: ST,
    read: 22,
    stale: "names scripts/check.sh, changed 2 days ago",
  },
  {
    k: "preference",
    text: "The app says Version N, never take.",
    where: ["repo ai-asset-pipeline"],
    by: ST,
    read: 3,
  },
]

const board: N[] = [
  {
    k: "decision",
    text: "Boats stop at the dock's edge: the collider is wider, two-PC check passed.",
    where: ["MLG"],
    from: ["mlg#12"],
    by: BL,
    read: 4,
  },
  {
    k: "pointer",
    text: "Lap times live in the shared sheet, not in the repo.",
    where: ["MLG"],
    by: ST,
    read: 2,
  },
]

function NoteRow({ n, dense }: { n: N; dense?: boolean }) {
  return (
    <div className={cn("grid gap-1.5 border-b border-border/70 px-4", dense ? "py-2" : "py-3")}>
      <div className="flex items-start gap-2">
        <KindBadge k={n.k} />
        <span className="flex-1 text-[13px] leading-snug text-ink">{n.text}</span>
        {!dense && <PencilIcon className="mt-0.5 size-3.5 text-muted-foreground" />}
      </div>
      <div className="flex flex-wrap items-center gap-1.5 pl-[3px] text-[11px] text-muted-foreground">
        {n.where.map((w) => (
          <Where key={w}>{w}</Where>
        ))}
        {n.from && <Ref n={n.from[0]} t={n.from[1]} />}
        <span className="flex items-center gap-1">
          <Av w={n.by} size={5} />
          {n.by.name}
          {n.ok && <> · OK&apos;d by {n.ok.name}</>}
        </span>
        {n.read !== undefined && <span>· read {n.read}×</span>}
        {n.stale && (
          <span className="flex items-center gap-1 text-warning">
            <AlertTriangleIcon className="size-3" /> Still true? {n.stale}
          </span>
        )}
      </div>
    </div>
  )
}

function Proposed({
  n,
  shared,
}: {
  n: N
  shared?: boolean
}) {
  return (
    <div className="grid gap-2 rounded-lg border border-info/40 bg-info-foreground/40 p-3">
      <div className="flex items-center gap-2 text-[11.5px] text-ink-2">
        <SparklesIcon className="size-3.5 text-info" />
        <Av w={n.by} size={5} />
        <span>
          <b className="font-medium text-ink">{n.by.name}</b>
          {n.by.of ? <> ({n.by.of}&apos;s agent)</> : null} proposes
        </span>
        {n.from && <Ref n={n.from[0]} t={n.from[1]} />}
        <span className="ml-auto">
          {shared ? (
            <Badge variant="warning" className="h-5 px-1.5 text-[10px]">Not read by any agent yet</Badge>
          ) : (
            <Badge variant="info" className="h-5 px-1.5 text-[10px]">Live for your agents</Badge>
          )}
        </span>
      </div>
      <div className="flex items-start gap-2">
        <KindBadge k={n.k} />
        <span className="text-[13px] leading-snug text-ink">{n.text}</span>
      </div>
      <div className="flex items-center gap-1.5">
        {n.where.map((w) => (
          <Where key={w}>{w}</Where>
        ))}
        <span className="ml-auto flex gap-1.5">
          <span className="flex h-7 items-center gap-1 rounded-md bg-primary px-2.5 text-[12px] font-medium text-primary-foreground">
            <CheckIcon className="size-3.5" /> {shared ? "Accept" : "Keep"}
          </span>
          <span className="flex h-7 items-center gap-1 rounded-md bg-secondary px-2.5 text-[12px] text-ink-2">
            <PencilIcon className="size-3.5" /> Edit
          </span>
          <span className="flex h-7 items-center gap-1 rounded-md bg-secondary px-2.5 text-[12px] text-ink-2">
            <XIcon className="size-3.5" /> {shared ? "Reject" : "Remove"}
          </span>
        </span>
      </div>
    </div>
  )
}

const kiteNote: N = {
  k: "gotcha",
  text: "Run the lobby test with both PCs on the same build hash, or the join silently fails.",
  where: ["MLG"],
  from: ["mlg#3"],
  by: KI,
}
const kestrelNote: N = {
  k: "gotcha",
  text: "bun can't install a git dependency from a subfolder: ship suzuri-view as a tarball on the release.",
  where: ["repo irori", "frontend/"],
  from: ["#367"],
  by: MJ,
}

const pcNote: N = {
  k: "gotcha",
  text: "On the PC, test builds never write suzuri's bridge.json: use a scratch LOCALAPPDATA.",
  where: ["repo rust-suzuri", "on the PC"],
  by: KE,
}

// ---------- A: the Notes page ----------

function NotesPage({ scope = 0 }: { scope?: number }) {
  return (
    <Window current="notes">
      <Header
        title="Notes"
        sub={<Tabs items={["Mine", "MLG · with Blake"]} on={scope} />}
        right={
          <>
            <span className="flex h-8 w-56 items-center gap-2 rounded-md border border-border px-2.5 text-[12.5px] text-muted-foreground">
              <SearchIcon className="size-3.5" /> Search notes
            </span>
            <span className="flex h-8 items-center rounded-md bg-primary px-3 text-[12.5px] font-medium text-primary-foreground">
              New note
            </span>
          </>
        }
      />
      <div className="flex min-h-0 flex-1">
        <div className="grid min-w-0 flex-1 content-start gap-4 overflow-hidden p-5">
          <div className="grid gap-2">
            <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
              New from agents · {scope === 0 ? 2 : 1}
            </div>
            {scope === 0 ? (
              <>
                <Proposed n={kestrelNote} />
                <Proposed n={pcNote} />
              </>
            ) : (
              <Proposed n={kiteNote} shared />
            )}
          </div>
          <div className="flex items-center gap-1.5 text-[12px]">
            {(scope === 0 ? ["All 38", "Rules 6", "Gotchas 17", "Decisions 9", "Pointers 4", "Preferences 2"] : ["All 2", "Decisions 1", "Pointers 1"]).map((t, i) => (
              <span key={t} className={cn("rounded-full px-2.5 py-1 text-ink-2", i === 0 ? "bg-secondary text-ink" : "border border-border")}>
                {t}
              </span>
            ))}
          </div>
          <div className="overflow-hidden rounded-lg border border-border">
            {(scope === 0 ? mine : board).map((n) => (
              <NoteRow key={n.text} n={n} />
            ))}
          </div>
        </div>
      </div>
    </Window>
  )
}

// ---------- B: built from the board ----------

function Learned() {
  const rows: [string, string, string][] = [
    ["Blocker answer", "#328 b1", "Use n0's public relays; Settings switch to turn them off."],
    ["Blocker answer", "#369 b2", "A shell starts only when you click New terminal."],
    ["Done proof", "#343", "A bare #N only a shared board's card has goes to the lookup."],
    ["Comment", "mlg#3 · kite", "Both PCs on the same build hash for the lobby test."],
    ["Comment", "#309 · Stephen", "they're taking control of my computer though when I need it"],
    ["Done proof", "#367", "bun can't take a git dependency from a subfolder."],
  ]
  return (
    <Window current="boards">
      <Header title="What Irori learned" sub={<Tabs items={["Mine", "MLG"]} on={0} />} />
      <div className="grid content-start gap-3 p-5">
        <div className="text-[12.5px] text-ink-2">
          Pulled from blocker answers, done proofs and comments this week. 214 items. Agents search it on wake.
        </div>
        <div className="overflow-hidden rounded-lg border border-border">
          {rows.map(([src, ref, text]) => (
            <div key={ref} className="flex items-center gap-3 border-b border-border/70 px-4 py-2.5 text-[13px]">
              <Badge variant="slate" className="h-5 w-28 justify-center px-1.5 text-[10px]">{src}</Badge>
              <Ref n={ref.split(" ")[0]} t={ref.split(" · ")[1]} />
              <span className="flex-1 truncate text-ink-2">{text}</span>
              <span className="text-[11px] text-muted-foreground">auto</span>
            </div>
          ))}
        </div>
        <Callout>
          A comment from Blake&apos;s agent on MLG is in here too: your agents would read it with nobody checking it first.
        </Callout>
      </div>
    </Window>
  )
}

// ---------- C: a board handbook ----------

function Handbook() {
  return (
    <Window current="boards">
      <Header title="MLG handbook" sub={<Badge variant="slate">Shared with Blake</Badge>} right={<span className="text-[12px] text-muted-foreground">edited by Stephen · 2d</span>} />
      <div className="grid content-start gap-3 overflow-hidden p-6 text-[13px] leading-relaxed text-ink-2">
        <h2 className="text-[15px] font-semibold text-ink">Building</h2>
        <p>Both PCs build from the same commit. The dock collider is wider since mlg#12.</p>
        <h2 className="text-[15px] font-semibold text-ink">Testing</h2>
        <p>Two-PC checks before a card is done. Lap times live in the shared sheet.</p>
        <div className="grid gap-1 rounded-lg border border-info/40 bg-info-foreground/40 p-3 font-mono text-[12px]">
          <div className="flex items-center gap-2 pb-1 font-sans text-[11.5px] text-ink-2">
            <Av w={KI} size={5} /> kite (Blake&apos;s agent) suggests an edit
            <span className="ml-auto flex gap-1.5 font-sans">
              <span className="rounded-md bg-primary px-2 py-0.5 text-[11.5px] text-primary-foreground">Apply</span>
              <span className="rounded-md bg-secondary px-2 py-0.5 text-[11.5px]">Reject</span>
            </span>
          </div>
          <div className="text-destructive">- Two-PC checks before a card is done.</div>
          <div className="text-success">+ Two-PC checks, both PCs on the same build hash, before a card is done.</div>
        </div>
        <h2 className="text-[15px] font-semibold text-ink">Releases</h2>
        <p>…</p>
      </div>
    </Window>
  )
}

// ---------- 1: three shapes ----------

function View1() {
  return (
    <div className="grid h-full grid-cols-3 gap-6 overflow-hidden p-6">
      <Frame
        title="A · Notes, kept on purpose"
        pick
        caption={
          <>
            Short notes (rule, gotcha, decision, pointer), each tagged with where it applies and linked to the card it came from. Agents
            propose, people keep or reject. <b>Mine</b> stays on your devices; a board&apos;s notes reach its members. Agents get the
            few that match on wake and search the rest.
          </>
        }
      >
        <NotesPage />
      </Frame>
      <Frame
        title="B · Built from the board"
        caption={
          <>
            No writing: Irori indexes blocker answers, done proofs and comments, and agents search it. Nothing to curate, but it fills
            with noise and stale answers, and on a shared board another person&apos;s agent&apos;s words reach yours unchecked.
          </>
        }
      >
        <Learned />
      </Frame>
      <Frame
        title="C · A handbook per board"
        caption={
          <>
            One page per board that people edit and agents read whole on wake; agents suggest edits as diffs. Easy to grasp and
            review, but it gets long, every agent reads all of it, and Mac-only or PC-only gotchas have nowhere to go.
          </>
        }
      >
        <Handbook />
      </Frame>
    </div>
  )
}

// ---------- 2: shared-board safety ----------

function View2() {
  return (
    <div className="grid h-full grid-cols-2 gap-6 overflow-hidden p-6">
      <Frame
        title="Your notes (Mine)"
        pick
        caption={
          <>
            Your agents&apos; notes go live at once and wait here for a look: <b>Keep</b>, edit or remove. Mine never leaves your devices,
            even when a note says it applies on a shared board (&ldquo;on MLG&rdquo;).
          </>
        }
      >
        <NotesPage scope={0} />
      </Frame>
      <Frame
        title="A shared board's notes"
        caption={
          <>
            A note from someone else&apos;s side (Blake or his agent kite) reaches <b>your</b> agents only once <b>you</b> accept it; Blake
            decides the same for his. Accepted notes still reach agents as data with who wrote and who accepted them, never as
            instructions.
          </>
        }
      >
        <NotesPage scope={1} />
      </Frame>
    </div>
  )
}

// ---------- 3: capture and recall ----------

function Capture() {
  return (
    <Window current="boards">
      <Header title={<span className="flex items-center gap-2"><span className="font-mono text-base text-muted-foreground">#369</span> Inline terminal on a card</span>} />
      <div className="grid content-start gap-3 p-5">
        <div className="grid gap-2 rounded-lg border border-border p-3">
          <div className="flex items-center gap-2 text-[12px] text-ink-2">
            <Badge variant="success" className="h-5 px-1.5 text-[10px]">Answered</Badge>
            Start a card&apos;s shell when you open the card, or only after you click?
          </div>
          <div className="flex items-start gap-2 text-[13px] text-ink">
            <Av w={ST} size={5} /> Only when I click New terminal. Several per card, as tabs.
          </div>
          <div className="relative flex justify-end">
            <span className="flex h-7 items-center gap-1 rounded-md bg-secondary px-2.5 text-[12px] text-ink-2 ring-1 ring-ring">
              <BookOpenIcon className="size-3.5" /> Keep as note
            </span>
            <div className="absolute top-9 right-0 z-10 grid w-[420px] gap-2.5 rounded-lg border border-border bg-popover p-3 shadow-xl">
              <div className="text-[12px] font-medium text-ink">Keep as note</div>
              <div className="rounded-md border border-border bg-background px-2.5 py-2 text-[13px] text-ink">
                A card&apos;s shell starts only on New terminal; a card can hold several, as tabs.
              </div>
              <div className="flex flex-wrap items-center gap-1.5 text-[11.5px] text-ink-2">
                Kind <KindBadge k="decision" />
                <span className="pl-2">Applies to</span>
                <Where>repo irori</Where>
                <Where>terminals</Where>
                <span className="text-muted-foreground">+ add</span>
              </div>
              <div className="flex items-center gap-2 text-[12px] text-ink-2">
                <span className="flex items-center gap-1 rounded-md bg-secondary px-2 py-1 text-ink">
                  <LockIcon className="size-3" /> Mine
                </span>
                <span className="flex items-center gap-1 rounded-md border border-border px-2 py-1">
                  <UsersIcon className="size-3" /> A shared board…
                </span>
                <span className="ml-auto rounded-md bg-primary px-2.5 py-1 font-medium text-primary-foreground">Keep</span>
              </div>
              <div className="flex items-center gap-1 text-[11px] text-muted-foreground">
                <LinkIcon className="size-3" /> Links back to #369 b2 and your answer
              </div>
            </div>
          </div>
        </div>
      </div>
    </Window>
  )
}

function Recall() {
  const got: [N, string][] = [
    [mine[0], "always"],
    [mine[2], "repo irori · on the PC"],
    [mine[1], "repo irori"],
    [{ ...kestrelNote, by: MJ, ok: ST, read: 1 }, "repo irori · frontend/"],
  ]
  return (
    <Window current="boards">
      <Header title={<span className="flex items-center gap-2"><Av w={KE} /> What kestrel got on wake</span>} sub={<Ref n="#369" t="Inline terminal on a card" />} />
      <div className="grid content-start gap-3 p-5">
        <div className="text-[12.5px] text-ink-2">
          Irori picks notes by the card&apos;s board, repo and paths, and the agent&apos;s machine. At most 8; the rest by{" "}
          <span className="font-mono text-ink">notes_search</span>.
        </div>
        <div className="overflow-hidden rounded-lg border border-border">
          {got.map(([n, why]) => (
            <div key={n.text} className="flex items-start gap-3 border-b border-border/70 px-4 py-2.5">
              <KindBadge k={n.k} />
              <span className="flex-1 text-[13px] leading-snug text-ink">{n.text}</span>
              <span className="shrink-0 text-[11px] text-muted-foreground">{why}</span>
            </div>
          ))}
        </div>
        <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <SearchIcon className="size-3.5" /> kestrel searched &ldquo;conpty flags&rdquo;: 2 notes
        </div>
        <Callout>Not sent: kite&apos;s MLG note (waits on your Accept), and anything from boards this card isn&apos;t on.</Callout>
      </div>
    </Window>
  )
}

function View3() {
  return (
    <div className="grid h-full grid-cols-2 gap-6 overflow-hidden p-6">
      <Frame
        title="Keeping one"
        caption={
          <>
            Any blocker answer, comment or done proof has <b>Keep as note</b>, prefilled from the text, linked back to where it came
            from. Agents propose the same way, with a tool. Nothing becomes a note by itself.
          </>
        }
      >
        <Capture />
      </Frame>
      <Frame
        title="Getting the right ones"
        caption={
          <>
            On wake, the agent gets the few notes whose tags match the card (board, repo, paths, machine), plus the always-on rules.
            The card page can show which ones it got, so you can see why an agent knew something.
          </>
        }
      >
        <Recall />
      </Frame>
    </div>
  )
}

export default function NotesMockups() {
  const v = new URLSearchParams(window.location.search).get("view") ?? "1"
  const view = v === "2" ? <View2 /> : v === "3" ? <View3 /> : <View1 />
  return <div className="h-svh w-full overflow-hidden bg-background text-foreground">{view}</div>
}
