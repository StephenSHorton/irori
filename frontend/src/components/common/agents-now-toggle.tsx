import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

// AgentsNowToggle is "Agents answer now" (card #391) on a composer or a
// thread: on, the agents it names are woken and Irori gives it an agent of
// its own at once (card #512); off, nobody is woken or handed it, and the
// agents in it read it at their next turn.
export function AgentsNowToggle({
  on,
  onChange,
  disabled,
  className,
}: {
  on: boolean
  onChange: (on: boolean) => void
  disabled?: boolean
  className?: string
}) {
  return (
    <label
      className={cn(
        "flex shrink-0 cursor-pointer items-center gap-1.5 text-[11.5px] whitespace-nowrap select-none",
        on ? "text-info" : "text-muted-foreground",
        disabled && "cursor-not-allowed opacity-60",
        className
      )}
      title={
        on
          ? "Agents answer now: Irori starts an agent to answer it at once, and the agents it names are woken"
          : "Agents answer later: nobody is woken or handed it; agents in it read it at their next turn and may reply"
      }
    >
      <Switch
        size="sm"
        checked={on}
        onCheckedChange={(v) => onChange(v)}
        disabled={disabled}
        className="data-checked:bg-info"
      />
      Agents answer now
    </label>
  )
}
