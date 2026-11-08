import { useEffect, useSyncExternalStore } from "react"

import {
  loadVoiceStatus,
  megabytes,
  sharedVoice,
  subscribeVoice,
} from "@/lib/voice"

// VoiceNote is Settings' word on voice input (card #240): where the audio
// goes (nowhere: Whisper runs on this device) and whether the speech model
// is downloaded. Hidden where there's no engine.
export function VoiceNote({ open }: { open: boolean }) {
  const v = useSyncExternalStore(subscribeVoice, sharedVoice)
  useEffect(() => {
    if (open) void loadVoiceStatus(true)
  }, [open])
  const st = v.status
  if (!st || st.state === "unsupported") return null
  const model =
    st.state === "needs-model"
      ? `The first time you press the mic, Irori asks to download the speech model (${megabytes(st.modelBytes)}). Huddle transcripts need it too.`
      : `The speech model (${megabytes(st.modelBytes)}) is downloaded.`
  return (
    <div className="grid gap-1">
      <span className="text-ink">Voice input</span>
      <p className="text-xs text-muted-foreground">
        The mic in the text boxes turns speech into text you can edit before
        sending. Speech is transcribed on this device with Whisper: no audio
        leaves it, and nothing is sent until you press Send. {model}
      </p>
      {st.state === "denied" && (
        <p className="text-xs text-destructive">{st.reason}</p>
      )}
    </div>
  )
}
