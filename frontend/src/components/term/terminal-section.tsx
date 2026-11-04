// The card page's Terminal section (card #369; inline-terminals.md 1, 6.1):
// the card's live shells on this device, as tabs. Nothing starts a shell
// but New terminal (Stephen, #369 b5), and a card can hold several. The
// shells belong to the Core, so leaving the page leaves them running and
// coming back shows them again.

import { useEffect, useState } from "react"
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
  openCardTerm,
  pickFolder,
  rememberFolder,
} from "@/lib/term"

import { TermOpened, type TermOpenedDetail } from "@/lib/agents"

import { Terminal } from "./terminal"

// The size a shell starts at, before the view measures its box.
const StartCols = 100
const StartRows = 24

export function TerminalSection({ cardId }: { cardId: string }) {
  return <CardTerminals key={cardId} cardId={cardId} />
}

function CardTerminals({ cardId }: { cardId: string }) {
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

  const current = terms?.find((t) => t.id === shown) ?? null

  return (
    <div className="grid gap-2">
      <div className="flex min-w-0 flex-wrap items-center gap-1">
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
              onClick={() => setShown(t.id)}
              title={t.title ? `${t.title}\n${t.cwd}` : t.cwd}
            >
              <SquareTerminalIcon className="size-3.5 shrink-0" aria-hidden />
              <span className="truncate">{tabName(t, i)}</span>
            </button>
            <button
              type="button"
              className="px-1 py-1 text-muted-foreground hover:text-foreground"
              onClick={() => close(t.id)}
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
          disabled={busy || folders === null}
          onClick={newTerm}
        >
          <PlusIcon aria-hidden />
          New terminal
        </Button>
      </div>

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

      {current && (
        <Terminal
          key={current.id}
          id={current.id}
          cwd={current.cwd}
          onRestart={() => {
            close(current.id)
            start(current.cwd)
          }}
        />
      )}
      {terms !== null && terms.length === 0 && !picking && (
        <p className="text-[12.5px] text-muted-foreground">
          No shell for this card on this device.
        </p>
      )}
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </div>
  )
}

function tabName(t: TermInfo, i: number): string {
  const base = t.cwd.split(/[\\/]/).filter(Boolean).pop() ?? "shell"
  return i === 0 ? base : `${base} ${i + 1}`
}
