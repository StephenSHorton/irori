// What the chat says about the assistant's computer (one-assistant.md §6,
// card 5): the header's "on Mac", the line above the composer for each state
// of assistant_home_status, and whether the box takes a question.

import type { AssistantHomeStatus } from "@/gen/bindings"
import { relativeTime } from "@/lib/irori"

export type HomeView = {
  // "on Mac", "" while nothing is recorded or read.
  where: string
  // The dot beside it: connected, recent or offline ("" for none).
  dot: string
  // The line above the composer, "" when there is none.
  line: string
  // Use <name> instead: a computer that says assistantChat and is connected
  // now, other than the assistant's. Null when none can take over.
  usePc: { device: string; name: string } | null
  // The box takes a question (it waits when the computer is away).
  composer: boolean
  // Why the box is off.
  placeholder: string
}

const quiet: HomeView = {
  where: "",
  dot: "",
  line: "",
  usePc: null,
  composer: true,
  placeholder: "",
}

// homeView reads one status. now is passed in so a test can fix it.
export function homeView(
  home: AssistantHomeStatus | null,
  now: Date = new Date()
): HomeView {
  if (!home) return quiet
  if (!home.device) {
    switch (home.none) {
      case "setting_up": {
        const c = home.candidates[0]
        return {
          ...quiet,
          line: `Setting up${c ? ` on ${c.name}` : ""}…`,
        }
      }
      case "needs_update": {
        const line =
          "None of your computers can answer from your other devices yet. Update Irori on the one that has Claude Code."
        return { ...quiet, line, composer: false, placeholder: line }
      }
      case "no_claude": {
        const line =
          "Your assistant needs a computer with Claude Code. None of your devices has it yet."
        return { ...quiet, line, composer: false, placeholder: line }
      }
    }
    return quiet
  }
  const name = home.name || "a computer"
  const take = home.candidates.find(
    (c) => c.device !== home.device && c.state === "connected"
  )
  const usePc = take ? { device: take.device, name: take.name } : null
  const base = {
    ...quiet,
    where: `on ${name}`,
    dot: home.here ? "connected" : home.state,
    usePc,
  }
  if (home.here) return { ...base, usePc: null }
  if (home.state !== "connected") {
    const seen = home.lastSeenMs
      ? `seen ${relativeTime(new Date(home.lastSeenMs), now)}`
      : "never seen"
    return {
      ...base,
      line: `Your assistant runs on ${name}, which isn't connected (${seen}). Questions wait for it.`,
    }
  }
  if (!home.claude)
    return {
      ...base,
      line: `Your assistant runs on ${name}, which can't find Claude Code. Install it there.`,
    }
  if (!home.chat)
    return {
      ...base,
      line: `${name} needs the latest Irori to answer from your other devices.`,
    }
  return base
}
