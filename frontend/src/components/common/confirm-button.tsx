import { useState, type ReactNode } from "react"

import { Button } from "@/components/ui/button"
import { errorText } from "@/lib/talk"

// ConfirmButton asks before it acts: the first click swaps the button for a
// question with Confirm / Cancel inline. It runs onConfirm once and shows
// its error, if any, until the next try.
export function ConfirmButton({
  label,
  question,
  confirmLabel = "Confirm",
  variant = "outline",
  disabled,
  onConfirm,
}: {
  label: ReactNode
  question: ReactNode
  confirmLabel?: string
  variant?: "outline" | "destructive" | "default"
  disabled?: boolean
  onConfirm: () => Promise<unknown>
}) {
  const [asking, setAsking] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  if (!asking) {
    return (
      <span className="flex flex-col items-end gap-1">
        <Button
          size="xs"
          variant={variant === "destructive" ? "outline" : variant}
          disabled={disabled}
          onClick={() => {
            setError(null)
            setAsking(true)
          }}
        >
          {label}
        </Button>
        {error && (
          <span className="max-w-xs text-right text-xs text-destructive">
            {error}
          </span>
        )}
      </span>
    )
  }

  return (
    <span
      role="group"
      aria-label="Confirm"
      className="flex flex-wrap items-center justify-end gap-1.5 text-xs text-ink-2"
    >
      <span className="max-w-md">{question}</span>
      <Button
        size="xs"
        variant={variant === "outline" ? "default" : variant}
        disabled={busy}
        autoFocus
        onClick={async () => {
          setBusy(true)
          try {
            await onConfirm()
            setAsking(false)
          } catch (err) {
            setError(errorText(err))
            setAsking(false)
          } finally {
            setBusy(false)
          }
        }}
      >
        {busy ? "Working…" : confirmLabel}
      </Button>
      <Button
        size="xs"
        variant="ghost"
        disabled={busy}
        onClick={() => setAsking(false)}
      >
        Cancel
      </Button>
    </span>
  )
}
