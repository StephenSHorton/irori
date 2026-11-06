// ReplyingRow is the line under a channel's or DM's posts while agents it
// woke are on their answer (card #645): "heron and otter are replying…".
// It looks like a thread's "oak is working…" line.

import { Spinner } from "@/components/cards/bits"

export function ReplyingRow({
  line,
}: {
  line: { who: string; verb: string; text: string }
}) {
  return (
    <p
      role="status"
      aria-label={line.text}
      className="flex items-center gap-2 px-1 py-1 text-[12.5px] text-ink-2"
    >
      <Spinner className="size-3.5" label={line.text} />
      <span className="min-w-0 truncate">
        <span className="font-medium">{line.who}</span> {line.verb} replying…
      </span>
    </p>
  )
}
