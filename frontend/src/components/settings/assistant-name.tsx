// The name of your Office Assistant, in Settings › Agents (card #1045,
// docs/design/call-agents.md §5). Blank keeps the default, "<your name>'s
// assistant". Only you set it, here; no agent's tool can.

import { useEffect, useId, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { devices } from "@/lib/devices"
import { MaxAssistantName, validAssistantName } from "@/lib/assistant-label"

export function AssistantNameSection({ open }: { open: boolean }) {
  const id = useId()
  const [saved, setSaved] = useState("")
  const [shown, setShown] = useState("")
  const [text, setText] = useState("")
  const [error, setError] = useState("")

  useEffect(() => {
    if (!open) return
    let live = true
    Promise.all([devices.assistantName(), devices.assistantLabel()])
      .then(([n, l]) => {
        if (!live) return
        setSaved(n)
        setText(n)
        setShown(l)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [open])

  const name = text.trim()
  const ok = validAssistantName(name)
  const changed = name !== saved
  const save = async (next: string) => {
    setError("")
    try {
      await devices.setAssistantName(next)
      setSaved(next)
      setText(next)
      setShown(await devices.assistantLabel())
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium text-ink">
        <label htmlFor={id}>Your assistant's name</label>
      </h3>
      <p id={`${id}-hint`} className="text-[12.5px] text-muted-foreground">
        What people call your Office Assistant. Leave it blank to keep{" "}
        {shown && !saved ? `“${shown}”` : "“your name's assistant”"}. Others see
        a chosen name beside yours, like “Juno · Addam's assistant”.
      </p>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          value={text}
          maxLength={MaxAssistantName}
          placeholder={shown && !saved ? shown : "Default"}
          aria-describedby={`${id}-hint`}
          aria-invalid={!ok}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && ok && changed) void save(name)
          }}
          className="h-8 max-w-[16rem] text-[13px]"
        />
        <Button
          size="sm"
          disabled={!ok || !changed}
          onClick={() => void save(name)}
        >
          Save
        </Button>
        {saved && (
          <Button size="sm" variant="ghost" onClick={() => void save("")}>
            Use the default
          </Button>
        )}
      </div>
      {!ok && (
        <p role="alert" className="text-xs text-destructive">
          Use up to {MaxAssistantName} letters and spaces.
        </p>
      )}
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </section>
  )
}
