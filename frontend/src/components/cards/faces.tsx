import { Avatar, AvatarBadge, AvatarFallback } from "@/components/ui/avatar"
import { displayName, initials, type AgentRef } from "@/lib/cards"
import { cn } from "@/lib/utils"

// Face is one agent (or the person) on a card: two letters in a rounded
// square, blue for working, grey for watching, violet for a person. A live
// working agent gets the green dot; a gone one is faded.
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
  return (
    <Avatar
      size="sm"
      className={cn(
        "size-[22px] rounded-[7px] after:rounded-[7px] after:border-0",
        gone && "opacity-55",
        className
      )}
      title={`${name} · ${who.where}${gone ? " · gone" : ""}`}
    >
      <AvatarFallback
        className={cn(
          "rounded-[7px] font-mono text-[10px] font-semibold",
          person
            ? "bg-[#ede4fe] font-sans text-[#6d28d9]"
            : watching
              ? "bg-muted text-muted-foreground ring-1 ring-border ring-inset"
              : "bg-accent text-primary"
        )}
      >
        {person ? "you" : initials(name)}
      </AvatarFallback>
      {!watching && !noDot && who.live && !person && (
        <AvatarBadge className="-right-0.5 -bottom-0.5 bg-success" />
      )}
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

// LiveDot is the green "live" dot in Who's on it.
export function LiveDot() {
  return (
    <span
      className="inline-block size-[7px] rounded-full bg-success"
      aria-hidden
    />
  )
}
