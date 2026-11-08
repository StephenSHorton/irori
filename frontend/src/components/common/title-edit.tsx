// TitleEdit is a card's or thread's title that the person renames in
// place (card #779): a click or tap on it opens a field, Enter or Save
// saves, Esc or Cancel leaves it. The new name shows at once and stays
// unless the save is refused, which says why under the field. An agent's
// later rename doesn't land (the host keeps a name the person gave).

import { useState, type FormEvent, type ReactNode } from "react"
import { PencilIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// The host's limit on a card's title (coord MAX_TITLE).
export const MaxTitle = 200

// nextTitle is what a save sends: trimmed, and null when it is empty or
// says what the title already does (nothing to save).
export function nextTitle(draft: string, now: string): string | null {
  const t = draft.trim()
  if (!t || t === now.trim()) return null
  return t
}

export function TitleEdit({
  title,
  save,
  as: Heading = "h1",
  className,
  inputClassName,
  buttonClassName,
  label = "Title",
  children,
}: {
  title: string
  // Saves the new title; a rejection is shown under the field.
  save: (title: string) => Promise<unknown>
  as?: "h1" | "h2"
  // The heading's look, as it was before it could be renamed.
  className?: string
  inputClassName?: string
  // The title's own look: "block truncate" for a one-line header.
  buttonClassName?: string
  label?: string
  // Shown in place of the title text (the "(untitled)" fallback).
  children?: ReactNode
}) {
  const [draft, setDraft] = useState<string | null>(null)
  // The name just saved, shown until the card says it too.
  const [saved, setSaved] = useState<{ was: string; now: string } | null>(null)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  // A newer title from the host (the save, or someone else's) wins.
  const shown = saved && saved.was === title ? saved.now : title
  if (saved && saved.was !== title) setSaved(null)

  const open = () => {
    setError("")
    setDraft(plainLine(shown))
  }
  const submit = async (e?: FormEvent) => {
    e?.preventDefault()
    if (busy || draft === null) return
    const t = nextTitle(draft, shown)
    if (t === null) {
      if (!draft.trim()) {
        setError("A card needs a title.")
        return
      }
      setDraft(null)
      return
    }
    setBusy(true)
    setSaved({ was: title, now: t })
    setDraft(null)
    try {
      await save(t)
      setError("")
    } catch (err) {
      setSaved(null)
      setDraft(t)
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  if (draft !== null) {
    return (
      <form
        className="grid min-w-0 flex-1 gap-1"
        onSubmit={(e) => void submit(e)}
      >
        <span className="flex min-w-0 items-center gap-1.5">
          <Input
            autoFocus
            value={draft}
            maxLength={MaxTitle}
            aria-label={label}
            className={cn("h-8 min-w-0 flex-1", inputClassName)}
            onFocus={(e) => e.currentTarget.select()}
            onChange={(e) => {
              setDraft(e.target.value)
              if (error) setError("")
            }}
            onKeyDown={(e) => {
              // Esc is Back on the card page: here it only leaves the field.
              if (e.key === "Escape") {
                e.preventDefault()
                e.stopPropagation()
                setDraft(null)
                setError("")
              }
            }}
          />
          <Button type="submit" size="sm" className="h-8" disabled={busy}>
            Save
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8"
            onClick={() => {
              setDraft(null)
              setError("")
            }}
          >
            Cancel
          </Button>
        </span>
        {error && (
          <small role="alert" className="text-xs text-destructive">
            {error}
          </small>
        )}
      </form>
    )
  }
  return (
    <Heading className={cn("group min-w-0", className)}>
      <button
        type="button"
        title="Rename"
        aria-label={`Rename: ${plainLine(shown)}`}
        className={cn(
          "max-w-full cursor-text rounded-sm text-left [overflow-wrap:anywhere] outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
          buttonClassName
        )}
        onClick={open}
      >
        {saved ? plainLine(shown) : (children ?? plainLine(shown))}
        <PencilIcon
          aria-hidden
          className="ml-1.5 inline size-[0.7em] align-baseline text-muted-foreground opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-60"
        />
      </button>
    </Heading>
  )
}
