import { useCallback, useEffect, useState } from "react"
import { UpdateService } from "@/lib/host"
import { inWails } from "@/lib/irori"
import { errorText } from "@/lib/talk"

export interface KeepUpToDateState {
  /**
   * Keep this device up to date (card #578): null where the device doesn't
   * update itself (a phone, an unsupported platform) or until it is read.
   */
  on: boolean | null
  error: string | null
  set: (on: boolean) => Promise<void>
}

// useKeepUpToDate reads and sets the per-device setting while `active`
// (the settings dialog is open). With no choice saved it follows the home
// server's switch, so the value read can change after the home server does.
// Outside the app (sample data) it works on local state only.
export function useKeepUpToDate(active: boolean): KeepUpToDateState {
  const [on, setOn] = useState<boolean | null>(() => (inWails() ? null : true))
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!active || !inWails()) return
    let live = true
    UpdateService.GetKeepUpToDate()
      .then((v) => live && setOn(v))
      .catch(() => live && setOn(null))
    return () => {
      live = false
    }
  }, [active])

  const set = useCallback(async (next: boolean) => {
    setOn(next) // the switch moves at once; the reply settles it
    if (!inWails()) return
    try {
      setOn(await UpdateService.SetKeepUpToDate(next))
      setError(null)
    } catch (err) {
      setError(errorText(err))
      setOn(await UpdateService.GetKeepUpToDate().catch(() => !next))
    }
  }, [])

  return { on, error, set }
}
