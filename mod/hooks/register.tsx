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
  AppendGate,
  appendArgs,
  appendsToEngage,
  BACKOFF_MIN_MS,
  MAX_WAKES,
  MOD_VERSION,
  NO_PATH_ERROR,
  POLICY_VERBS,
  STANDBY_SINCE,
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
  heldReminder,
  sideReminder,
  applyPause,
  pausedFromBoard,
  pausedRefusal,
  type PausedCard,
  type BoardLists,
  MAX_TITLES,
  toolArgs,
  validSpec,
} from './lib'

/** How long /irori reconnect waits for the new hello before it answers. */
const RECONNECT_WAIT_MS = 5_000

/** Only a person's Enter at the prompt may change the wake policy (design §5). */
const PERSON_ORIGINS = ['composer']

const INITIAL_VIEW: AttachView = { status: 'connecting', unread: 0, held: 0 }
const attachAtom = atom({ plugin: 'irori', key: 'attach' } as const, INITIAL_VIEW)
const wakesAtom = atom({ plugin: 'irori', key: 'wakes' } as const, [])
const appendsAtom = atom({ plugin: 'irori', key: 'appends' } as const, [])
/** Titles this mod set on the session (card #21), newest last. */
const titlesAtom = atom({ plugin: 'irori', key: 'titles' } as const, [] as string[])
/** Cards the person paused that this session holds (card #207). */
const pausedAtom = atom({ plugin: 'irori', key: 'paused' } as const, [] as PausedCard[])

// Reset on every (re)load; a reload also ends the running child.
let pluginOptions: PluginOptions = {}
let generation = 0
let stopChild: (() => void) | null = null
let backoffMs = BACKOFF_MIN_MS
let restartTimer: { cancel: () => void } | null = null
let ended = false
// Appends wait here while a main-loop response streams (card #197).
const appendGate = new AppendGate()
/** The main loop's running turn (card #207): what a stop cuts. */
let runningTurn: string | null = null
/** From the newest hello: where control and tool calls go, and who they act as. */
let api: { url: string; token: string } | null = null
let attachSecret = ''
/** The cwd the attach runs with, for a reattach the mod starts itself. */
let attachCwd = ''
/** hello's `v`: what the Core this session last attached to can do. */
let coreVersion = 0
/** Superseded: attach only once no other attach holds this session (GitHub #57). */
let standby = false
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
  const secret = attachSecret
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
    // A wrong token (the Core restarted) or a secret the Core no longer
    // knows (its stream ended): this attach is dead, whatever the runner
    // says. Drop it and attach again (card #111).
    if (response.status === 401 || (response.status === 403 && typeof error === 'string' && error.includes('X-Irori-Attach'))) {
      await reattach($, secret, `the Irori Core refused this session's attach (${response.status}); reattaching`)
    }
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

/**
 * Drops a dead attach and starts a new one, once per dead secret: calls that
 * fail together on the same secret start one reattach.
 */
async function reattach($: EngineInterface, deadSecret: string, why: string) {
  if (!deadSecret || attachSecret !== deadSecret || ended) return
  api = null
  attachSecret = ''
  $.ui.log(`irori: ${why}`, { to: 'debug' })
  await startAttach($, attachCwd, why)
}

/**
 * A tool call or a main turn after session.end: the session went on without
 * a session.start (a restart or resume in place), so attach it again, once
 * (card #244). session.end dropped the secret, so nothing sends a dead one.
 * True when this call started the attach.
 */
async function revive($: EngineInterface): Promise<boolean> {
  // generation 0: a reload that brought no session.start, so nothing attached.
  if (!ended && generation !== 0) return false
  ended = false
  if (!iroriPath()) return false
  await startAttach($, attachCwd, 'the session went on after it ended; reattaching')
  return true
}

/** Waits up to `ms` for a hello after a reattach; true once attached. */
async function attached($: EngineInterface, ms: number): Promise<boolean> {
  for (let waited = 0; waited < ms; waited += 250) {
    if (api && attachSecret) return true
    await $.clock.sleep(250)
  }
  return Boolean(api && attachSecret)
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
    const body = known.appended === true ? { token, appended: true } : { token }
    if (!known.delivered && (await report($, '/v1/deliveries/delivered', body))) {
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
  if (e.pause?.length || e.resume?.length) await update($, pausedAtom, list => applyPause(list, e))
  if (e.stop) await cutRunningTurn($)
  if (!e.reminder) {
    try {
      $.ui.toast(`irori: message from ${e.from || 'another agent'}`)
    } catch {
      // A toast the host refuses must not stop the wake.
    }
  }
  // `delivered` means accepted into Claude Code's queue, not "its turn started".
  // A submit made during a running turn resolves only when its own turn starts,
  // which can be past the Core's ReattachGrace; waiting for it would let the
  // Core reclaim the wake and wake the same deliveries again under a new token.
  // So a submit still pending after SUBMIT_ACCEPT_MS counts as queued.
  // A submit can also resolve as { drop } (a hook or the engine refused it):
  // the prompt never entered, so it goes in as an append instead (card #23).
  // A submit that throws at once (before it returns a promise) fails like one
  // that rejects, so it is appended instead of silently lost (card #23).
  let pending: ReturnType<EngineInterface['prompt']['submit']>
  try {
    pending = $.prompt.submit({ text: prompt })
  } catch (err) {
    pending = Promise.reject(err)
  }
  const submitted = Promise.resolve(pending).then(
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
  if (first?.failed) return appendInstead($, token, prompt, first.err)
  if (await report($, '/v1/deliveries/delivered', { token })) await patchWake($, token, { delivered: true })
  if (first === null) {
    // Reported while still queued; a later throw or drop appends it instead,
    // so the next turn still reads it.
    const outcome = await submitted
    if (outcome.failed) await appendInstead($, token, prompt, outcome.err)
  }
}

/**
 * The person's stop (card #207): cancels the main loop's running turn and
 * its tools, so the stop's prompt, submitted next, starts at once. The cut
 * turn's wakes are reported ended, so the Core's gate opens. An idle
 * session has nothing to cut.
 */
async function cutRunningTurn($: EngineInterface) {
  const turnId = runningTurn
  if (!turnId) return
  try {
    await $.turn.abort({ turnId })
  } catch (err) {
    // Not the running turn any more (it just ended): nothing to cut.
    $.ui.log(`irori: stop: ${errText(err)}`, { to: 'debug' })
  }
  if (runningTurn === turnId) runningTurn = null
  await markEnded($, turnId)
}

/**
 * A wake whose submit the host refused or that threw. Forgetting it used to
 * leave the Core re-issuing it under a new token every ReattachGrace, and a
 * host that kept refusing (card #23) never got it: 71 tries over three hours.
 * It goes into the conversation as an append instead, which starts no turn
 * and needs no idle prompt; it is reported delivered, and the next main-loop
 * turn, whatever starts it, engages it. Only an append that fails as well
 * forgets the wake, so the Core's re-send tries again.
 */
async function appendInstead($: EngineInterface, token: string, prompt: string, err: unknown) {
  $.ui.log(`irori: prompt.submit failed: ${errText(err)}; appending the wake for the next turn`, { to: 'debug' })
  let failure: unknown = undefined
  let failed = false
  try {
    const stored = await appendRow($, prompt)
    if (typeof stored?.deny === 'string') {
      failed = true
      failure = new Error(`append refused: ${stored.deny}`)
    }
  } catch (e) {
    failed = true
    failure = e
  }
  if (failed) return forgetWake($, token, failure)
  await patchWake($, token, { appended: true })
  // Reported even when the wake was reported delivered while its submit was
  // pending: appended tells the Core no prompt waits in the host's queue, so
  // this wake does not hold the session's next one back.
  if (await report($, '/v1/deliveries/delivered', { token, appended: true })) {
    await patchWake($, token, { delivered: true })
  }
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
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
  // Held behind a streaming response (card #197): reported delivered now, so
  // the Core does not reclaim and re-send it while a long step (an advisor
  // call can take minutes) runs past REATTACH_GRACE.
  let delivered = false
  if (appendGate.streaming && (await report($, '/v1/deliveries/delivered', { token }))) {
    delivered = true
    await patchAppend($, token, { delivered: true })
  }
  let failure: unknown = undefined
  let failed = false
  try {
    // `{ deny }`: a plugin above refused it and nothing was stored.
    const stored = await appendRow($, prompt)
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
  if (!delivered && (await report($, '/v1/deliveries/delivered', { token }))) await patchAppend($, token, { delivered: true })
}

/** The mod's one way into `$.session.append`: never while a main response streams (card #197). */
async function appendRow($: EngineInterface, prompt: string) {
  await appendGate.idle()
  return $.session.append(appendArgs(prompt))
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

/** A wake neither submitted nor appended: forget it, so the Core's re-send (reattach, or its reclaim) tries again. */
async function forgetWake($: EngineInterface, token: string, err: unknown) {
  $.ui.log(`irori: could not hand the wake over: ${errText(err)}`, { to: 'debug' })
  await update($, wakesAtom, list => list.filter(w => w.token !== token))
}

async function onHello($: EngineInterface, e: AttachEvent) {
  if (e.api?.url && e.api.token) api = { url: e.api.url.replace(/\/+$/, ''), token: e.api.token }
  attachSecret = String(e.attachSecret ?? '')
  coreVersion = Number(e.v ?? 0)
  standby = false
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
  void refreshPaused($, e.agentKey).catch(() => {})
}

/** Rebuilds the paused set from the board at hello (card #207). */
async function refreshPaused($: EngineInterface, agentKey: string | undefined) {
  const board = await callCore($, 'GET', '/v1/board?done=1')
  if (!board.ok) return
  const before = await read($, pausedAtom)
  await update($, pausedAtom, () => pausedFromBoard(board.body as BoardLists, agentKey, before))
}

async function handleEvent($: EngineInterface, e: AttachEvent): Promise<'superseded' | void> {
  switch (e.type) {
    case 'hello':
      await onHello($, e)
      return
    case 'deliver':
      // Detached: a submit can wait for the running turn, and other events keep flowing.
      void onDeliver($, e).catch(err => {
        try {
          $.ui.log(`irori: wake not handed over: ${errText(err)}`, { to: 'debug' })
        } catch {
          // The module unloaded; nothing left to log to.
        }
      })
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
      // `irori attach` says error only while it has no stream (Irori down,
      // a stream that dropped, a session off the roster): the secret from
      // the last hello is dead until the next one (card #111).
      $.ui.log(`irori attach: ${e.message ?? 'error'}`, { to: 'debug' })
      api = null
      attachSecret = ''
      await setView($, { status: 'connecting', error: e.message })
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
async function startAttach($: EngineInterface, cwd: string, why?: string) {
  stopChild?.()
  stopChild = null
  restartTimer?.cancel()
  restartTimer = null
  const run = ++generation
  attachCwd = cwd
  const path = iroriPath()
  if (!path) {
    await setView($, { status: 'off', error: NO_PATH_ERROR })
    return
  }
  const sessionId = await $.session.id()
  // Standby keeps saying why it waits; its hello sets the view.
  if (!standby) await setView($, why ? { status: 'connecting', error: why } : { status: 'connecting' })

  void (async () => {
    let superseded = false
    let exit = ''
    try {
      const child = $.process.spawn({
        argv: [path, 'attach', '--session', sessionId, '--cwd', cwd, '--mod-version', MOD_VERSION, ...(standby ? ['--standby'] : [])],
      })
      // A newer startAttach ran while this one awaited: this child is stale.
      if (run !== generation) {
        void child.return(undefined as never)
        return
      }
      stopChild = () => void child.return(undefined as never)
      let buffer = ''
      for (;;) {
        const step = await child.next()
        if (step.done) {
          exit = `irori attach exited (code ${step.value?.code ?? '?'})`
          break
        }
        if (run !== generation) {
          // Leaving the loop without return() would leave it running.
          void child.return(undefined as never)
          return
        }
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
    if (superseded) {
      // A newer attach for this session took over (a reload, another copy).
      // It may leave while this session goes on, so wait for that: standby
      // never takes the session from a live attach, so two live copies can't
      // keep kicking each other off (GitHub #57). A Core too old to refuse
      // would be kicked instead, so stop there, and leave the view alone:
      // after a reload it is the new copy's.
      if (coreVersion < STANDBY_SINCE) return
      standby = true
      await setView($, { status: 'superseded', error: undefined })
    } else {
      await setView($, { status: 'connecting', error: `${exit}; retrying` })
    }
    scheduleRestart($, cwd, run)
  })().catch(() => {
    // The module unloaded while the loop ran; nothing left to update.
  })
}

/**
 * A main-loop turn started: engage each wake whose token its text carries,
 * and each wake that went in as an append (its token is never a turn's text).
 */
async function markEngaged($: EngineInterface, text: string, turnId: string) {
  const named = (await read($, wakesAtom)).filter(
    w => !w.reminder && !w.turnId && !w.ended && (w.appended === true || text.includes(w.token)),
  )
  for (const w of named) {
    await patchWake($, w.token, { turnId })
    // An appended wake did not start this turn: the Core then sets no running
    // wake, so a message sent from it is not counted a hop deeper.
    const body = w.appended === true && !text.includes(w.token) ? { token: w.token, turnId, appended: true } : { token: w.token, turnId }
    if (await report($, '/v1/deliveries/engaged', body)) {
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

/** What a tool call says while this session has no attach. */
async function notAttached($: EngineInterface): Promise<string> {
  const view = await read($, attachAtom)
  return `Error: this session isn't attached to Irori (${describeAttach(view)}). It reattaches by itself: try again in a few seconds. If it stays this way, check that the Irori app is running; /irori shows the status.`
}

async function callTool($: EngineInterface, e: Record<string, unknown>, budgetMs: number): Promise<string> {
  const name = bareToolName(String(e.tool ?? ''))
  if (!name) return 'Error: not an Irori tool.'
  const path = `/v1/tools/${encodeURIComponent(name)}`
  // After session.end, wait for the new attach's hello before calling.
  if ((await revive($)) && !(await attached($, Math.min(15_000, budgetMs - 5_000)))) return notAttached($)
  const secret = attachSecret
  let answer = await callCore($, 'POST', path, { args: toolArgs(e) })
  // A refused attach ran nothing: once the reattach says hello, try again.
  const refused = !answer.ok && (answer.status === 401 || answer.status === 403) && attachSecret !== secret
  // Wait for the new hello within this hook's budget, keeping time for the retry.
  if (refused && (await attached($, Math.min(15_000, budgetMs - 5_000)))) {
    answer = await callCore($, 'POST', path, { args: toolArgs(e) })
  }
  if (!answer.ok) {
    if (!api) return notAttached($)
    return `Error: ${answer.error}`
  }
  const text = (answer.body as { text?: unknown } | undefined)?.text
  return typeof text === 'string' ? text : 'Error: the Irori Core gave no result.'
}

/** /irori reconnect: attaches now, whatever state the last attach ended in. */
async function reconnect($: EngineInterface): Promise<string> {
  if (!iroriPath()) return `irori mod ${MOD_VERSION}\n${NO_PATH_ERROR}`
  ended = false
  standby = false
  api = null
  attachSecret = ''
  backoffMs = BACKOFF_MIN_MS
  await startAttach($, attachCwd, 'reconnecting')
  await attached($, RECONNECT_WAIT_MS)
  return `irori mod ${MOD_VERSION}\n${describeAttach(await read($, attachAtom))}`
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

/**
 * Adds the held-cards reminder to a UserPromptSubmit result while this
 * session holds cards (GitHub #3): what the agent needs from the person goes
 * on the card as a blocker, even when the person asked in this terminal. Any
 * failure returns the result unchanged.
 */
async function withHeld<R extends { additionalContext?: string[] }>($: EngineInterface, result: R): Promise<R> {
  try {
    const view = await read($, attachAtom)
    if (view.status !== 'attached' || !view.agentKey) return result
    const board = await callCore($, 'GET', '/v1/board?done=1')
    if (!board.ok) return result
    // Which side this device runs (card #496), then the cards it holds.
    const texts = [sideReminder(board.body as BoardLists), heldReminder(board.body as BoardLists, view.agentKey)].filter(Boolean)
    if (!texts.length) return result
    return { ...result, additionalContext: [...(result?.additionalContext ?? []), ...texts] }
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
    standby = false
    await onSessionStart($, e.cwd)
    return started
  })

  // Irori's name for a session nobody named shows as its title (card #21).
  // SessionStart usually fires before the attach, so a new session takes
  // it at its first prompt; a resumed one already has its name.
  on('classic.SessionStart', async ($, e, next) => withTitle($, await next(e), e.session_title))
  on('classic.UserPromptSubmit', async ($, e, next) =>
    withHeld($, await withTitle($, await next(e), e.session_title)),
  )

  on('session.end', async ($, e, next) => {
    ended = true
    restartTimer?.cancel()
    stopChild?.()
    stopChild = null
    // The Core forgets this attach with its stream: its secret is dead.
    api = null
    attachSecret = ''
    return next(e)
  })

  // While the person has a card paused (card #207), every tool but Irori's
  // own is refused with why; Irori's go on to the hook below.
  on('tool.call', async ($, e, next) => {
    const tool = String((e as unknown as { tool?: unknown }).tool ?? '')
    const why = pausedRefusal(await read($, pausedAtom), tool)
    if (why) return { result: why }
    return next(e)
  })

  // Every Irori tool, whichever names the Core listed (registered late, after hello).
  on('tool.call', { tool: /^mcp__irori__/ }, async ($, e, next) => ({
    result: await callTool($, e as unknown as Record<string, unknown>, next.budget.remainingMs),
  })).catch(() => ({ result: 'Error: the irori mod failed on this call (it timed out or threw); try again.' }))

  on('turn.start', async ($, e, next) => {
    const started = await next(e)
    // Subagent turns can quote a token; only the main loop engages a wake,
    // and only the main loop's turns read appends.
    if (e.agentId === undefined) {
      void revive($).catch(() => {})
      runningTurn = e.turnId
      void markEngaged($, e.text, e.turnId).catch(() => {})
      void markAppendsEngaged($, e.turnId).catch(() => {})
    }
    return started
  })

  // A main-loop model request is streaming: the mod's appends wait for its
  // end (card #197), then go in order, after the whole response is kept.
  on('turn.step', async function* ($, e, next) {
    if (e.agentId !== undefined) return yield* next(e)
    appendGate.enter()
    try {
      return yield* next(e)
    } finally {
      appendGate.leave()
    }
  })

  on('turn.complete', async ($, e, next) => {
    const done = await next(e)
    if (e.agentId === undefined) {
      if (runningTurn === e.turnId) runningTurn = null
      void markEnded($, e.turnId).catch(() => {})
    }
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
    if (verb === 'reconnect') return { text: await reconnect($) }
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
