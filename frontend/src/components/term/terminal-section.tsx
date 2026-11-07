// The card page's Terminal section (card #369; inline-terminals.md 1, 6.1):
// the card's live shells on this device, as tabs. Nothing starts a shell
// but New terminal (Stephen, #369 b5), and a card can hold several. The
// shells belong to the Core, so leaving the page leaves them running and
// coming back shows them again.
//
// The bottom terminal panel (card #734) shows the same section: a card's
// shells on its page, else the person's own shells on this device (the
// device scope), the first opened as the panel opens, in the folder the
// last one was in.

import { type ReactNode, useEffect, useState } from "react"
import { PlusIcon, SquareTerminalIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { cn } from "cn"
import {
  type Folder,
  type TermInfo,
  cardFolders,
  cardTerms,
  closeTerm,
  firstDeviceTerm,
  openCardTerm,
  openDeviceTerm,
  pickFolder,
  rememberFolder,
} from "@/lib/term"
import { lastFolder, rememberLastFolder } from "@/lib/term-panel"

import { TermOpened, type TermOpenedDetail } from "@/lib/agents"

import { Terminal } from "./terminal"

// The size a shell starts at, before the view measures its box.
const StartCols = 100
const StartRows = 24

// fill: the section takes its container's height (the bottom panel).
export function TerminalSection({
  cardId,
  fill = false,
}: {
  cardId: string
  fill?: boolean
}) {
  return <CardTerminals key={cardId} cardId={cardId} fill={fill} />
}

function CardTerminals({ cardId, fill }: { cardId: string; fill: boolean }) {
  const [terms, setTerms] = useState<TermInfo[] | null>(null)
  const [shown, setShown] = useState<number | null>(null)
  // The New terminal row: the folders offered, and the one chosen.
  const [folders, setFolders] = useState<Folder[] | null>(null)
  const [choice, setChoice] = useState("")
  const [picking, setPicking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  // The shells already running for this card (none are started here).
  useEffect(() => {
    let live = true
    Promise.all([cardTerms(cardId), cardFolders(cardId)])
      .then(([ts, fs]) => {
        if (!live) return
        setTerms(ts)
        setShown(ts.length ? ts[ts.length - 1].id : null)
        setFolders(fs)
        setChoice(pickFolder(cardId, fs) ?? fs[0]?.path ?? "")
      })
      .catch((e: Error) => live && setError(e.message))
    return () => {
      live = false
    }
  }, [cardId])

  // Attach on an agent Irori started (card #456) opened a tab here.
  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<TermOpenedDetail>).detail
      if (d?.cardId !== cardId) return
      setTerms((ts) =>
        ts?.some((t) => t.id === d.term.id) ? ts : [...(ts ?? []), d.term]
      )
      setShown(d.term.id)
    }
    window.addEventListener(TermOpened, on)
    return () => window.removeEventListener(TermOpened, on)
  }, [cardId])

  const start = (cwd: string) => {
    setBusy(true)
    setError(null)
    openCardTerm(cardId, StartCols, StartRows, cwd || null)
      .then((t) => {
        if (cwd) rememberFolder(cardId, cwd)
        setTerms((ts) => [...(ts ?? []), t])
        setShown(t.id)
        setPicking(false)
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false))
  }

  // New terminal: straight away when the folder is known, else the picker.
  const newTerm = () => {
    const pick = folders ? pickFolder(cardId, folders) : null
    if (pick) start(pick)
    else setPicking(true)
  }

  const close = (id: number) => {
    closeTerm(id)
      .catch(() => {})
      .finally(() => {
        setTerms((ts) => {
          const left = (ts ?? []).filter((t) => t.id !== id)
          if (shown === id)
            setShown(left.length ? left[left.length - 1].id : null)
          return left
        })
      })
  }

  return (
    <Shells
      terms={terms}
      shown={shown}
      onShow={setShown}
      onClose={close}
      onNew={newTerm}
      newDisabled={busy || folders === null}
      onRestart={(t) => {
        close(t.id)
        start(t.cwd)
      }}
      fill={fill}
      empty={!picking}
      error={error}
      emptyText="No shell for this card on this device."
    >
      {picking && folders !== null && (
        <div className="flex flex-wrap items-center gap-2">
          {folders.length > 1 ? (
            <NativeSelect
              size="sm"
              value={choice}
              onChange={(e) => setChoice(e.target.value)}
              aria-label="Folder"
              className="max-w-full"
            >
              {folders.map((f) => (
                <NativeSelectOption key={f.path} value={f.path}>
                  {f.path}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          ) : (
            <Input
              className="h-8 max-w-md font-mono text-[12.5px]"
              placeholder="No checkout of this card's repo here: a folder path"
              value={choice}
              onChange={(e) => setChoice(e.target.value)}
              aria-label="Folder"
            />
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={busy || !choice.trim()}
            onClick={() => start(choice.trim())}
          >
            Open
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setPicking(false)}>
            Cancel
          </Button>
        </div>
      )}
    </Shells>
  )
}

// DeviceTerminals is the person's own shells on this device, tied to no
// card: the terminal panel's off a card's page (card #734). Mounting it is
// the person asking for a shell (they opened the panel), so with none
// running it starts one, in the folder the last one was in. New terminal
// opens another there; closing the last one starts nothing.
export function DeviceTerminals({ fill = false }: { fill?: boolean }) {
  const [terms, setTerms] = useState<TermInfo[] | null>(null)
  const [shown, setShown] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let live = true
    firstDeviceTerm(StartCols, StartRows, lastFolder())
      .then((ts) => {
        if (!live) return
        setTerms(ts)
        setShown(ts.length ? ts[ts.length - 1].id : null)
      })
      .catch((e: Error) => live && setError(e.message))
    return () => {
      live = false
    }
  }, [])

  const start = (cwd: string | null) => {
    setBusy(true)
    setError(null)
    openDeviceTerm(StartCols, StartRows, cwd)
      .then((t) => {
        setTerms((ts) => [...(ts ?? []), t])
        setShown(t.id)
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false))
  }

  const close = (id: number) => {
    closeTerm(id)
      .catch(() => {})
      .finally(() => {
        setTerms((ts) => {
          const left = (ts ?? []).filter((t) => t.id !== id)
          if (shown === id)
            setShown(left.length ? left[left.length - 1].id : null)
          return left
        })
      })
  }

  return (
    <Shells
      terms={terms}
      shown={shown}
      onShow={setShown}
      onClose={close}
      onNew={() => start(lastFolder())}
      newDisabled={busy || terms === null}
      // Where the shell was last, not where it started.
      onRestart={(t) => {
        close(t.id)
        start(lastFolder() ?? t.cwd)
      }}
      onCwd={rememberLastFolder}
      fill={fill}
      empty
      error={error}
      emptyText="No shell open on this device."
    />
  )
}

// Shells is the tab row, New terminal, and the shown shell.
function Shells({
  terms,
  shown,
  onShow,
  onClose,
  onNew,
  newDisabled,
  onRestart,
  onCwd,
  fill,
  empty,
  error,
  emptyText,
  children,
}: {
  terms: TermInfo[] | null
  shown: number | null
  onShow: (id: number) => void
  onClose: (id: number) => void
  onNew: () => void
  newDisabled: boolean
  onRestart: (t: TermInfo) => void
  onCwd?: (cwd: string) => void
  fill: boolean
  // empty: say so when there are no shells (not while picking a folder).
  empty: boolean
  error: string | null
  emptyText: string
  children?: ReactNode
}) {
  const current = terms?.find((t) => t.id === shown) ?? null
  return (
    <div
      className={cn("gap-2", fill ? "flex h-full min-h-0 flex-col" : "grid")}
    >
      <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-1">
        {terms?.map((t, i) => (
          <div
            key={t.id}
            className={cn(
              "flex items-center rounded-md border text-[12px]",
              t.id === shown
                ? "border-border bg-muted text-ink-2"
                : "border-transparent text-muted-foreground"
            )}
          >
            <button
              type="button"
              className="flex max-w-56 items-center gap-1.5 truncate px-2 py-1"
              onClick={() => onShow(t.id)}
              title={t.title ? `${t.title}\n${t.cwd}` : t.cwd}
            >
              <SquareTerminalIcon className="size-3.5 shrink-0" aria-hidden />
              <span className="truncate">{tabName(t, i)}</span>
            </button>
            <button
              type="button"
              className="px-1 py-1 text-muted-foreground hover:text-foreground"
              onClick={() => onClose(t.id)}
              title="End this shell"
              aria-label="End this shell"
            >
              <XIcon className="size-3" aria-hidden />
            </button>
          </div>
        ))}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-muted-foreground"
          disabled={newDisabled}
          onClick={onNew}
        >
          <PlusIcon aria-hidden />
          New terminal
        </Button>
      </div>

      {children}

      {current && (
        <Terminal
          key={current.id}
          id={current.id}
          cwd={current.cwd}
          fill={fill}
          onCwd={onCwd}
          onRestart={() => onRestart(current)}
        />
      )}
      {terms !== null && terms.length === 0 && empty && (
        <p className="text-[12.5px] text-muted-foreground">{emptyText}</p>
      )}
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </div>
  )
}

function tabName(t: TermInfo, i: number): string {
  const base = t.cwd.split(/[\\/]/).filter(Boolean).pop() ?? "shell"
  return i === 0 ? base : `${base} ${i + 1}`
}
