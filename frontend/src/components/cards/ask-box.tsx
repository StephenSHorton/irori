import { useEffect, useRef, useState, type FormEvent } from "react"
import { PaperclipIcon, XIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useBoards } from "@/lib/boards"
import { cards, MaxComment, MaxUploadFiles, type AskRepo } from "@/lib/cards"
import { useDraft } from "@/lib/drafts"
import { errorText } from "@/lib/talk"

// AskBox is the person talking to the room (docs/design/board.md "Asks"):
// one line over the grid. What they type goes to every free agent, one of
// them takes it, and it shows first in the grid as an ask until it becomes
// a card or closes. It is the card comment box lifted out: Enter sends,
// Shift+Enter is a new line. It always wakes them: only idle agents hear
// an ask, and an idle agent gets no next turn until someone types in its
// window. The repo chip keeps it to one repo's agents. Files go with it: pick them with the clip, drop them
// on the box, or paste a screenshot (card #25). The window stays on the
// grid. The board chip posts it to a shared board instead (v0.5 §9, card
// #59): every member's free agents hear it, the other people's through
// their own devices. A board's agents carry no repo, and a board ask takes
// no files yet, so both chips step aside for it.
export function AskBox({ onAsked }: { onAsked: (id: string) => void }) {
  const [text, setText] = useDraft("ask/room")
  const [repo, setRepo] = useState("")
  const [repos, setRepos] = useState<AskRepo[]>([])
  const [board, setBoard] = useState("")
  const { boards } = useBoards()
  const shared = boards.filter((b) => b.ready && !b.leaving)
  // A board that went away (forgotten, left) falls back to the private room.
  const onBoard = shared.some((b) => b.boardId === board) ? board : ""
  const [busy, setBusy] = useState(false)
  const [files, setFiles] = useState<File[]>([])
  const picker = useRef<HTMLInputElement>(null)
  const addFiles = (list: FileList | File[] | null | undefined) => {
    const add = Array.from(list ?? [])
    if (!add.length) return
    setFiles((fs) => [...fs, ...add].slice(0, MaxUploadFiles))
    setLine(null)
  }
  const [line, setLine] = useState<{ text: string; error?: boolean } | null>(
    null
  )

  useEffect(() => {
    let live = true
    cards
      .askRepos()
      .then((rs) => live && setRepos(rs))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (busy || !text.trim()) return
    if (onBoard && files.length) {
      setLine({
        text: "Files don't go on a board ask yet: remove them, or ask on My board.",
        error: true,
      })
      return
    }
    setBusy(true)
    setLine(null)
    try {
      const res = onBoard
        ? await cards.ask(text, "", true, [], onBoard)
        : await cards.ask(text, repo, true, files)
      // On a board the other people's agents are told by their own
      // devices: count every agent the ask's round names.
      const n = onBoard
        ? (res.task.ask?.told ?? 0)
        : (res.told.to?.length ?? 0)
      setLine({
        text:
          n === 0
            ? "Nobody is free to hear it yet; it stays first on the board."
            : `Woke ${n} ${n === 1 ? "agent" : "agents"}; one of them takes it.`,
      })
      setText("")
      setFiles([])
      onAsked(res.task.id)
    } catch (err) {
      setLine({ text: errorText(err), error: true })
    } finally {
      setBusy(false)
    }
  }

  return (
    <form
      className="grid gap-1"
      onSubmit={(e) => void submit(e)}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) e.preventDefault()
      }}
      onDrop={(e) => {
        if (!e.dataTransfer.files.length) return
        e.preventDefault()
        if (!busy) addFiles(e.dataTransfer.files)
      }}
    >
      <div className="glass-panel flex items-start gap-2 p-1.5">
        <Textarea
          autoSize
          rows={1}
          value={text}
          maxLength={MaxComment}
          onChange={(e) => {
            setText(e.target.value)
            setLine(null)
          }}
          onPaste={(e) => {
            // A pasted screenshot is a file; pasted text stays text.
            if (busy || !e.clipboardData.files.length) return
            e.preventDefault()
            addFiles(e.clipboardData.files)
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault()
              e.currentTarget.form?.requestSubmit()
            }
          }}
          placeholder="Ask the room…"
          aria-label="Ask every free agent"
          className="max-h-40 min-h-9 flex-1 border-transparent bg-transparent shadow-none focus-visible:border-transparent"
          // readOnly, not disabled: disabling the focused box would blur it,
          // and the next ask would need a click back in.
          readOnly={busy}
          aria-busy={busy}
        />
        {shared.length > 0 && (
          <ChoiceSelect
            label="Which board it goes on"
            size="sm"
            value={onBoard}
            onValueChange={(v) => {
              setBoard(v)
              setLine(null)
            }}
            disabled={busy}
            className="w-auto shrink-0"
            choices={[
              { value: "", label: "My board" },
              ...shared.map((b) => ({ value: b.boardId, label: b.name || b.tag })),
            ]}
          />
        )}
        {!onBoard && repos.length > 0 && (
          <ChoiceSelect
            label="Which agents hear it"
            size="sm"
            value={repo}
            onValueChange={setRepo}
            disabled={busy}
            className="w-auto shrink-0"
            choices={[
              { value: "", label: "Any repo" },
              ...repos.map((r) => ({ value: r.repoKey, label: r.label })),
            ]}
          />
        )}
        <input
          ref={picker}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            addFiles(e.target.files)
            e.target.value = ""
          }}
        />
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          className="shrink-0"
          title="Add files (or drop them here, or paste a screenshot)"
          aria-label="Add files"
          disabled={busy || !!onBoard || files.length >= MaxUploadFiles}
          onClick={() => picker.current?.click()}
        >
          <PaperclipIcon />
        </Button>
        <Button
          type="submit"
          size="sm"
          className="shrink-0"
          disabled={busy || !text.trim()}
        >
          {busy ? "Asking…" : "Ask"}
        </Button>
      </div>
      {files.length > 0 && (
        <ul className="flex flex-wrap gap-1.5 px-1" aria-label="Files with the ask">
          {files.map((f, i) => (
            <li
              key={`${i}-${f.name}`}
              className="flex items-center gap-1 bg-secondary px-1.5 py-0.5 text-xs ring-1 ring-border"
            >
              <PaperclipIcon className="size-3 text-muted-foreground" aria-hidden />
              <span className="max-w-48 truncate">{f.name || "pasted image"}</span>
              <button
                type="button"
                className="text-muted-foreground hover:text-foreground"
                aria-label={`Remove ${f.name || "pasted image"}`}
                disabled={busy}
                onClick={() => setFiles((fs) => fs.filter((_, j) => j !== i))}
              >
                <XIcon className="size-3" />
              </button>
            </li>
          ))}
        </ul>
      )}
      {line && (
        <p
          className={
            line.error
              ? "px-1 text-xs text-destructive"
              : "px-1 text-xs text-muted-foreground"
          }
          role={line.error ? "alert" : undefined}
        >
          {line.text}
        </p>
      )}
    </form>
  )
}
