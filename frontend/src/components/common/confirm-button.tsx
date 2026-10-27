import { useRef, useState, type ReactNode } from "react"

import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogClose,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/dialog"
import { errorText } from "@/lib/talk"

// ConfirmButton asks before it acts: the click opens an alert dialog with
// the question and Confirm / Cancel. Confirm has focus, so Enter confirms
// and Escape cancels. A plain-text question is the dialog's title; a richer
// one sits under confirmLabel as the description. It runs onConfirm once
// and shows its error, if any, under the button until the next try.
export function ConfirmButton({
  label,
  question,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  variant = "outline",
  disabled,
  onConfirm,
}: {
  label: ReactNode
  question: ReactNode
  confirmLabel?: string
  cancelLabel?: string
  variant?: "outline" | "destructive" | "default"
  disabled?: boolean
  onConfirm: () => Promise<unknown>
}) {
  const [asking, setAsking] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const confirmRef = useRef<HTMLButtonElement>(null)

  const confirm = async () => {
    setBusy(true)
    try {
      await onConfirm()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
      setAsking(false)
    }
  }

  return (
    <span className="flex flex-col items-end gap-1">
      <AlertDialog
        open={asking}
        onOpenChange={(open) => {
          if (busy) return
          if (open) setError(null)
          setAsking(open)
        }}
      >
        <AlertDialogTrigger
          render={
            <Button
              size="xs"
              variant={variant === "destructive" ? "outline" : variant}
              disabled={disabled}
            />
          }
        >
          {label}
        </AlertDialogTrigger>
        <AlertDialogContent initialFocus={confirmRef}>
          <AlertDialogHeader>
            {typeof question === "string" ? (
              <AlertDialogTitle>{question}</AlertDialogTitle>
            ) : (
              <>
                <AlertDialogTitle>{confirmLabel}</AlertDialogTitle>
                <AlertDialogDescription render={<div />}>
                  {question}
                </AlertDialogDescription>
              </>
            )}
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogClose
              disabled={busy}
              render={<Button size="sm" variant="ghost" />}
            >
              {cancelLabel}
            </AlertDialogClose>
            <Button
              ref={confirmRef}
              size="sm"
              variant={variant === "outline" ? "default" : variant}
              disabled={busy}
              onClick={() => void confirm()}
            >
              {busy ? "Working…" : confirmLabel}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      {error && (
        <span className="max-w-xs text-right text-xs text-destructive">
          {error}
        </span>
      )}
    </span>
  )
}
