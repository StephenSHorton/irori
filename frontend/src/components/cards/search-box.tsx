// SearchBox is the Board's search (card #171): it sits left of the
// List/Canvas switch and narrows the cards shown, List and Canvas alike,
// as you type (lib/search). "/" anywhere outside a text field puts the
// cursor in it; Esc or the x clears it.

import { useEffect, useRef } from "react"
import { SearchIcon, XIcon } from "lucide-react"

import { Kbd } from "@/components/ui/kbd"
import { useCommands } from "@/lib/commands"
import { openDialogs, SearchShortcut } from "@/lib/app-shortcuts"
import { cn } from "@/lib/utils"

export function SearchBox({
  query,
  onQuery,
  className,
}: {
  query: string
  onQuery: (q: string) => void
  className?: string
}) {
  const box = useRef<HTMLInputElement>(null)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.defaultPrevented || e.isComposing) return
      if (e.metaKey || e.ctrlKey || e.altKey) return
      const t = e.target instanceof HTMLElement ? e.target : null
      if (t?.closest("input,textarea,select,[contenteditable]")) return
      if (document.querySelector(openDialogs)) return
      e.preventDefault()
      box.current?.focus()
      box.current?.select()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
  useCommands([
    {
      id: "search.cards",
      title: "Search cards on this page",
      // Cmd+F (card #733); "/" works too, and the box itself shows it.
      shortcut: SearchShortcut,
      keywords: ["find", "filter"],
      run: () => {
        box.current?.focus()
        box.current?.select()
      },
    },
  ])
  return (
    <div
      className={cn("relative flex w-56 items-center", className)}
      role="search"
    >
      <SearchIcon
        className="pointer-events-none absolute left-2 size-3.5 text-muted-foreground"
        aria-hidden
      />
      <input
        ref={box}
        type="text"
        value={query}
        onChange={(e) => onQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            onQuery("")
            box.current?.blur()
          }
        }}
        placeholder="Search cards"
        aria-label="Search cards"
        spellCheck={false}
        autoComplete="off"
        className="h-8 w-full min-w-0 rounded-[min(var(--radius-md),12px)] border border-border bg-background/60 pr-7 pl-7 text-[13px] outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring"
      />
      {query ? (
        <button
          type="button"
          className="absolute right-1 flex size-5 items-center justify-center text-muted-foreground hover:text-foreground"
          aria-label="Clear the search"
          title="Clear (Esc)"
          onClick={() => {
            onQuery("")
            box.current?.focus()
          }}
        >
          <XIcon className="size-3.5" aria-hidden />
        </button>
      ) : (
        <Kbd className="absolute right-1.5 h-4 min-w-4 rounded-none px-1 text-[10px]">
          /
        </Kbd>
      )}
    </div>
  )
}
