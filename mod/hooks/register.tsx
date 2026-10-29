// The Irori mod: attaches this Claude Code session to the local Irori Core
// through `irori attach`, submits the wakes the Core sends, appends its
// next-turn deliveries without starting a turn, reports turn boundaries, and
// serves Irori's tools as mcp__irori__<name>.
// Design: docs/design/v0.2-talk.md §7-§8; routes: internal/api/routes_v02.go.
//
// Loader rules (docs/research/v02/mod.md §9): `$` is only passed to top-level
// function declarations; no Node globals; atoms use literal plugin/key.
import { atom, read, update } from 'claude-code'
import type { EngineInterface, PluginOptions, Register } from 'claude-code'

import type { AttachEvent, AttachView, Policy, ToolSpec, TrackedAppend, TrackedWake } from '../types'
import {
  appendArgs,
  appendsToEngage,
  BACKOFF_MIN_MS,
  MAX_WAKES,
  MOD_VERSION,
  NO_PATH_ERROR,
  POLICY_VERBS,
  SUBMIT_ACCEPT_MS,
  USAGE,
  bandText,
  bareToolName,
  describeAttach,
  describePolicy,
  editPolicy,
  nextBackoff,
  parseArgs,
  takeEvents,
  titleToSet,
  MAX_TITLES,
  toolArgs,
  validSpec,
} from './lib'

/** Only a person's Enter at the prompt may change the wake policy (design §5). */
const PERSON_ORIGINS = ['composer']

const INITIAL_VIEW: AttachView = { status: 'connecting', unread: 0, held: 0 }
const attachAtom = atom({ plugin: 'irori', key: 'attach' } as const, INITIAL_VIEW)
const wakesAtom = atom({ plugin: 'irori', key: 'wakes' } as const, [])
const appendsAtom = atom({ plugin: 'irori', key: 'appends' } as const, [])
/** Titles this mod set on the session (card #21), newest last. */
const titlesAtom = atom({ plugin: 'irori', key: 'titles' } as const, [] as string[])

// Reset on every (re)load; a reload also ends the running child.
let pluginOptions: PluginOptions = {}
let generation = 0
let stopChild: (() => void) | null = null
let backoffMs = BACKOFF_MIN_MS
let restartTimer: { cancel: () => void } | null = null
let ended = false
/** From the newest hello: where control and tool calls go, and who they act as. */
let api: { url: string; token: string } | null = null
let attachSecret = ''
/** Tool names registered from the Core's list, so a second hello doesn't redo them. */
const registered = new Set<string>()

type Answer = { ok: true; status: number; body: unknown } | { ok: false; status: number; error: string }

function iroriPath(): string {
  return String(pluginOptions.iroriPath ?? '').trim()
}

function headers(extra: Record<string, string> = {}): Record<string, string> {
  // The bearer goes in headers, never `auth`: CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC
  // refuses plugin requests that carry `auth` (design §7).
  return {
    'content-type': 'application/json',
    authorization: `Bearer ${api?.token ?? ''}`,
    'X-Irori-Attach': attachSecret,
    ...extra,
  }
}

/** One call to the Core: its parsed body, or the error text. */
async function callCore(
  $: EngineInterface,
  method: 'GET' | 'POST',
  path: string,
  body?: unknown,
  extra?: Record<string, string>,
): Promise<Answer> {
  if (!api || !attachSecret) return { ok: false, status: 0, error: 'not attached to Irori yet' }
  try {
    const response = await $.http.fetch(api.url + path, {
      method,
      headers: headers(extra),
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    let parsed: unknown = undefined
    try {
      parsed = response.text ? JSON.parse(response.text) : undefined
    } catch {
      parsed = undefined
    }
    if (response.ok) return { ok: true, status: response.status, body: parsed }
    const error = (parsed as { error?: unknown } | undefined)?.error
    return {
      ok: false,
      status: response.status,
      error: typeof error === 'string' && error ? error : `the Irori Core answered ${response.status}`,
    }
  } catch (err) {
    return { ok: false, status: 0, error: err instanceof Error ? err.message : String(err) }
  }
}

/** A control call retried a few times (the Core may be restarting); true when it landed. */
async function report($: EngineInterface, path: string, body: object): Promise<boolean> {
  for (let attempt = 0; attempt < 5; attempt++) {
    const answer = await callCore($, 'POST', path, body)
    if (answer.ok) return true
    // A 4xx other than 403 (unknown token, say) won't get better by retrying.
    if (answer.status >= 400 && answer.status < 500 && answer.status !== 403) {
      $.ui.log(`irori: ${path} refused: ${answer.error}`, { to: 'debug' })
      return false
    }
    await $.clock.sleep(3_000)
  }
  $.ui.log(`irori: ${path} failed after retries`, { to: 'debug' })
  return false
}

async function setView($: EngineInterface, patch: Partial<AttachView>) {
  await update($, attachAtom, v => ({ ...v, ...patch }))
}

async function patchWake($: EngineInterface, token: string, patch: Partial<TrackedWake>) {
  await update($, wakesAtom, list => list.map(w => (w.token === token ? { ...w, ...patch } : w)))
}

/** Registers the Core's tools for this session. Registering a name again replaces it. */
async function registerTools($: EngineInterface, specs: ToolSpec[]) {
  for (const spec of specs) {
    await $.tool.register({
      name: spec.name,
      description: spec.description,
      inputSchema: spec.inputSchema ?? { type: 'object', properties: {} },
    })
    registered.add(spec.name)
  }
}

async function loadTools($: EngineInterface) {
  const answer = await callCore($, 'GET', '/v1/tools?client=claude')
  if (!answer.ok) {
    $.ui.log(`irori: could not list tools: ${answer.error}`, { to: 'debug' })
    return
  }
  const specs = (Array.isArray(answer.body) ? answer.body : []).filter(validSpec)
  try {
    await registerTools($, specs)
  } catch (err) {
    $.ui.toast(`irori: could not register tools: ${err instanceof Error ? err.message : String(err)}`)
  }
}

/** Submits a wake's prompt, then tells the Core it was handed over (accepted into the queue). */
async function onDeliver($: EngineInterface, e: AttachEvent) {
  const token = String(e.token ?? '')
  const prompt = String(e.prompt ?? '')
  if (!token || !prompt) return
  const known = (await read($, wakesAtom)).find(w => w.token === token)
  if (known) {
    // The Core handed the same wake again (a reattach): it was submitted already.
    if (!known.delivered && (await report($, '/v1/deliveries/delivered', { token }))) {
      await patchWake($, token, { delivered: true })
    }
    return
  }
  // Record it before submitting, so a turn that starts at once still matches.
  const wake: TrackedWake = {
    token,
    deliveryIds: e.deliveryIds ?? [],
    reminder: Boolean(e.reminder),
    delivered: false,
    engaged: false,
    ended: false,
  }
  await update($, wakesAtom, list => [...list.filter(w => w.token !== token), wake].slice(-MAX_WAKES))
  if (!e.reminder) $.ui.toast(`irori: message from ${e.from || 'another agent'}`)
  // `delivered` means accepted into Claude Code's queue, not "its turn started".
  // A submit made during a running turn resolves only when its own turn starts,
  // which can be past the Core's ReattachGrace; waiting for it would let the
  // Core reclaim the wake and wake the same deliveries again under a new token.
  // So a submit still pending after SUBMIT_ACCEPT_MS counts as queued.
  // A submit can also resolve as { drop } (a hook or the engine refused it):
  // the prompt never entered, so it counts as a failure, not a delivery.
  const submitted = $.prompt.submit({ text: prompt }).then(
    r =>
      typeof r?.drop === 'string'
        ? { failed: true as const, err: new Error(`prompt dropped: ${r.drop}`) }
        : { failed: false as const },
    (err: unknown) => ({ failed: true as const, err }),
  )
  let markQueued: () => void = () => {}
  const queued = new Promise<null>(resolve => (markQueued = () => resolve(null)))
  const timer = $.clock.after(SUBMIT_ACCEPT_MS, () => markQueued())
  const first = await Promise.race([submitted, queued])
  timer.cancel()
  if (first?.failed) return forgetWake($, token, first.err)
  if (await report($, '/v1/deliveries/delivered', { token })) await patchWake($, token, { delivered: true })
  if (first === null) {
    // Reported while still queued; a later throw or drop still forgets the wake
    // (the Core's delivered-but-never-engaged reminder then brings it back).
    const outcome = await submitted
    if (outcome.failed) await forgetWake($, token, outcome.err)
  }
}

async function patchAppend($: EngineInterface, token: string, patch: Partial<TrackedAppend>) {
  await update($, appendsAtom, list => list.map(a => (a.token === token ? { ...a, ...patch } : a)))
}

/**
 * A next-turn delivery (protocol.go TypeAppend): appended to the conversation
 * as a meta user row, which starts no turn, then reported delivered. No
 * toast: it is meant to be quiet. Engaged at the next main-loop turn start.
 */
async function onAppend($: EngineInterface, e: AttachEvent) {
  const token = String(e.token ?? '')
  const prompt = String(e.prompt ?? '')
  if (!token || !prompt) return
  const known = (await read($, appendsAtom)).find(a => a.token === token)
  if (known) {
    // Sent again (a reattach): appended already, or still being appended.
    if (known.appended && !known.delivered && (await report($, '/v1/deliveries/delivered', { token }))) {
      await patchAppend($, token, { delivered: true })
    }
    return
  }
  const tracked: TrackedAppend = { token, deliveryIds: e.deliveryIds ?? [], appended: false, delivered: false, engaged: false }
  await update($, appendsAtom, list => [...list.filter(a => a.token !== token), tracked].slice(-MAX_WAKES))
  let failure: unknown = undefined
  let failed = false
  try {
    // `{ deny }`: a plugin above refused it and nothing was stored.
    const stored = await $.session.append(appendArgs(prompt))
    if (typeof stored?.deny === 'string') {
      failed = true
      failure = new Error(`append refused: ${stored.deny}`)
    }
  } catch (err) {
    failed = true
    failure = err
  }
  if (failed) {
    // Forget it, so the Core's re-send (reattach, or its reclaim) appends it again.
    $.ui.log(`irori: session.append failed: ${failure instanceof Error ? failure.message : String(failure)}`, { to: 'debug' })
    await update($, appendsAtom, list => list.filter(a => a.token !== token))
    return
  }
  await patchAppend($, token, { appended: true })
  if (await report($, '/v1/deliveries/delivered', { token })) await patchAppend($, token, { delivered: true })
}

/** A main-loop turn started: it reads every delivered append, so engage them all. */
async function markAppendsEngaged($: EngineInterface, turnId: string) {
  for (const a of appendsToEngage(await read($, appendsAtom))) {
    await patchAppend($, a.token, { turnId })
    if (await report($, '/v1/deliveries/engaged', { token: a.token, turnId })) {
      await patchAppend($, a.token, { engaged: true })
    }
  }
}

/** A submit that failed: forget the wake, so the Core's re-send (on reattach) submits it again. */
async function forgetWake($: EngineInterface, token: string, err: unknown) {
  $.ui.log(`irori: prompt.submit failed: ${err instanceof Error ? err.message : String(err)}`, { to: 'debug' })
  await update($, wakesAtom, list => list.filter(w => w.token !== token))
}

async function onHello($: EngineInterface, e: AttachEvent) {
  if (e.api?.url && e.api.token) api = { url: e.api.url.replace(/\/+$/, ''), token: e.api.token }
  attachSecret = String(e.attachSecret ?? '')
  backoffMs = BACKOFF_MIN_MS
  await setView($, {
    status: 'attached',
    error: undefined,
    name: e.name,
    nameSource: e.nameSource,
    aiTitle: e.aiTitle,
    agentKey: e.agentKey,
  })
  void loadTools($).catch(() => {})
}

async function handleEvent($: EngineInterface, e: AttachEvent): Promise<'superseded' | void> {
  switch (e.type) {
    case 'hello':
      await onHello($, e)
      return
    case 'deliver':
      // Detached: a submit can wait for the running turn, and other events keep flowing.
      void onDeliver($, e).catch(() => {})
      return
    case 'append':
      void onAppend($, e).catch(() => {})
      return
    case 'held':
      $.ui.toast(`irori: held a message from ${e.from || 'an agent'} (${e.why || 'policy'}). Review it in the Irori app.`)
      return
    case 'moved':
      $.ui.toast(`irori: a message went to ${e.to || 'another agent'}, since this chat hadn't started on it.`)
      return
    case 'inbox':
      await setView($, { unread: Number(e.unread ?? 0), held: Number(e.held ?? 0) })
      return
    case 'name':
      await setView($, { name: e.name, nameSource: e.nameSource, aiTitle: e.aiTitle })
      return
    case 'error':
      $.ui.log(`irori attach: ${e.message ?? 'error'}`, { to: 'debug' })
      await setView($, { error: e.message })
      return
    case 'superseded':
      return 'superseded'
  }
}

function scheduleRestart($: EngineInterface, cwd: string, run: number) {
  restartTimer?.cancel()
  const wait = backoffMs
  backoffMs = nextBackoff(backoffMs)
  restartTimer = $.clock.after(wait, () => {
    restartTimer = null
    if (run === generation && !ended) void startAttach($, cwd)
  })
}

/** Spawns `irori attach` and reads its events; a newer call wins (generation guard). */
async function startAttach($: EngineInterface, cwd: string) {
  stopChild?.()
  stopChild = null
  restartTimer?.cancel()
  restartTimer = null
  const run = ++generation
  const path = iroriPath()
  if (!path) {
    await setView($, { status: 'off', error: NO_PATH_ERROR })
    return
  }
  const sessionId = await $.session.id()
  await setView($, { status: 'connecting' })

  void (async () => {
    let superseded = false
    let exit = ''
    try {
      const child = $.process.spawn({
        argv: [path, 'attach', '--session', sessionId, '--cwd', cwd, '--mod-version', MOD_VERSION],
      })
      stopChild = () => void child.return(undefined as never)
      let buffer = ''
      for (;;) {
        const step = await child.next()
        if (step.done) {
          exit = `irori attach exited (code ${step.value?.code ?? '?'})`
          break
        }
        if (run !== generation) return
        const { stream, text } = step.value
        if (stream === 'stderr') {
          $.ui.log(text, { to: 'debug' })
          continue
        }
        const { events, rest } = takeEvents(buffer + text)
        buffer = rest
        for (const event of events) {
          if ((await handleEvent($, event)) === 'superseded') superseded = true
        }
        if (superseded) {
          void child.return(undefined as never)
          break
        }
      }
    } catch (err) {
      exit = `can't run ${path}: ${err instanceof Error ? err.message : String(err)}`
    }
    if (run !== generation || ended) return
    stopChild = null
    api = null
    attachSecret = ''
    // A newer attach for this session took over (a reload, another copy): stop
    // quietly. The view is left alone: after a reload it is the new copy's.
    if (superseded) return
    await setView($, { status: 'connecting', error: `${exit}; retrying` })
    scheduleRestart($, cwd, run)
  })().catch(() => {
    // The module unloaded while the loop ran; nothing left to update.
  })
}

/** A main-loop turn started: engage each wake whose token its text carries. */
async function markEngaged($: EngineInterface, text: string, turnId: string) {
  const named = (await read($, wakesAtom)).filter(w => !w.reminder && !w.turnId && !w.ended && text.includes(w.token))
  for (const w of named) {
    await patchWake($, w.token, { turnId })
    if (await report($, '/v1/deliveries/engaged', { token: w.token, turnId })) {
      await patchWake($, w.token, { engaged: true, delivered: true })
    }
  }
}

/** That turn ended: tell the Core, so the next wake can go. */
async function markEnded($: EngineInterface, turnId: string) {
  const done = (await read($, wakesAtom)).filter(w => w.turnId === turnId && !w.ended)
  for (const w of done) {
    await patchWake($, w.token, { ended: true })
    await report($, '/v1/deliveries/turn-ended', { token: w.token, turnId })
  }
}

async function callTool($: EngineInterface, e: Record<string, unknown>): Promise<string> {
  const name = bareToolName(String(e.tool ?? ''))
  if (!name) return 'Error: not an Irori tool.'
  const answer = await callCore($, 'POST', `/v1/tools/${encodeURIComponent(name)}`, { args: toolArgs(e) })
  if (!answer.ok) {
    if (!api) {
      const view = await read($, attachAtom)
      return `Error: this session isn't attached to Irori (${describeAttach(view)})`
    }
    return `Error: ${answer.error}`
  }
  const text = (answer.body as { text?: unknown } | undefined)?.text
  return typeof text === 'string' ? text : 'Error: the Irori Core gave no result.'
}

async function changePolicy($: EngineInterface, verb: string, rest: string): Promise<string> {
  const current = await callCore($, 'GET', '/v1/policy')
  if (!current.ok) return `Not changed: ${current.error}.`
  const edit = editPolicy(current.body as Policy, verb, rest)
  if ('error' in edit) return edit.error
  const saved = await callCore($, 'POST', '/v1/policy', edit.policy, { 'X-Irori-Origin': 'composer' })
  if (!saved.ok) return `Not changed: ${saved.error}.`
  return `${edit.text}\n${describePolicy((saved.body as Policy | undefined) ?? edit.policy)}`
}

/**
 * Adds Irori's name as the session title to a SessionStart or UserPromptSubmit
 * result when titleToSet says so (card #21). Another hook's title stands, and
 * any failure returns the result unchanged.
 */
async function withTitle<R extends { sessionTitle?: string }>($: EngineInterface, result: R, current: string | undefined): Promise<R> {
  try {
    if (result?.sessionTitle) return result
    const view = await read($, attachAtom)
    const title = titleToSet(view, current, await read($, titlesAtom))
    if (!title) return result
    await update($, titlesAtom, list => [...list.filter(t => t !== title), title].slice(-MAX_TITLES))
    return { ...result, sessionTitle: title }
  } catch {
    return result
  }
}

async function onSessionStart($: EngineInterface, cwd: string) {
  await $.command.register({
    name: 'irori',
    description: 'Irori: attach status, and who may wake agents here (mode, block, allow, pause)',
  })
  if (!iroriPath()) {
    await setView($, { status: 'off', error: NO_PATH_ERROR })
    $.ui.toast(`irori: ${NO_PATH_ERROR}`)
    return
  }
  await startAttach($, cwd)
}

export const register: Register = (on, options) => {
  pluginOptions = options

  on('session.start', async ($, e, next) => {
    const started = await next(e)
    ended = false
    await onSessionStart($, e.cwd)
    return started
  })

  // Irori's name for a session nobody named shows as its title (card #21).
  // SessionStart usually fires before the attach, so a new session takes
  // it at its first prompt; a resumed one already has its name.
  on('classic.SessionStart', async ($, e, next) => withTitle($, await next(e), e.session_title))
  on('classic.UserPromptSubmit', async ($, e, next) => withTitle($, await next(e), e.session_title))

  on('session.end', async ($, e, next) => {
    ended = true
    restartTimer?.cancel()
    stopChild?.()
    return next(e)
  })

  // Every Irori tool, whichever names the Core listed (registered late, after hello).
  on('tool.call', { tool: /^mcp__irori__/ }, async ($, e) => ({
    result: await callTool($, e as unknown as Record<string, unknown>),
  })).catch(() => ({ result: 'Error: the irori mod failed on this call (it timed out or threw); try again.' }))

  on('turn.start', async ($, e, next) => {
    const started = await next(e)
    // Subagent turns can quote a token; only the main loop engages a wake,
    // and only the main loop's turns read appends.
    if (e.agentId === undefined) {
      void markEngaged($, e.text, e.turnId).catch(() => {})
      void markAppendsEngaged($, e.turnId).catch(() => {})
    }
    return started
  })

  on('turn.complete', async ($, e, next) => {
    const done = await next(e)
    if (e.agentId === undefined) void markEnded($, e.turnId).catch(() => {})
    return done
  })

  on('command.run', { command: 'irori' }, async ($, e) => {
    const { verb, rest } = parseArgs(e.args)
    if ((POLICY_VERBS as readonly string[]).includes(verb)) {
      if (!PERSON_ORIGINS.includes(e.origin?.kind ?? '')) {
        return { text: 'Who may wake agents here only changes when a person types the command (or in the Irori app).' }
      }
      return { text: await changePolicy($, verb, rest) }
    }
    const view = await read($, attachAtom)
    const status = describeAttach(view)
    if (verb === '' || verb === 'status') return { text: `irori mod ${MOD_VERSION}\n${status}` }
    return { text: `irori mod ${MOD_VERSION}\n${status}\n\n${USAGE}` }
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (e.props.hasSurvey) return next(e)
    const view = await read($, attachAtom)
    const { Box, Text } = $.ui.resolve(e)
    const bad = view.status === 'off' || view.status === 'error'
    return (
      <Box>
        <Text dimColor={!bad} color={bad ? 'red' : undefined}>
          {bandText(view)}
        </Text>
      </Box>
    )
  })
}
