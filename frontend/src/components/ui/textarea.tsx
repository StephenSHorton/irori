import * as React from "react"
import { cn } from "cn"

function Textarea({
  className,
  autoSize = false,
  ...props
}: React.ComponentProps<"textarea"> & {
  /** Grow with the content (CSS field-sizing). Cap it with max-h-*. */
  autoSize?: boolean
}) {
  return (
    <textarea
      data-slot="textarea"
      data-auto-size={autoSize || undefined}
      className={cn(
        "flex min-h-16 w-full min-w-0 resize-y rounded-lg border border-border bg-background px-2.5 py-2 text-sm transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:cursor-not-allowed disabled:bg-input/50 disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40",
        autoSize && "field-sizing-content supports-[field-sizing:content]:resize-none",
        className
      )}
      {...props}
    />
  )
}

export { Textarea }
