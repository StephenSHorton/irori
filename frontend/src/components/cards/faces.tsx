import { Avatar, AvatarBadge, AvatarFallback } from "@/components/ui/avatar"
import { displayName, initials, type AgentRef } from "@/lib/cards"
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
      className="absolute -top-1.5 -right-1.5 z-10 bg-background px-[2px] font-mono text-[8px] leading-[11px] font-semibold text-muted-foreground ring-1 ring-border"
    >
      {initials(device)}
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
