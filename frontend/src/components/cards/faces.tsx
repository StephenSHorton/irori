import { Avatar, AvatarBadge, AvatarFallback } from "@/components/ui/avatar"
import { deviceVar, displayName, initials, type AgentRef } from "@/lib/cards"
import { cn } from "@/lib/utils"

// Face is one agent (or the person) on a card: two letters in a rounded
// square, blue for working, grey for watching, violet for a person. A live
// working agent gets the green dot; a gone one, or one on an offline
// device, is faded. Once you have a second device, a face from another
// device carries a small device chip, and a contested lease rings it amber
// (docs/design/v0.4-devices.md §9).
export function Face({
  who,
  watching,
  noDot,
  className,
}: {
  who: AgentRef
  watching?: boolean
  noDot?: boolean
  className?: string
}) {
  const person = who.status === "person"
  const gone = who.status === "gone"
  // A working face wears its device's colour once there are two devices
  // (card #22).
  const tint = !person && !watching ? deviceVar(who.color) : ""
  const name = displayName(who)
  const title = [
    name,
    who.where,
    gone && "gone",
    who.offline && "offline",
    who.conflict && `lease conflict on ${who.conflict}`,
  ]
    .filter(Boolean)
    .join(" · ")
  return (
    <Avatar
      size="sm"
      className={cn(
        "size-[22px] rounded-none after:rounded-none after:border-0",
        (gone || who.offline) && "opacity-55",
        who.conflict && "ring-2 ring-warning",
        className
      )}
      title={title}
    >
      <AvatarFallback
        className={cn(
          "rounded-none font-mono text-[10px] font-semibold",
          person
            ? "bg-person-surface font-sans text-person"
            : watching
              ? "bg-muted text-muted-foreground ring-1 ring-border ring-inset"
              : "bg-primary/15 text-primary"
        )}
        style={
          tint
            ? {
                color: tint,
                background: `color-mix(in oklch, ${tint} 16%, transparent)`,
              }
            : undefined
        }
      >
        {person ? "you" : initials(name)}
      </AvatarFallback>
      {!watching && !noDot && who.live && !who.offline && !person && (
        <AvatarBadge className="-right-0.5 -bottom-0.5 bg-success" />
      )}
      {who.device && <DeviceChip device={who.device} />}
    </Avatar>
  )
}

// Faces is a row of faces.
export function Faces({
  who,
  watching,
}: {
  who: AgentRef[]
  watching?: boolean
}) {
  return (
    <span className="flex items-center gap-1">
      {who.map((w) => (
        <Face key={w.key} who={w} watching={watching} />
      ))}
    </span>
  )
}

// DeviceChip is the small device mark on a face from another device: the
// device name's first two letters.
function DeviceChip({ device }: { device: string }) {
  return (
    <span
      aria-hidden
      className="absolute -top-2 -right-2 z-10 bg-secondary px-[3px] font-mono text-[9px] leading-3 font-semibold text-foreground ring-1 ring-border"
    >
      {initials(device)}
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
      <span className="flex items-center -space-x-1">
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
            style={{ animationDelay: `${i * 150}ms`, animationDuration: "900ms" }}
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
