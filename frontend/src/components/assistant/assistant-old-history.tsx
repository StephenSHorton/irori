import type { ChatEntry } from "@/lib/assistant"

// OldHistory is this device's chat from before the chat was shared (card
// #1105, one-assistant §10), read-only above a divider: plain text, no
// buttons that act, never sent to the assistant, gone with New chat.
export function OldHistory({
  entries,
  narrow,
}: {
  entries: readonly ChatEntry[]
  narrow: boolean
}) {
  if (!entries.length) return null
  return (
    <section
      aria-label="Earlier chat, read only"
      className="mb-4 grid gap-3 opacity-75"
      data-old-history=""
    >
      <div className="grid gap-3">
        {entries.map((e, i) => (
          <Line key={i} entry={e} />
        ))}
      </div>
      <div
        role="separator"
        className="flex items-center gap-2 text-[11.5px] text-muted-foreground"
      >
        <span className="h-px flex-1 bg-border" />
        <span>
          Earlier on this {narrow ? "phone" : "computer"}, before your chats
          were shared
        </span>
        <span className="h-px flex-1 bg-border" />
      </div>
    </section>
  )
}

function Line({ entry }: { entry: ChatEntry }) {
  const text = entry.text.replace(/<<go[^>]*>>/g, "").trim()
  if (!text) return null
  if (entry.role === "person")
    return (
      <p
        className="max-w-[85%] self-end justify-self-end rounded-2xl bg-muted px-3 py-1.5 text-[13px] [overflow-wrap:anywhere] whitespace-pre-wrap"
        data-old-person=""
      >
        {text}
      </p>
    )
  return (
    <p
      className={
        entry.note
          ? "text-[12.5px] [overflow-wrap:anywhere] text-muted-foreground"
          : "text-[13px] leading-relaxed [overflow-wrap:anywhere] whitespace-pre-wrap"
      }
      data-old-assistant=""
    >
      {text}
    </p>
  )
}
