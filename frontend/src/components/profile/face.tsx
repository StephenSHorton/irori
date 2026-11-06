// Face is a person on the window (card #393): their avatar, else their
// initial on their colour (card #385). Only an avatar coord::person_avatar would
// count is drawn; anything else falls back to the initial.

import { initials, isAvatar } from "@/lib/avatar"
import { cn } from "@/lib/utils"

const sizes = {
  sm: "size-5 text-[10px]",
  md: "size-7 text-xs",
  lg: "size-12 text-lg",
  xl: "size-16 text-2xl",
} as const

export function Face({
  name,
  color,
  avatar,
  size = "md",
  className,
  title,
}: {
  name: string
  color?: string
  avatar?: string
  size?: keyof typeof sizes
  className?: string
  title?: string
}) {
  const pic = isAvatar(avatar) ? avatar : ""
  return (
    <span
      title={title ?? name}
      className={cn(
        "inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full font-semibold text-background select-none",
        !color && !pic && "bg-muted text-muted-foreground",
        sizes[size],
        className
      )}
      style={
        !pic && color ? { background: `var(--device-${color})` } : undefined
      }
    >
      {pic ? (
        <img
          src={pic}
          alt=""
          className="size-full object-cover"
          draggable={false}
        />
      ) : (
        <span aria-hidden>{initials(name)}</span>
      )}
      <span className="sr-only">{name}</span>
    </span>
  )
}
