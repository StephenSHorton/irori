/** One NDJSON line from `irori attach` (internal/attach/protocol.go Event). */
export type AttachEvent = {
  type: 'hello' | 'deliver' | 'append' | 'held' | 'moved' | 'inbox' | 'superseded' | 'error' | string
  // hello
  v?: number
  session?: string
  agentKey?: string
  name?: string
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
  mode: 'everyone' | 'allowlist'
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

declare module 'claude-code' {
  interface PluginState {
    irori: {
      attach: AttachView
      /** Newest last; kept across a hot reload so a turn that starts after it still engages. */
      wakes: TrackedWake[]
      /** Newest last; engaged at the next main-loop turn start after delivery. */
      appends: TrackedAppend[]
    }
  }
}
