// Pure helpers for the Irori mod: no `$`, no engine imports, so `bun test`
// can cover them as well as `claude plugin test`.

import type { AttachEvent, AttachView, Policy, ToolSpec, TrackedAppend } from '../types'

/** Matches .claude-plugin/plugin.json; sent to `irori attach` and shown by /irori. */
export const MOD_VERSION = '0.4.2'

export const PLUGIN = 'irori'
export const TOOL_PREFIX = 'mcp__irori__'

/** Restart backoff for `irori attach`: 1 s doubling to 16 s (design §7). */
export const BACKOFF_MIN_MS = 1_000
export const BACKOFF_MAX_MS = 16_000

/** Wakes the mod remembers (newest kept), for token matching and dedupe. */
export const MAX_WAKES = 50
/**
 * How long `$.prompt.submit` may stay pending before the mod reports the wake
 * delivered anyway. A submit made during a running turn is queued and its
 * promise resolves only when its own turn starts; still pending after this
 * long, it was accepted into the queue (a refusal comes back at once).
 */
export const SUBMIT_ACCEPT_MS = 3_000

export const NO_PATH_ERROR = 'set iroriPath in /plugin settings (the absolute path to irori.exe)'

/**
 * Splits `irori attach`'s stdout into whole NDJSON events and the unfinished
 * tail. Blank keepalive lines, stray text and lines that fail to parse are skipped.
 */
export function takeEvents(buffer: string): { events: AttachEvent[]; rest: string } {
  const lines = buffer.split('\n')
  const rest = lines.pop() ?? ''
  const events: AttachEvent[] = []
  for (const raw of lines) {
    const line = raw.trim()
    if (!line.startsWith('{')) continue
    try {
      const parsed = JSON.parse(line) as unknown
      if (parsed && typeof parsed === 'object' && typeof (parsed as { type?: unknown }).type === 'string') {
        events.push(parsed as AttachEvent)
      }
    } catch {
      // Not an event; skip it.
    }
  }
  return { events, rest }
}

/**
 * The `$.session.append` args for a next-turn delivery (an append event): a
 * user-role row the person does not see as typed, which the model reads on
 * its next request. It starts no turn.
 */
export function appendArgs(prompt: string) {
  return { message: { type: 'user' as const, content: [{ type: 'text' as const, text: prompt }] } }
}

/**
 * The appends a main-loop turn start engages: every one reported delivered
 * and not yet engaged (or being engaged by an earlier turn start). The
 * append's token is never in the turn's text; whatever started the turn
 * reads it.
 */
export function appendsToEngage(list: readonly TrackedAppend[]): TrackedAppend[] {
  return list.filter(a => a.delivered && !a.engaged && !a.turnId)
}

export function nextBackoff(ms: number): number {
  return Math.min(BACKOFF_MAX_MS, Math.max(BACKOFF_MIN_MS, ms * 2))
}

/** `/irori <verb> <rest>`: the verb lowercased, the rest trimmed and kept whole (names may hold spaces). */
export function parseArgs(args: string): { verb: string; rest: string } {
  const trimmed = (args ?? '').trim()
  const space = trimmed.search(/\s/)
  if (space < 0) return { verb: trimmed.toLowerCase(), rest: '' }
  return { verb: trimmed.slice(0, space).toLowerCase(), rest: trimmed.slice(space).trim() }
}

/** The tool's arguments as the model gave them, without the envelope the engine adds. */
export function toolArgs(e: Record<string, unknown>): Record<string, unknown> {
  const { tool: _tool, tool_use_id: _id, agentId: _agent, ...args } = e
  return args
}

/** The bare Irori tool name from `mcp__irori__<name>`, or '' for another tool. */
export function bareToolName(tool: string): string {
  return tool.startsWith(TOOL_PREFIX) ? tool.slice(TOOL_PREFIX.length) : ''
}

/** A spec the engine will take: a short name of letters, digits, `_` or `-`. */
export function validSpec(spec: unknown): spec is ToolSpec {
  if (!spec || typeof spec !== 'object') return false
  const s = spec as Partial<ToolSpec>
  return typeof s.name === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(s.name) && typeof s.description === 'string'
}

/** The band's text: "irori · 2 unread · 1 held", or the state it is in. */
export function bandText(view: AttachView): string {
  if (view.status === 'off' || view.status === 'error') return `irori · ${view.error ?? 'off'}`
  if (view.status === 'connecting') return view.error ? `irori · reconnecting (${view.error})` : 'irori · connecting…'
  if (view.status === 'superseded') return 'irori · attached in another window'
  return `irori · ${view.unread} unread · ${view.held} held`
}

/** /irori's status line. */
export function describeAttach(view: AttachView): string {
  switch (view.status) {
    case 'attached':
      return `Attached as ${view.name || '(unnamed)'} (${view.agentKey ?? 'unknown key'}). ${view.unread} unread, ${view.held} held.`
    case 'connecting':
      return view.error ? `Reconnecting to the Irori Core: ${view.error}` : 'Connecting to the Irori Core…'
    case 'superseded':
      return 'A newer attach for this session took over (a reload or another window); this copy stopped.'
    case 'error':
      return `Not attached: ${view.error ?? 'unknown error'}`
    default:
      return `Not attached: ${view.error ?? NO_PATH_ERROR}`
  }
}

export const POLICY_VERBS = ['mode', 'block', 'unblock', 'allow', 'deny', 'pause', 'resume'] as const

export const USAGE = [
  '/irori                               version and attach status',
  '/irori mode everyone|allowlist       who may wake this device\'s agents (everyone: all of them)',
  '/irori block <key or name>           (everyone mode) that sender\'s messages are held',
  '/irori unblock <key or name>         remove a block',
  '/irori allow <key or name>           (allowlist mode) let that sender wake agents',
  '/irori deny <key or name>            remove an allowed sender',
  '/irori pause | resume                hold every new wake, or let them through again',
  'Policy changes only work when a person types them.',
].join('\n')

/**
 * Applies one policy verb to the current policy. Returns the new policy and
 * what to say, or an error text for bad arguments.
 */
export function editPolicy(
  current: Policy,
  verb: string,
  rest: string,
): { policy: Policy; text: string } | { error: string } {
  const policy: Policy = {
    // open (the old Full auto) is everyone now: nothing else holds.
    mode: current.mode === 'allowlist' ? 'allowlist' : 'everyone',
    block: [...(current.block ?? [])],
    allow: [...(current.allow ?? [])],
    paused: Boolean(current.paused),
  }
  const who = rest.trim()
  const without = (list: string[]) => list.filter(x => x.toLowerCase() !== who.toLowerCase())
  switch (verb) {
    case 'mode': {
      // open was Full auto; it is everyone now, so it is still taken.
      const mode = who.toLowerCase() === 'open' ? 'everyone' : who.toLowerCase()
      if (mode !== 'everyone' && mode !== 'allowlist') {
        return { error: 'Use /irori mode everyone or /irori mode allowlist.' }
      }
      policy.mode = mode
      return {
        policy,
        text:
          mode === 'everyone'
            ? 'Every agent may now wake this device\'s agents, except blocked senders (/irori block).'
            : 'Only allowed senders now wake this device\'s agents (/irori allow); other messages are held.',
      }
    }
    case 'block':
    case 'unblock':
    case 'allow':
    case 'deny': {
      if (!who) return { error: `Give an agent key or name: /irori ${verb} <key or name>.` }
      if (who.length > 200) return { error: 'That name is too long.' }
      if (verb === 'block') {
        policy.block = [...without(policy.block), who]
        return { policy, text: `Blocked ${who}. Their messages are held instead of waking anyone here.` }
      }
      if (verb === 'unblock') {
        policy.block = without(policy.block)
        return { policy, text: `Unblocked ${who}.` }
      }
      if (verb === 'allow') {
        policy.allow = [...without(policy.allow), who]
        return { policy, text: `Allowed ${who}. Their messages now wake agents here.` }
      }
      policy.allow = without(policy.allow)
      return { policy, text: `Removed ${who} from the allow list.` }
    }
    case 'pause':
      policy.paused = true
      return { policy, text: 'Paused: every new message is held and nothing wakes until /irori resume.' }
    case 'resume':
      policy.paused = false
      return { policy, text: 'Resumed: held-by-pause messages are queued again.' }
    default:
      return { error: USAGE }
  }
}

/** A one-line summary of the policy for /irori's answers. */
export function describePolicy(p: Policy): string {
  const list = p.mode === 'allowlist' ? p.allow : p.block
  const names = list.length ? list.join(', ') : 'nobody'
  const who =
    p.mode === 'allowlist' ? `allowlist mode, allowed: ${names}` : `everyone mode, blocked: ${names}`
  return `Policy: ${who}${p.paused ? ' · paused' : ''}.`
}

/** How many titles the mod remembers having set (titleToSet's setBefore). */
export const MAX_TITLES = 10

/**
 * The title to set on Claude's session (card #21), or undefined to leave it.
 * Only Irori's own name for a session nobody named, and only over a title
 * nobody chose: none, one this mod set before, or Claude's own ai-title. Any
 * other title is a rename the Core hasn't seen yet (presence lags a couple of
 * seconds), so it stays.
 */
export function titleToSet(
  view: Pick<AttachView, 'name' | 'nameSource' | 'aiTitle'>,
  current: string | undefined,
  setBefore: readonly string[],
): string | undefined {
  const name = (view.name ?? '').trim()
  if (view.nameSource !== 'irori' || !name) return undefined
  const now = (current ?? '').trim()
  if (now === name) return undefined
  if (now === '' || setBefore.includes(now) || (view.aiTitle && now === view.aiTitle.trim())) return name
  return undefined
}

/** A Board card as GET /v1/board lists it: only what heldReminder reads. */
export type BoardCard = { short?: string; title?: string; state?: string; holders?: string[] }
export type BoardLists = { needsYou?: BoardCard[]; blocked?: BoardCard[]; working?: BoardCard[]; open?: BoardCard[] }

/**
 * The reminder a person's prompt carries while this session holds cards
 * (GitHub #3): the person works from the Irori app, not this terminal, so
 * what the agent needs from them goes on the card as a blocker even when
 * they asked here. Empty when it holds none.
 */
export function heldReminder(board: BoardLists | undefined, agentKey: string | undefined): string {
  if (!board || !agentKey) return ''
  const seen = new Set<string>()
  const held: string[] = []
  for (const c of [...(board.needsYou ?? []), ...(board.blocked ?? []), ...(board.working ?? []), ...(board.open ?? [])]) {
    if (!c.holders?.includes(agentKey)) continue
    const ref = c.short ? `#${c.short}` : (c.title ?? '')
    if (!ref || seen.has(ref)) continue
    seen.add(ref)
    held.push(c.title ? `${ref} "${c.title}"` : ref)
  }
  if (!held.length) return ''
  return (
    `Irori: you hold ${held.join(', ')}. The person follows your work in the Irori app and may not be watching this terminal. ` +
    'If you need them to decide, approve or do something (sign in, give access, pick an option), raise it on the card with the irori board_block tool, one blocker per question, even when they asked you here; you can also say it here. ' +
    'Keep the card\'s steps current with board_step.'
  )
}
