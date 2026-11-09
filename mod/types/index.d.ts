/** One NDJSON line from `irori attach` (internal/attach/protocol.go Event). */
export type AttachEvent = {
  type: 'hello' | 'deliver' | 'append' | 'held' | 'moved' | 'inbox' | 'superseded' | 'error' | string
  // hello
  v?: number
  session?: string
  agentKey?: string
  name?: string
  /** hello and name events: where name came from ("user", "irori" or ""), card #21. */
  nameSource?: string
  /** hello and name events: the transcript's newest ai-title. */
  aiTitle?: string
  api?: { url: string; token: string }
  attachSecret?: string
  // deliver
  wakeId?: string
  token?: string
  reminder?: boolean
  deliveryIds?: string[]
  messageIds?: string[]
  prompt?: string
  from?: string
  /** "next_turn" on an append (and a Grok next_turn-only push). */
  mode?: string
  /** Delivery ids named in the prompt's last line instead of in full (part of deliveryIds). */
  overflow?: string[]
  /** The person's stop (card #207): cut the running turn, then submit. */
  stop?: boolean
  /** Cards the person paused (card #207): refuse every non-Irori tool until resumed. */
  pause?: string[]
  /** Cards the person resumed (card #207). */
  resume?: string[]
  // held / moved
  deliveryId?: string
  why?: string
  preview?: string
  to?: string
  // inbox
  unread?: number
  held?: number
  // error
  message?: string
}

/** GET /v1/tools?client=claude (internal/tools Spec). */
export type ToolSpec = {
  name: string
  description: string
  inputSchema?: Record<string, unknown>
  readOnly?: boolean
  clients?: string[]
}

/** internal/inbox Policy. */
export type Policy = {
  mode: 'everyone' | 'allowlist' | 'open'
  block: string[]
  allow: string[]
  paused: boolean
}

/** What the band and /irori show about this session's attach. */
export type AttachView = {
  /** off: no iroriPath (or the mod stopped); error: irori could not run. */
  status: 'off' | 'connecting' | 'attached' | 'superseded' | 'error'
  error?: string
  name?: string
  /** Where name came from: "irori" means Irori named this session (card #21). */
  nameSource?: string
  aiTitle?: string
  agentKey?: string
  unread: number
  held: number
}

/** A wake this session was handed: the token its prompt carries, and its turn once started. */
export type TrackedWake = {
  token: string
  deliveryIds: string[]
  reminder: boolean
  /** The main-loop turn the prompt started; cleared when it ends. */
  turnId?: string
  /** Where the mod's reports to the Core stand. */
  delivered: boolean
  engaged: boolean
  ended: boolean
  /** The host refused its submit, so it went in as an append (card #23): the next main turn engages it. */
  appended?: boolean
}

/** A next-turn delivery (append event) this session was handed. */
export type TrackedAppend = {
  token: string
  deliveryIds: string[]
  /** `$.session.append` resolved: the row is stored. */
  appended: boolean
  delivered: boolean
  /** The main-loop turn start that engages it, once one began to. */
  turnId?: string
  engaged: boolean
}

/** A card the person paused (card #207), and who paused it. */
export type PausedCard = { ref: string; by: string }

/**
 * One `POST /v1/runs` body (card #1249, docs/design/live-tasks.md §2.1): a
 * background run started or ended, or a foreground shell call started or
 * finished. The Core keys it by the caller's attach, never by the body.
 */
export type RunReport = {
  event: 'start' | 'end' | 'fg-start' | 'fg'
  /** Absent on an end whose task id names no kind (not b/a/w-prefixed). */
  kind?: 'bash' | 'agent' | 'workflow' | 'fg'
  /** Claude Code's task id; absent on fg-start and fg. */
  taskId?: string
  /** The call that started it, when known. */
  toolUseId?: string
  /** The shell command, or the agent's description, or the workflow's name. */
  command?: string
  description?: string
  /** Where Claude Code writes the run's output, verbatim from the result text. */
  outputFile?: string
  cwd?: string
  startedAt?: number
  endedAt?: number
  /** end: completed, failed or killed (or the notification's own word). */
  status?: string
  exitCode?: number
  /** fg: the foreground call was interrupted / reported an error. */
  interrupted?: boolean
  isError?: boolean
}

declare module 'claude-code' {
  interface PluginState {
    irori: {
      attach: AttachView
      /** Newest last; kept across a hot reload so a turn that starts after it still engages. */
      wakes: TrackedWake[]
      /** Newest last; engaged at the next main-loop turn start after delivery. */
      appends: TrackedAppend[]
      /** Titles this mod set on the session (card #21), newest last. */
      titles: string[]
      /** Cards the person paused that this session holds (card #207). */
      paused: PausedCard[]
      /** Run reports not sent yet (no attach), oldest first; flushed at hello (card #1249). */
      runs: RunReport[]
    }
  }
}
