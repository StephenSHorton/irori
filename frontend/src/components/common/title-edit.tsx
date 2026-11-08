// TitleEdit is a card's or thread's title that the person renames in
// place (card #779): a click or tap on it opens a field, Enter or Save
// saves, Esc or Cancel leaves it. The new name shows at once and stays
// unless the save is refused, which says why under the field. An agent's
// later rename doesn't land (the host keeps a name the person gave).

import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"
import { PencilIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// The host's limit on a card's title (coord MAX_TITLE).
export const MaxTitle = 200

// nextTitle is what a save sends: trimmed, and null when it is empty or
// says what the title already does (nothing to save). The field shows the
// title on one line, so that spelling of it is no change either: opening
// it and saving must not make the name the person's.
export function nextTitle(draft: string, now: string): string | null {
  const t = draft.trim()
  if (!t || t === now.trim() || t === plainLine(now).trim()) return null
  return t
}

export function TitleEdit({
  title,
  save,
  as: Heading = "h1",
  className,
  inputClassName,
  oneLine,
  formClassName,
  label = "Title",
  editing = false,
  children,
}: {
  title: string
  // Saves the new title; a rejection is shown under the field.
  save: (title: string) => Promise<unknown>
  as?: "h1" | "h2"
  // The heading's look, as it was before it could be renamed.
  className?: string
  inputClassName?: string
  // A one-line header: the title is cut with "…", the pencil stays.
  oneLine?: boolean
  // Where the field sits while it is open (a row of its own on a phone).
  formClassName?: string
  label?: string
  // Opens with the field showing (the preview's ?rename=<id>).
  editing?: boolean
  // Shown in place of the title text (the "(untitled)" fallback).
  children?: ReactNode
}) {
  const [draft, setDraft] = useState<string | null>(() =>
    editing ? plainLine(title) : null
  )
  // The name just saved, shown until the card says it too.
  const [saved, setSaved] = useState<{ was: string; now: string } | null>(null)
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  // Focus goes back to the title when the field closes (keyboard users).
  const button = useRef<HTMLButtonElement>(null)
  const refocus = useRef(false)
  useEffect(() => {
    if (draft === null && refocus.current) {
      refocus.current = false
      button.current?.focus()
    }
  }, [draft])
  const close = () => {
    refocus.current = true
    setDraft(null)
  }
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
      close()
      return
    }
    setBusy(true)
    setSaved({ was: title, now: t })
    close()
    try {
      await save(t)
      setError("")
    } catch (err) {
      setSaved(null)
      // Back to the field with what they typed, unless they reopened it.
      setDraft((d) => d ?? t)
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  if (draft !== null) {
    return (
      <form
        className={cn("grid min-w-0 flex-1 gap-1", formClassName)}
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
              if (e.key === "Escape" && !e.nativeEvent.isComposing) {
                e.preventDefault()
                e.stopPropagation()
                close()
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
              close()
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
      {/* The heading's name stays the title; Rename is its hint. */}
      <button
        ref={button}
        type="button"
        title="Rename"
        data-title-edit=""
        className={cn(
          "max-w-full cursor-text rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
          oneLine ? "flex min-w-0 items-center" : "[overflow-wrap:anywhere]"
        )}
        onClick={open}
      >
        <span className={cn(oneLine && "min-w-0 truncate")}>
          {saved ? plainLine(shown) : (children ?? plainLine(shown))}
        </span>
        <PencilIcon
          aria-hidden
          className={cn(
            "ml-1.5 size-[0.7em] text-muted-foreground opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-60",
            oneLine ? "shrink-0" : "inline align-baseline"
          )}
        />
      </button>
    </Heading>
  )
}
