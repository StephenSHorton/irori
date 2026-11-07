// Voice calls, the parts still a mock-up (card #571). Only the browser
// preview shows them, behind ?call= (lib/preview.ts): nothing is wired. The
// huddle itself is real now (#654, huddle.tsx; ?call=live and ?call=start
// show it on lib/call.ts's stand-in), and so is a screen share (#658,
// share.tsx; ?call=share). What's left here is what a huddle could grow
// into:
//
//   ?call=transcript  the huddle ended: its post in the channel, the
//                     transcript beside it, an agent acting on it

import {
  BotIcon,
  FileTextIcon,
  HeadphonesIcon,
  SearchIcon,
  SparklesIcon,
  SquarePlusIcon,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { previewFlag } from "@/lib/preview"
import { cn } from "@/lib/utils"

export type CallMock = "transcript"

export function callMock(): CallMock | null {
  // Server-rendered tests have no window.
  if (typeof window === "undefined") return null
  const v = previewFlag("call")
  return v === "transcript" ? v : null
}

// The channel the mock-up's huddle is in.
export const callChannel = "sample-ch-suzuri"

type Who = {
  name: string
  initials: string
  hue: string
  agent?: boolean
}

const people: Who[] = [
  {
    name: "You",
    initials: "S",
    hue: "var(--device-teal)",
  },
  {
    name: "Spencer",
    initials: "Sp",
    hue: "var(--device-orange)",
  },
  {
    name: "Aiko",
    initials: "A",
    hue: "var(--device-pink)",
  },
  {
    name: "sample-heron",
    initials: "",
    hue: "var(--device-indigo)",
    agent: true,
  },
]

// Lines as each speaker's own device writes them down.
const lines: { at: string; who: string; text: string; tag?: string }[] = [
  {
    at: "10:02",
    who: "You",
    text: "Okay, quick one: are we tagging 0.10.0 today or waiting on #8?",
  },
  {
    at: "10:03",
    who: "Spencer",
    text: "#8 merged an hour ago. The only thing left is the PC transfer, it still needs the Firewall click.",
  },
  {
    at: "10:04",
    who: "Aiko",
    text: "I can do the click when I'm back at the PC, around two.",
  },
  {
    at: "10:05",
    who: "You",
    text: "Then let's tag at two, right after Aiko's click.",
    tag: "Decision",
  },
  {
    at: "10:07",
    who: "Spencer",
    text: "Someone should rerun the latency test on the PC once it's idle, the spikes looked like load.",
    tag: "To-do",
  },
  {
    at: "10:09",
    who: "You",
    text: "heron, can you make a card for that rerun?",
    tag: "To-do",
  },
]

function Avatar({ w, size = "md" }: { w: Who; size?: "sm" | "md" | "lg" }) {
  const cls =
    size === "lg"
      ? "size-16 text-lg"
      : size === "sm"
        ? "size-5 text-[9px]"
        : "size-10 text-sm"
  return (
    <span
      className={cn(
        "relative flex shrink-0 items-center justify-center rounded-full font-semibold text-background",
        cls,
        w.agent && "border border-dashed border-current bg-transparent"
      )}
      style={w.agent ? { color: w.hue } : { background: w.hue }}
      aria-hidden
    >
      {w.agent ? <BotIcon className="size-1/2" /> : w.initials}
    </span>
  )
}

// MockHuddleEnded is an ended huddle's post with what an agent made of its
// transcript, and the transcript one click away.
export function MockHuddleEnded({ onOpen }: { onOpen?: () => void }) {
  return (
    <div className="mx-2 my-1.5 rounded-lg border border-border bg-card/60 p-3 text-[13px]">
      <div className="flex items-center gap-2">
        <HeadphonesIcon className="size-4 shrink-0 text-muted-foreground" />
        <span className="shrink-0 font-medium text-ink">Huddle ended</span>
        <span className="min-w-0 truncate text-[12px] text-muted-foreground">
          10:02 to 10:20 · 18 min
        </span>
      </div>
      <p className="pl-6 text-[12px] text-muted-foreground">
        You, Spencer and Aiko talked; sample-heron listened.
      </p>
      <div className="mt-2 grid gap-1 pl-6 text-ink-2">
        <p className="flex items-center gap-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          <SparklesIcon className="size-3" />
          Summary by sample-heron
        </p>
        <p>
          <Badge variant="info" className="mr-1.5 normal-case">
            Decision
          </Badge>
          Tag 0.10.0 at two, right after Aiko's Firewall click on the PC.
        </p>
        <p>
          <Badge variant="warning" className="mr-1.5 normal-case">
            To-do
          </Badge>
          Rerun the latency test on the PC once it's idle.
        </p>
      </div>
      <div className="mt-2.5 flex gap-1.5 pl-6">
        <Button size="xs" variant="secondary" onClick={onOpen}>
          <FileTextIcon />
          Transcript · 38 lines
        </Button>
        <Button size="xs" variant="ghost">
          <SquarePlusIcon />
          Make a card
        </Button>
      </div>
    </div>
  )
}

// TranscriptPane is the whole transcript beside the channel: each line by
// who said it, and what an agent picked out of it.
export function TranscriptPane({ wide }: { wide?: boolean }) {
  return (
    <aside
      aria-label="Huddle transcript"
      className={cn(
        "flex min-h-0 flex-col",
        wide ? "flex-1" : "w-[340px] shrink-0 border-l border-border"
      )}
    >
      <div className="grid gap-1 border-b border-border px-3 py-3">
        <span className="flex items-center gap-2 text-[13px] font-semibold text-ink">
          <FileTextIcon className="size-4" />
          Huddle transcript
          <span className="font-normal text-muted-foreground">
            Today 10:02 · 18 min
          </span>
        </span>
        <span className="text-[11.5px] text-muted-foreground">
          Who can read it: the people and agents of #suzuri. Agents read it like
          any post in the channel.
        </span>
        <span className="mt-1 flex items-center gap-2 rounded-md border border-border px-2 py-1 text-[12px] text-muted-foreground">
          <SearchIcon className="size-3.5" />
          Search the transcript
        </span>
      </div>
      <div className="grid min-h-0 flex-1 content-start gap-2.5 overflow-y-auto px-3 py-3 text-[12.5px]">
        {lines.map((l) => {
          const w = people.find((p) => p.name === l.who)!
          return (
            <div key={l.at + l.who} className="flex gap-2">
              <Avatar w={w} size="sm" />
              <p className="min-w-0 flex-1 text-ink-2">
                <span className="font-medium text-ink">{l.who}</span>{" "}
                <span className="text-[11px] text-muted-foreground tabular-nums">
                  {l.at}
                </span>
                {l.tag && (
                  <Badge
                    variant={l.tag === "Decision" ? "info" : "warning"}
                    className="ml-1.5 normal-case"
                  >
                    {l.tag}
                  </Badge>
                )}
                <br />
                {l.text}
              </p>
            </div>
          )
        })}
        <p className="text-center text-[11px] text-muted-foreground">
          … 32 more lines
        </p>
      </div>
      <div className="flex items-center gap-1.5 border-t border-border px-3 py-2">
        <Button size="xs" variant="secondary">
          <SquarePlusIcon />
          Make a card from it
        </Button>
        <Button size="xs" variant="ghost">
          <BotIcon />
          Ask an agent
        </Button>
        <Button size="xs" variant="ghost" className="ml-auto text-destructive">
          Delete
        </Button>
      </div>
    </aside>
  )
}
