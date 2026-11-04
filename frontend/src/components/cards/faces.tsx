import type { CSSProperties } from "react"
import { UserIcon } from "lucide-react"
import { minidenticon } from "minidenticons"
import { AgentHover } from "@/components/cards/agent-hover"
import { agentStatus } from "@/lib/agent-status"
import { Avatar, AvatarBadge } from "@/components/ui/avatar"
import { deviceVar, displayName, type AgentRef } from "@/lib/cards"
import { cn } from "@/lib/utils"

// Face is one agent (or the person) on a card (card #79): a 5x5 pixel
// identicon drawn from the agent's key, so an agent keeps its face for
// as long as it lives (0.11.1 puts these back in place of #407's animated
// faces, which kept WindowServer busy). A working face wears its device's colour once there
// are two devices (card #22), so the colour says which device it is on;
// with one device it is blue. A watcher is grey; the person is a user
// icon in their device's colour (card #108). A live working agent gets the green dot; a gone one, or one on
// an offline device, is faded; a contested lease rings it amber
// (docs/design/v0.4-devices.md §9). Hovering an agent's face shows its
// hover card (card #104): name, device, and its cards; noHover leaves it
// out where the face already sits in a popup. inline is the small face
// beside a name in a thread row (card #121): about the name's height, in
// the text's colour (grey for the person or a gone agent), and in its
// device colour only while the face or its row (group/row) is hovered.
export function Face({
  who,
  watching,
  noDot,
  noHover,
  inline,
  className,
}: {
  who: AgentRef
  watching?: boolean
  noDot?: boolean
  noHover?: boolean
  inline?: boolean
  className?: string
}) {
  const person = who.status === "person"
  const gone = who.status === "gone"
  const tint = !watching ? deviceVar(who.color) : ""
  // A face that isn't answering is amber, as #407's faces were.
  const quiet = !person && !watching && who.status === "unresponsive"
  const name = displayName(who)
  // How it stands, as everywhere else (card #489); idle goes unsaid.
  const said = agentStatus(who).text
  const title = [
    name,
    who.where,
    who.device && `on ${who.device}`,
    said !== "idle" && said,
    who.conflict && `lease conflict on ${who.conflict}`,
  ]
    .filter(Boolean)
    .join(" · ")
  if (inline) {
    const face = (
      <span
        className={cn(
          "inline-flex size-3.5 shrink-0 items-center justify-center",
          person || gone
            ? "text-muted-foreground"
            : quiet
              ? "text-warning"
              : "text-ink",
          "transition-colors group-hover/row:text-(--face-tint) hover:text-(--face-tint)",
          (gone || who.offline) && "opacity-55",
          className
        )}
        style={
          {
            "--face-tint": quiet
              ? "var(--color-warning)"
              : tint || "var(--color-primary)",
          } as CSSProperties
        }
        title={noHover || person ? title : undefined}
        aria-label={title}
        role="img"
      >
        {person ? (
          <UserIcon className="size-full" strokeWidth={2.5} aria-hidden />
        ) : (
          <Pixels cells={cells(who.key)} full />
        )}
      </span>
    )
    return noHover || person ? face : <AgentHover who={who} trigger={face} />
  }
  const face = (
    <Avatar
      size="sm"
      className={cn(
        // A dim square outline frames every face (card #104).
        "size-[26px] rounded-none after:rounded-none after:border after:border-muted-foreground/35 after:mix-blend-normal dark:after:mix-blend-normal",
        (gone || who.offline) && "opacity-55",
        who.conflict && "ring-2 ring-warning",
        className
      )}
      title={noHover || person ? title : undefined}
      aria-label={title}
    >
      {person ? (
        // The person is a plain user icon (card #179), not a pixel face, so
        // they never read as one more agent; in the colour of the device
        // they wrote from (card #108), blue with one device.
        <span
          className={cn(
            "flex size-full items-center justify-center",
            watching ? "text-muted-foreground/70" : "text-primary"
          )}
          style={tint ? { color: tint } : undefined}
        >
          <UserIcon className="size-5" strokeWidth={2.25} aria-hidden />
        </span>
      ) : (
        <span
          className={cn(
            "flex size-full items-center justify-center",
            watching ? "text-muted-foreground/70" : "text-primary"
          )}
          style={
            quiet
              ? { color: "var(--color-warning)" }
              : tint
                ? { color: tint }
                : undefined
          }
        >
          <Identicon seed={who.key} />
        </span>
      )}
      {!watching && !noDot && who.live && !who.offline && !person && (
        <AvatarBadge className="-right-0.5 -bottom-0.5 bg-success" />
      )}
    </Avatar>
  )
  return noHover ? face : <AgentHover who={who} trigger={face} />
}

// fnv1a is a 32-bit FNV-1a hash: minidenticons' own hash leaves near
// keys (sample-wren) with three or four lit squares.
function fnv1a(str: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return h >>> 0
}

// cells are the lit squares of a seed's identicon, read from
// minidenticons' SVG but hashed with fnv1a, so they don't match
// minidenticon(key) on its own hash. A pattern too sparse
// to read as a face (under 8 squares) is drawn again from a salted seed.
const cellCache = new Map<string, [number, number][]>()
function cells(seed: string): [number, number][] {
  let c = cellCache.get(seed)
  if (!c) {
    for (let salt = 0; salt < 8; salt++) {
      const svg = minidenticon(salt ? `${seed}#${salt}` : seed, 0, 0, fnv1a)
      c = [...svg.matchAll(/x="(\d+)" y="(\d+)"/g)].map(
        (m) => [Number(m[1]), Number(m[2])] as [number, number]
      )
      if (c.length >= 8) break
    }
    cellCache.set(seed, c!)
  }
  return c!
}

// faceColor is an agent face's colour outside a Face: amber while it
// isn't answering, else its tint, else blue.
export function faceColor(status: string, tint: string): string {
  return status === "unresponsive"
    ? "var(--color-warning)"
    : tint || "var(--color-primary)"
}

// Identicon draws the seed's 5x5 pattern; full fills its box (the strip,
// the hover card and the canvas size it), else 20px.
export function Identicon({ seed, full }: { seed: string; full?: boolean }) {
  return <Pixels cells={cells(seed)} full={full} />
}

// Pixels draws a 5x5 pattern in currentColor at 4px a square, so it
// stays crisp at 20px.
function Pixels({
  cells,
  full,
}: {
  cells: [number, number][]
  full?: boolean
}) {
  return (
    <svg
      viewBox="0 0 5 5"
      className={full ? "size-full" : "size-5"}
      fill="currentColor"
      shapeRendering="crispEdges"
      aria-hidden
    >
      {cells.map(([x, y]) => (
        <rect key={`${x}-${y}`} x={x} y={y} width={1} height={1} />
      ))}
    </svg>
  )
}

// FACES_MAX is how many faces a tile's working or watching row shows
// before "+N" (card #222).
export const FACES_MAX = 5

// Faces is a row of faces. With max, at most max of them show, the rest
// as "+N" whose tooltip names them (card #222: a card's twelve watchers
// ran past its edge).
export function Faces({
  who,
  watching,
  max,
}: {
  who: AgentRef[]
  watching?: boolean
  max?: number
}) {
  const over = max !== undefined && who.length > max
  const shown = over ? who.slice(0, Math.max(max - 1, 1)) : who
  const rest = who.slice(shown.length)
  return (
    <span className="flex min-w-0 items-center gap-1">
      {shown.map((w) => (
        <Face key={w.key} who={w} watching={watching} />
      ))}
      {rest.length > 0 && (
        <span
          className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
          title={rest.map(displayName).join(", ")}
          aria-label={`and ${rest.length} more: ${rest.map(displayName).join(", ")}`}
        >
          +{rest.length}
        </span>
      )}
    </span>
  )
}

// TalkBubble shows agents talking to each other on a card (card #22):
// their faces, in their device colours, with a speech bubble whose dots
// bounce in turn; with reduced motion the dots stand still.
export function TalkBubble({ who }: { who: AgentRef[] }) {
  if (who.length < 2) return null
  const names = who.map(displayName).join(", ")
  return (
    <span
      className="flex items-center gap-1"
      title={`Talking on this card: ${names}`}
      aria-label={`${names} are talking on this card`}
    >
      <span className="flex items-center gap-0.5">
        {who.map((w) => (
          <Face key={w.key} who={w} noDot />
        ))}
      </span>
      <span
        aria-hidden
        className="relative flex h-4 items-center gap-[3px] rounded-full bg-secondary px-1.5 ring-1 ring-border"
      >
        {[0, 1, 2].map((i) => (
          <span
            key={i}
            className="size-[3px] rounded-full bg-foreground motion-safe:animate-bounce"
            style={{
              animationDelay: `${i * 150}ms`,
              animationDuration: "900ms",
            }}
          />
        ))}
      </span>
    </span>
  )
}

// LiveDot is the green "live" dot in Who's on it.
export function LiveDot() {
  return (
    <span
      className="inline-block size-[7px] rounded-full bg-success"
      aria-hidden
    />
  )
}
