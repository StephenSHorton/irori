// StoragePage is Storage (card #728, docs/design/storage.md): everything
// Irori keeps on this device and how much space each kind takes, each
// kind's items to browse, and the clean-outs the person asks for. It took
// over Settings > Maintenance (#138): Clear done cards lives on the log's
// page here. Nothing is cleaned out by itself (board.md "No retention
// rules"), and each clean says plainly what can't be undone.

import { useCallback, useMemo, useState } from "react"
import {
  AlertTriangleIcon,
  ArrowLeftIcon,
  ChevronRightIcon,
  ExternalLinkIcon,
  RefreshCwIcon,
} from "lucide-react"

import { ClearDone, clearAges } from "@/components/cards/clear-done"
import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Loading } from "@/components/ui/loading"
import {
  ClosedBoards,
  useClosedBoards,
  type ClosedBoard,
} from "@/components/storage/closed-boards"
import { relativeTime } from "@/lib/irori"
import {
  bytesText,
  cleanableBytes,
  infoOf,
  measuredFrom,
  partLabels,
  refreshStorage,
  storage,
  total,
  useStorageFresh,
  useStorageItems,
  useStorageReport,
  type ClearedCard,
  type LogPart,
  type StorageCleaned,
  type StorageItem,
  type StorageKind,
} from "@/lib/storage"
import { errorText, useLatest } from "@/lib/talk"
import { cn } from "@/lib/utils"

// The bar's colours, largest kind first; the rest share the last.
const barColours = [
  "bg-chart-1",
  "bg-chart-2",
  "bg-chart-3",
  "bg-chart-4",
  "bg-chart-5",
  "bg-muted-foreground/40",
]

export function StoragePage({
  tab,
  onTab,
  onOpen,
  now,
}: {
  // "" for the overview, else one kind's page.
  tab: string
  onTab: (tab: string) => void
  onOpen: (cardId: string) => void
  now: Date
}) {
  const { report, loading, error } = useStorageReport()
  // Build output and Claude Code's folder are measured by listing them.
  const builds = useStorageItems("builds")
  const claude = useStorageItems("claude")
  const kinds = useMemo(
    () =>
      report.kinds.map((k) =>
        k.id === "builds" && !builds.loading && !builds.error
          ? measuredFrom(k, builds.items)
          : k.id === "claude" && !claude.loading && !claude.error
            ? measuredFrom(k, claude.items)
            : k
      ),
    [report.kinds, builds, claude]
  )
  const walkError = builds.error ?? claude.error
  const kind = kinds.find((k) => k.id === tab)
  // Closed boards (#690) aren't a size of their own: their cards are in
  // their space's log. They are browsed here all the same.
  const closed = useClosedBoards()

  if (tab === "closed")
    return (
      <div className="grid content-start gap-4">
        <BackToStorage onTab={onTab} />
        <h1 className="text-lg font-semibold text-ink">Closed boards</h1>
        <p className="max-w-3xl text-[13px] text-ink-2">
          A closed board keeps its cards, read-only, in its space's log
          (Personal's in Irori's log), so it takes no space of its own. Its done
          cards clear with Clear done cards on Irori's log's page. Open one to
          read it or reopen it.
        </p>
        <ClosedBoards list={closed.list} error={closed.error} now={now} />
      </div>
    )

  if (tab && !loading && !kind)
    return (
      <div className="grid content-start gap-4">
        <BackToStorage onTab={onTab} />
        {error ? (
          <p className="text-xs text-destructive">
            Could not read what Irori keeps: {error}
          </p>
        ) : (
          <p className="text-[13px] text-muted-foreground">
            Irori keeps nothing called “{tab}” on this device.
          </p>
        )}
      </div>
    )
  if (tab && kind)
    return (
      <KindPage
        kind={kind}
        // The overview already lists these two (a long walk): no second one.
        list={
          kind.id === "builds"
            ? builds
            : kind.id === "claude"
              ? claude
              : undefined
        }
        onTab={onTab}
        onOpen={onOpen}
        now={now}
      />
    )
  return (
    <div className="grid content-start gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold text-ink">Storage</h1>
        <Button
          size="xs"
          variant="ghost"
          className="ml-auto"
          onClick={() => refreshStorage()}
        >
          <RefreshCwIcon className="size-3.5" aria-hidden />
          Measure again
        </Button>
      </div>
      <p className="max-w-3xl text-[13px] text-muted-foreground">
        Everything Irori keeps on this device, and how much space it takes.
        Nothing here is cleaned out by itself: open a kind to see what it holds
        and choose what goes.
      </p>
      {error && (
        <p className="text-xs text-destructive">
          Could not read what Irori keeps: {error}
        </p>
      )}
      {walkError && (
        <p className="text-xs text-destructive">
          Could not measure every folder: {walkError}
        </p>
      )}
      <Loading loading={loading} label="Measuring storage" className="min-w-0">
        <Overview kinds={kinds} closed={closed.list} onTab={onTab} />
      </Loading>
      {report.dir && (
        <p className="text-[12px] break-all text-muted-foreground">
          Irori's folder: <span className="font-mono">{report.dir}</span>
        </p>
      )}
    </div>
  )
}

function Overview({
  kinds,
  closed,
  onTab,
}: {
  kinds: StorageKind[]
  closed: ClosedBoard[]
  onTab: (tab: string) => void
}) {
  const sum = total(kinds)
  const measuring = kinds.some((k) => !k.measured)
  const ranked = [...kinds]
    .filter((k) => k.measured && k.bytes > 0)
    .sort((a, b) => b.bytes - a.bytes)
  const colour = (id: string) => {
    const i = ranked.findIndex((k) => k.id === id)
    return i < 0 ? "bg-muted" : barColours[Math.min(i, barColours.length - 1)]
  }
  const rows = [...kinds].sort(
    (a, b) => Number(b.measured) - Number(a.measured) || b.bytes - a.bytes
  )
  return (
    <div className="grid gap-3">
      <div className="grid gap-1.5">
        <p className="text-[13px] text-ink">
          <span className="text-xl font-semibold tabular-nums">
            {bytesText(sum)}
          </span>{" "}
          on this device{measuring ? ", still measuring some folders" : ""}
        </p>
        <div
          className="flex h-2.5 w-full overflow-hidden rounded-full bg-muted"
          role="img"
          aria-label={`Storage by kind: ${ranked
            .map((k) => `${infoOf(k.id).label} ${bytesText(k.bytes)}`)
            .join(", ")}`}
        >
          {sum > 0 &&
            ranked.map((k) => (
              <div
                key={k.id}
                className={cn("h-full", colour(k.id))}
                style={{ width: `${(k.bytes / sum) * 100}%` }}
              />
            ))}
        </div>
      </div>
      <ul
        className="overflow-hidden p-0! glass-panel"
        aria-label="What Irori keeps"
      >
        {rows.map((k) => {
          const info = infoOf(k.id)
          return (
            <li key={k.id} className="border-b border-border last:border-b-0">
              <button
                type="button"
                onClick={() => onTab(k.id)}
                className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-secondary/50"
              >
                <span
                  className={cn("size-2.5 shrink-0 rounded-full", colour(k.id))}
                  aria-hidden
                />
                <span className="grid min-w-0 flex-1 gap-0.5">
                  <span className="text-[13.5px] font-medium text-ink">
                    {info.label}
                  </span>
                  <span className="truncate text-[12px] text-muted-foreground">
                    {info.what}
                  </span>
                </span>
                <span className="grid shrink-0 justify-items-end gap-0.5">
                  <span className="text-[13px] text-ink tabular-nums">
                    {k.measured ? bytesText(k.bytes) : "Measuring…"}
                  </span>
                  {k.measured && k.items > 0 && (
                    <span className="text-[11.5px] text-muted-foreground tabular-nums">
                      {k.items.toLocaleString()}{" "}
                      {k.items === 1 ? "item" : "items"}
                    </span>
                  )}
                </span>
                <ChevronRightIcon
                  className="size-4 shrink-0 text-muted-foreground"
                  aria-hidden
                />
              </button>
            </li>
          )
        })}
      </ul>
      <ul className="overflow-hidden p-0! glass-panel" aria-label="Archived">
        <li>
          <button
            type="button"
            onClick={() => onTab("closed")}
            className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-secondary/50"
          >
            <span className="size-2.5 shrink-0" aria-hidden />
            <span className="grid min-w-0 flex-1 gap-0.5">
              <span className="text-[13.5px] font-medium text-ink">
                Closed boards
              </span>
              <span className="truncate text-[12px] text-muted-foreground">
                Boards you closed, kept read-only with their cards, in the logs
                above.
              </span>
            </span>
            <span className="shrink-0 text-[13px] text-ink tabular-nums">
              {closed.length} {closed.length === 1 ? "board" : "boards"}
            </span>
            <ChevronRightIcon
              className="size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
          </button>
        </li>
      </ul>
    </div>
  )
}

function BackToStorage({ onTab }: { onTab: (tab: string) => void }) {
  return (
    <div>
      <Button size="xs" variant="ghost" onClick={() => onTab("")}>
        <ArrowLeftIcon className="size-3.5" aria-hidden />
        Storage
      </Button>
    </div>
  )
}

// Listed is a kind's items as a hook returns them.
type Listed = { items: StorageItem[]; loading: boolean; error: string | null }

function KindPage({
  kind,
  list,
  onTab,
  onOpen,
  now,
}: {
  kind: StorageKind
  list?: Listed
  onTab: (tab: string) => void
  onOpen: (cardId: string) => void
  now: Date
}) {
  const info = infoOf(kind.id)
  return (
    <div className="grid content-start gap-4">
      <BackToStorage onTab={onTab} />
      <div className="flex flex-wrap items-baseline gap-3">
        <h1 className="text-lg font-semibold text-ink">{info.label}</h1>
        <span className="text-[13px] text-muted-foreground tabular-nums">
          {kind.measured ? bytesText(kind.bytes) : "Measuring…"}
        </span>
      </div>
      <div className="grid max-w-3xl gap-1.5 text-[13px]">
        <p className="text-ink-2">{info.what}</p>
        <p
          className={cn(
            "flex gap-1.5",
            info.cleanable ? "text-ink-2" : "text-muted-foreground"
          )}
        >
          {info.cleanable && (
            <AlertTriangleIcon
              className="mt-0.5 size-3.5 shrink-0 text-warning"
              aria-hidden
            />
          )}
          <span>{info.clean}</span>
        </p>
        {kind.path && (
          <p className="text-[12px] break-all text-muted-foreground">
            <span className="font-mono">{kind.path}</span>
          </p>
        )}
      </div>
      {kind.id === "log" ? (
        <LogPage onOpen={onOpen} now={now} />
      ) : kind.id === "capture" ? null : (
        <ItemsList kind={kind.id} list={list} onOpen={onOpen} now={now} />
      )}
    </div>
  )
}

// ItemsList is one kind's items, with Delete for the ones that may go.
function ItemsList({
  kind,
  list,
  onOpen,
  now,
}: {
  kind: string
  list?: Listed
  onOpen: (cardId: string) => void
  now: Date
}) {
  const own = useStorageItems(list ? "" : kind)
  const { items, loading, error } = list ?? own
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [done, setDone] = useState<StorageCleaned | null>(null)
  const info = infoOf(kind)
  // Only picks still listed and deletable count: a clean or a refresh may
  // have taken some.
  const pickedItems = items.filter((i) => i.cleanable && picked.has(i.id))
  const free = cleanableBytes(items)
  const goes = items.filter((i) => i.cleanable).length
  const onlyHere = (list: StorageItem[]) => list.some((i) => i.onlyHere)
  const toggle = (id: string, on: boolean) =>
    setPicked((p) => {
      const n = new Set(p)
      if (on) n.add(id)
      else n.delete(id)
      return n
    })

  const question = (list: StorageItem[], all: boolean) => (
    <>
      Delete{" "}
      {all
        ? "every item that can go"
        : list.length === 1
          ? `“${list[0].name}”`
          : `${list.length} items`}{" "}
      ({bytesText(cleanableBytes(list))})? This can't be undone. {info.clean}
      {onlyHere(list) && (
        <>
          {" "}
          Some of these were attached on this device, which may hold the only
          copy.
        </>
      )}
    </>
  )

  return (
    <section aria-label={info.label} className="grid gap-2">
      {error && (
        <p className="text-xs text-destructive">Could not list them: {error}</p>
      )}
      {info.cleanable && (
        <div className="flex flex-wrap items-center gap-2">
          <ConfirmButton
            label={
              pickedItems.length
                ? `Delete ${pickedItems.length} selected…`
                : "Delete selected…"
            }
            disabled={!pickedItems.length}
            variant="destructive"
            confirmLabel={`Delete ${pickedItems.length}`}
            question={question(pickedItems, false)}
            onConfirm={async () => {
              setDone(
                await storage.clean(
                  kind,
                  pickedItems.map((i) => i.id)
                )
              )
              setPicked(new Set())
            }}
          />
          <ConfirmButton
            label={
              goes
                ? `Delete all that can go (${bytesText(free)})…`
                : "Nothing to delete"
            }
            disabled={!goes}
            variant="destructive"
            confirmLabel="Delete all"
            question={question(
              items.filter((i) => i.cleanable),
              true
            )}
            onConfirm={async () => {
              // The ones the confirm showed, never what turned up since.
              setDone(
                await storage.clean(
                  kind,
                  items.filter((i) => i.cleanable).map((i) => i.id)
                )
              )
              setPicked(new Set())
            }}
          />
          {done && <CleanedLine done={done} items={items} />}
        </div>
      )}
      <Loading
        loading={loading}
        label={`Listing ${info.label.toLowerCase()}`}
        className="min-w-0"
      >
        {items.length === 0 ? (
          <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-[13px] text-muted-foreground">
            {loading ? "Listing…" : "Nothing here."}
          </p>
        ) : (
          <ul className="overflow-hidden p-0! glass-panel" aria-label="Items">
            {items.map((i) => (
              <ItemRow
                key={i.id}
                i={i}
                pickable={info.cleanable}
                picked={picked.has(i.id)}
                onPick={(on) => toggle(i.id, on)}
                onOpen={onOpen}
                now={now}
              />
            ))}
          </ul>
        )}
      </Loading>
    </section>
  )
}

function CleanedLine({
  done,
  items,
}: {
  done: StorageCleaned
  items: StorageItem[]
}) {
  const name = (id: string) => items.find((i) => i.id === id)?.name ?? id
  return (
    <span className="text-xs text-muted-foreground" role="status">
      {done.removed
        ? `Deleted ${done.removed}, freed ${bytesText(done.freed)}.`
        : "Nothing was deleted."}
      {done.kept.length > 0 &&
        ` Kept ${done.kept.map((k) => `${name(k.id)} (${k.why})`).join("; ")}.`}
    </span>
  )
}

function ItemRow({
  i,
  pickable,
  picked,
  onPick,
  onOpen,
  now,
}: {
  i: StorageItem
  pickable: boolean
  picked: boolean
  onPick: (on: boolean) => void
  onOpen: (cardId: string) => void
  now: Date
}) {
  const at = i.at && !i.at.startsWith("0001") ? new Date(i.at) : null
  const [busy, setBusy] = useState(false)
  const [fail, setFail] = useState("")
  // End it and Delete anyway: the list refreshes through storage's change
  // feed, so a failure is all this row has to say.
  const run = (f: () => Promise<void>) => {
    setBusy(true)
    setFail("")
    f()
      .catch((err: unknown) => setFail(errorText(err)))
      .finally(() => setBusy(false))
  }
  return (
    <li className="flex items-center gap-3 border-b border-border px-4 py-2.5 last:border-b-0">
      {pickable && (
        <Checkbox
          checked={picked}
          disabled={!i.cleanable}
          onCheckedChange={(on) => onPick(on === true)}
          aria-label={`Select ${i.name}`}
        />
      )}
      <span className="grid min-w-0 flex-1 gap-0.5">
        <span className="truncate text-[13px] text-ink">{i.name}</span>
        <span className="truncate text-[12px] text-muted-foreground">
          {[
            i.detail,
            at ? relativeTime(at, now) : "",
            i.keep ? `Stays: ${i.keep}` : "",
            i.pinned ? `Kept: ${i.pinned}` : "",
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
        {fail && (
          <span className="text-[12px] text-destructive" role="alert">
            {fail}
          </span>
        )}
      </span>
      {i.holders.map((h) => (
        <Button
          key={h.pid}
          size="xs"
          variant="ghost"
          disabled={busy}
          onClick={() =>
            run(() => storage.endHolder(i.id, h.pid).then(() => undefined))
          }
          aria-label={`End ${h.name} (pid ${h.pid}), which holds ${i.name}`}
        >
          End it ({h.name})
        </Button>
      ))}
      {i.deleteAnyway && (
        <Button
          size="xs"
          variant="ghost"
          disabled={busy}
          onClick={() => {
            if (
              window.confirm(
                `Delete ${i.name} (${i.pinned}) and everything in it? The branch stays; unpushed work in the folder is lost.`
              )
            )
              run(() => storage.deleteTree(i.id))
          }}
          aria-label={`Delete ${i.name} anyway`}
        >
          Delete anyway
        </Button>
      )}
      {i.cards.length > 0 && (
        <Button
          size="xs"
          variant="ghost"
          onClick={() => onOpen(i.cards[0])}
          aria-label={`Open the card for ${i.name}`}
        >
          <ExternalLinkIcon className="size-3.5" aria-hidden />
          Card
        </Button>
      )}
      <span className="shrink-0 text-[13px] text-ink tabular-nums">
        {bytesText(i.bytes)}
      </span>
    </li>
  )
}

// LogPage is Irori's log's page: what its rows are, the clears that tidy
// the lists (Maintenance's, #138, and resolved threads), and the cleared
// cards and threads, with Put back.
function LogPage({
  onOpen,
  now,
}: {
  onOpen: (cardId: string) => void
  now: Date
}) {
  const loadParts = useCallback(() => storage.log(), [])
  const parts = useLatest<LogPart[]>(loadParts, [], "the log")
  useStorageFresh(parts.reload, false, "log-size")
  const sum = parts.value.reduce((n, p) => n + p.bytes, 0)
  return (
    <div className="grid gap-6">
      <section aria-label="What the log holds" className="grid gap-2">
        <h2 className="text-sm font-medium text-ink">What it holds</h2>
        {parts.error && (
          <p className="text-xs text-destructive">{parts.error}</p>
        )}
        <Loading
          loading={parts.loading}
          label="Reading the log"
          className="min-w-0"
        >
          <ul
            className="overflow-hidden p-0! glass-panel"
            aria-label="The log by kind"
          >
            {parts.value.map((p) => (
              <li
                key={p.part}
                className="grid gap-1 border-b border-border px-4 py-2.5 last:border-b-0"
              >
                <span className="flex items-baseline gap-3">
                  <span className="flex-1 text-[13px] text-ink">
                    {partLabels[p.part] ?? p.part}
                  </span>
                  <span className="text-[12px] text-muted-foreground tabular-nums">
                    {p.rows.toLocaleString()} {p.rows === 1 ? "row" : "rows"}
                  </span>
                  <span className="w-20 text-right text-[13px] text-ink tabular-nums">
                    {bytesText(p.bytes)}
                  </span>
                </span>
                <span className="h-1.5 overflow-hidden rounded-full bg-muted">
                  <span
                    className="block h-full rounded-full bg-chart-1"
                    style={{ width: sum ? `${(p.bytes / sum) * 100}%` : 0 }}
                  />
                </span>
              </li>
            ))}
          </ul>
        </Loading>
        <p className="text-[12px] text-muted-foreground">
          Sizes are the rows' own; the file is larger by its indexes.
        </p>
      </section>

      <section aria-label="Clear done cards" className="grid gap-2">
        <h2 className="text-sm font-medium text-ink">Clear done cards</h2>
        <p className="max-w-3xl text-[12.5px] text-muted-foreground">
          Cleared cards leave the app's lists, here and in your spaces. They
          stay in Irori's log, so agents still find them with search and show,
          and you can put them back below. To clear or delete single cards, use
          Select on Closed.
        </p>
        <ClearDone />
      </section>

      <section aria-label="Clear resolved threads" className="grid gap-2">
        <h2 className="text-sm font-medium text-ink">Clear resolved threads</h2>
        <p className="max-w-3xl text-[12.5px] text-muted-foreground">
          Resolved threads leave Talk's list the same way. They stay in the log,
          and you can put them back below.
        </p>
        <ClearThreads />
      </section>

      <ClearedList onOpen={onOpen} now={now} />
    </div>
  )
}

// ClearThreads clears resolved threads by age, as ClearDone does cards.
function ClearThreads() {
  const [age, setAge] = useState<(typeof clearAges)[number]["id"]>("week")
  const [count, setCount] = useState<number | null>(null)
  const [last, setLast] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const pick = clearAges.find((a) => a.id === age) ?? clearAges[1]
  // Counted again on each change here (Put back) and on board:changed.
  const recount = useCallback(
    () =>
      storage
        .clearThreadsCount(pick.secs)
        .then((n) => {
          setCount(n)
          setError(null)
        })
        .catch((err: unknown) => setError(errorText(err))),
    [pick.secs]
  )
  useStorageFresh(recount, true, "log")
  const plural = (n: number) =>
    `${n} resolved ${n === 1 ? "thread" : "threads"}`
  const what = pick.phrase
    ? `${plural(count ?? 0)} ${pick.phrase}`
    : `all ${plural(count ?? 0)}`
  return (
    <div className="grid gap-2.5">
      <div
        className="flex flex-wrap items-center gap-1"
        role="group"
        aria-label="Clear resolved threads by age"
      >
        {clearAges.map((a) => (
          <Button
            key={a.id}
            size="xs"
            aria-pressed={age === a.id}
            variant={age === a.id ? "secondary" : "ghost"}
            onClick={() => {
              setAge(a.id)
              setLast(null)
            }}
          >
            {a.label}
          </Button>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[12.5px] text-muted-foreground tabular-nums">
          {count === null
            ? "Counting…"
            : count === 0
              ? "Nothing to clear."
              : `Clears ${what}.`}
        </span>
        <ConfirmButton
          label={count ? `Clear ${count}…` : "Clear"}
          disabled={!count}
          confirmLabel={count ? `Clear ${count}` : "Clear"}
          question={
            <>
              Clear {what}? They leave Talk's list. Agents still find them with
              search and show. Nothing is deleted.
            </>
          }
          onConfirm={async () => {
            setLast(await storage.clearThreads(pick.secs))
          }}
        />
        {last !== null && (
          <span className="text-xs text-muted-foreground" role="status">
            Cleared {plural(last)}.
          </span>
        )}
        {error && <span className="text-xs text-destructive">{error}</span>}
      </div>
    </div>
  )
}

// ClearedList is what the person cleared, to look through and put back.
function ClearedList({
  onOpen,
  now,
}: {
  onOpen: (cardId: string) => void
  now: Date
}) {
  const load = useCallback(() => storage.cleared(), [])
  const { value, loading, error, reload } = useLatest<ClearedCard[]>(
    load,
    [],
    "cleared cards"
  )
  useStorageFresh(reload, true, "log")
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [shown, setShown] = useState(50)
  return (
    <section aria-label="Cleared cards and threads" className="grid gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-sm font-medium text-ink">
          Cleared cards and threads
        </h2>
        <span className="text-[12px] text-muted-foreground tabular-nums">
          {value.length}
        </span>
        <span className="ml-auto">
          <ConfirmButton
            label={picked.size ? `Put back ${picked.size}…` : "Put back…"}
            disabled={!picked.size}
            confirmLabel={`Put back ${picked.size}`}
            question={`Put ${picked.size} back in the app's lists?`}
            onConfirm={async () => {
              await storage.unclear([...picked])
              setPicked(new Set())
            }}
          />
        </span>
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}
      <Loading
        loading={loading}
        label="Listing cleared cards"
        className="min-w-0"
      >
        {value.length === 0 ? (
          <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-[13px] text-muted-foreground">
            Nothing cleared.
          </p>
        ) : (
          <ul className="overflow-hidden p-0! glass-panel" aria-label="Cleared">
            {value.slice(0, shown).map((c) => (
              <li
                key={c.id}
                className="flex items-center gap-3 border-b border-border px-4 py-2 last:border-b-0"
              >
                <Checkbox
                  checked={picked.has(c.id)}
                  onCheckedChange={(on) =>
                    setPicked((p) => {
                      const n = new Set(p)
                      if (on === true) n.add(c.id)
                      else n.delete(c.id)
                      return n
                    })
                  }
                  aria-label={`Select ${c.short}`}
                />
                <button
                  type="button"
                  onClick={() => onOpen(c.id)}
                  className="grid min-w-0 flex-1 gap-0.5 text-left"
                >
                  <span className="truncate text-[13px] text-ink">
                    <span className="mr-1.5 text-muted-foreground tabular-nums">
                      {c.short}
                    </span>
                    {c.title}
                  </span>
                  <span className="text-[12px] text-muted-foreground">
                    {[
                      c.thread ? "Thread" : "Card",
                      c.board || "",
                      relativeTime(new Date(c.updatedAt), now),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Loading>
      {value.length > shown && (
        <div>
          <Button
            size="xs"
            variant="ghost"
            onClick={() => setShown((n) => n + 100)}
          >
            Show more ({value.length - shown} left)
          </Button>
        </div>
      )}
    </section>
  )
}
