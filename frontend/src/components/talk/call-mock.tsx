// Voice calls in the browser preview (card #571). The mock-up's parts are
// all real now: the huddle (#654, huddle.tsx; ?call=live and ?call=start
// show it on lib/call.ts's stand-in), a screen share (#658, share.tsx;
// ?call=share) and its transcripts (#656, transcript-pane.tsx). What's left
// here is the flag that opens one on the preview's channel:
//
//   ?call=transcript  #suzuri's earlier, ended huddle with its real
//                     transcript open beside it (conversation.tsx)

import { previewFlag } from "@/lib/preview"

export type CallMock = "transcript"

export function callMock(): CallMock | null {
  // Server-rendered tests have no window.
  if (typeof window === "undefined") return null
  const v = previewFlag("call")
  return v === "transcript" ? v : null
}

// The channel the preview's huddles are in.
export const callChannel = "sample-ch-suzuri"
