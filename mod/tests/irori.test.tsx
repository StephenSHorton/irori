// Run with: claude plugin test mod
import { describe, expect, mock, test } from 'claude-code/testing'
import type { On, PromptOrigin } from 'claude-code'

// The test runtime has timers; the mod's lib (es2023, no DOM) doesn't declare them.
declare function setTimeout(fn: () => void, ms: number): unknown
const pause = (ms: number) => new Promise<void>(resolve => setTimeout(() => resolve(), ms))

const PERSON: PromptOrigin = { kind: 'composer' }
const PLUGIN: PromptOrigin = { kind: 'plugin', name: 'other' } as PromptOrigin
const PRESENTATION = { isFullscreen: false, columns: 120 }
function irori(args: string, origin: PromptOrigin = PLUGIN) {
  return { command: 'irori', args, origin, presentation: PRESENTATION }
}

const IRORI = 'C:/Program Files/Irori/irori.exe'
const SESSION = 'b886ef92-0000-4000-8000-000000000001'
const API = 'http://127.0.0.1:47811'
const TOKEN = '[irori-wake:wake_01]'
const OPTIONS = { options: { iroriPath: IRORI }, timeoutMs: 20_000 }

const hello = {
  type: 'hello',
  v: 1,
  session: SESSION,
  agentKey: `dev1/${SESSION}`,
  name: 'api work',
  api: { url: API, token: 'bearer-1' },
  attachSecret: 'secret-1',
}

const specs = [
  { name: 'send', description: 'Send a message.', inputSchema: { type: 'object', properties: { to: { type: 'string' } } }, readOnly: false, clients: ['claude', 'grok'] },
  { name: 'reply', description: 'Reply to a delivery.', inputSchema: { type: 'object' }, readOnly: false, clients: ['claude', 'grok'] },
  { name: 'bad name!', description: 'not a valid tool name' },
]

function line(event: unknown): string {
  return `${JSON.stringify(event)}\n`
}

async function until(check: () => boolean, what: string) {
  for (let i = 0; i < 200; i++) {
    if (check()) return
    await pause(10)
  }
  throw new Error(`timed out waiting for ${what}`)
}

/** A child's stdout the test writes to; `end()` makes the child exit. */
function makeChild() {
  const queue: Array<string | null> = []
  let wake: (() => void) | null = null
  return {
    write(text: string) {
      queue.push(text)
      wake?.()
    },
    end() {
      queue.push(null)
      wake?.()
    },
    async next(): Promise<string | null> {
      while (queue.length === 0) await new Promise<void>(r => (wake = r))
      wake = null
      return queue.shift() as string | null
    },
  }
}

type Call = { url: string; method: string; headers: Record<string, string>; body?: string }

/** The engine beneath the mod: spawn, fetch, submit, register, toast. */
function world(on: On, opts: { exitCode?: number } = {}) {
  const children: Array<ReturnType<typeof makeChild>> = []
  const spawned: Array<readonly string[]> = []
  const calls: Call[] = []
  const submitted: string[] = []
  const registered: Array<{ name: string; description: string; inputSchema?: unknown }> = []
  const toasts: string[] = []

  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('session.id', () => ({ value: SESSION }) as never)
  on('command.register', () => ({ value: { command: 'irori' } }) as never)
  on('ui.toast', ($, e) => {
    toasts.push(String(e.text))
    return { value: undefined } as never
  })
  on('ui.log', () => ({ value: undefined }) as never)
  on('tool.register', ($, e) => {
    registered.push(e as never)
    return { value: undefined } as never
  })
  on('prompt.submit', ($, e) => {
    submitted.push(e.text)
    return { drop: 'recorded by the test' }
  })
  on('turn.start', ($, e) => ({ turnId: e.turnId }))
  on('turn.complete', () => ({ text: '' }) as never)
  on('http.fetch', ($, e) => {
    calls.push({ url: e.url, method: e.init?.method ?? 'GET', headers: { ...(e.init?.headers ?? {}) }, body: e.init?.body })
    if (e.url.endsWith('/v1/tools?client=claude')) return { value: { ok: true, status: 200, headers: {}, text: JSON.stringify(specs) } } as never
    if (e.url.includes('/v1/tools/')) return { value: { ok: true, status: 200, headers: {}, text: '{"text":"Sent: msg_01 to bob."}' } } as never
    if (e.url.endsWith('/v1/policy') && (e.init?.method ?? 'GET') === 'GET')
      return { value: { ok: true, status: 200, headers: {}, text: '{"mode":"everyone","block":[],"allow":[],"paused":false}' } } as never
    if (e.url.endsWith('/v1/policy')) return { value: { ok: true, status: 200, headers: {}, text: e.init?.body ?? '{}' } } as never
    return { value: { ok: true, status: 200, headers: {}, text: '{"ok":true}' } } as never
  })
  on('process.spawn', async function* ($, e) {
    spawned.push(e.argv)
    const child = makeChild()
    children.push(child)
    for (;;) {
      const text = await child.next()
      if (text === null) return { value: { code: opts.exitCode ?? 1, signal: null } } as never
      yield { stream: 'stdout' as const, text }
    }
  })
  return { children, spawned, calls, submitted, registered, toasts }
}

const posts = (calls: Call[], path: string) => calls.filter(c => c.method === 'POST' && c.url === API + path)

describe('attach', () => {
  test('with no iroriPath the band says to set it, and nothing is spawned', async ($, on) => {
    const w = world(on)
    await $.session.start({ cwd: 'C:/work', surface: 'terminal', isInteractive: true })
    expect(w.spawned).toEqual([])
    expect(w.toasts.join('\n')).toContain('set iroriPath in /plugin settings')
    for (const surface of ['terminal', 'desktop'] as const) {
      const band = await $.ui.mount({ plugin: 'irori', surface, component: 'AbovePrompt', props: {} } as never)
      expect(await band.find({ type: 'Text', text: /set iroriPath in \/plugin settings/ })).toBeDefined()
      await band.unmount()
    }
    const ran = await $.command.run(irori(''))
    expect(ran.text).toContain('irori mod 0.2.0')
    expect(ran.text).toContain('set iroriPath')
  })

  test('a wake is submitted, then delivered, engaged by its turn and ended with it', OPTIONS, async ($, on) => {
    const w = world(on)
    await $.session.start({ cwd: 'C:/work/api', surface: 'terminal', isInteractive: true })
    await until(() => w.children.length === 1, 'spawn')
    expect(w.spawned[0]).toEqual([IRORI, 'attach', '--session', SESSION, '--cwd', 'C:/work/api', '--mod-version', '0.2.0'])

    // hello split across two pieces, a blank keepalive, and a stray log line.
    const h = line(hello)
    w.children[0]!.write(h.slice(0, 20))
    w.children[0]!.write(h.slice(20) + '\n' + 'not json\n' + line({ type: 'inbox', unread: 3, held: 1 }))
    await until(() => w.registered.length === 2, 'tools registered')
    expect(w.registered.map(t => t.name)).toEqual(['send', 'reply'])
    const list = w.calls.find(c => c.url === `${API}/v1/tools?client=claude`)!
    expect(list.method).toBe('GET')
    expect(list.headers.authorization).toBe('Bearer bearer-1')
    expect(list.headers['X-Irori-Attach']).toBe('secret-1')

    for (const surface of ['terminal', 'desktop'] as const) {
      const band = await $.ui.mount({ plugin: 'irori', surface, component: 'AbovePrompt', props: {} } as never)
      expect(await band.find({ type: 'Text', text: 'irori · 3 unread · 1 held' })).toBeDefined()
      await band.unmount()
    }

    const prompt = `${TOKEN}\n<irori-message from="bob" key="dev1/s2" delivery="del_1" trust="untrusted">hi</irori-message>`
    w.children[0]!.write(line({ type: 'deliver', wakeId: 'wake_01', token: TOKEN, deliveryIds: ['del_1'], prompt, from: 'bob' }))
    await until(() => posts(w.calls, '/v1/deliveries/delivered').length === 1, 'delivered')
    expect(w.submitted).toEqual([prompt])
    expect(posts(w.calls, '/v1/deliveries/delivered')[0]!.body).toBe(JSON.stringify({ token: TOKEN }))
    expect(w.toasts.some(t => t.includes('message from bob'))).toBe(true)

    // The same wake again (a reattach) is not submitted twice.
    w.children[0]!.write(line({ type: 'deliver', wakeId: 'wake_01', token: TOKEN, deliveryIds: ['del_1'], prompt, from: 'bob' }))
    await pause(50)
    expect(w.submitted.length).toBe(1)

    // Another turn, and a subagent's turn quoting the token, engage nothing.
    await $.turn.start({ text: 'something else', turnId: 'turn-0' })
    await $.turn.start({ text: `The irori plugin sent a message:\n${prompt}`, turnId: 'turn-sub', agentId: 'agent-1' } as never)
    await $.turn.start({ text: `The irori plugin sent a message:\n${prompt}`, turnId: 'turn-1' })
    await until(() => posts(w.calls, '/v1/deliveries/engaged').length > 0, 'engaged')
    await pause(50)
    expect(posts(w.calls, '/v1/deliveries/engaged').map(c => c.body)).toEqual([JSON.stringify({ token: TOKEN, turnId: 'turn-1' })])

    await $.turn.complete({ turnId: 'turn-0', answer: '', durationMs: 1, isAborted: false, reason: 'answer' } as never)
    await $.turn.complete({ turnId: 'turn-1', answer: '', durationMs: 1, isAborted: false, reason: 'answer' } as never)
    await until(() => posts(w.calls, '/v1/deliveries/turn-ended').length > 0, 'turn-ended')
    await pause(50)
    expect(posts(w.calls, '/v1/deliveries/turn-ended').map(c => c.body)).toEqual([JSON.stringify({ token: TOKEN, turnId: 'turn-1' })])
    // Every control call carries the bearer and the attach secret, never `auth`.
    for (const c of w.calls) {
      expect(c.headers.authorization).toBe('Bearer bearer-1')
      expect(c.headers['X-Irori-Attach']).toBe('secret-1')
    }

    // A held item and a move only toast.
    w.children[0]!.write(line({ type: 'held', deliveryId: 'del_2', from: 'grok-1', why: 'bypass_receiver', preview: 'x' }))
    w.children[0]!.write(line({ type: 'moved', deliveryId: 'del_3', to: 'front desk' }))
    await until(() => w.toasts.some(t => t.includes('front desk')), 'moved toast')
    expect(w.toasts.some(t => t.includes('held a message from grok-1 (bypass_receiver)'))).toBe(true)
    expect(w.submitted.length).toBe(1)
  })

  test('a reminder wake is submitted but never engaged', OPTIONS, async ($, on) => {
    const w = world(on)
    await $.session.start({ cwd: 'C:/work', surface: 'terminal', isInteractive: true })
    await until(() => w.children.length === 1, 'spawn')
    const token = '[irori-reminder:wake_02]'
    w.children[0]!.write(line(hello) + line({ type: 'deliver', token, reminder: true, deliveryIds: ['del_1'], prompt: `${token} reminder` }))
    await until(() => w.submitted.length === 1, 'submit')
    await $.turn.start({ text: `${token} reminder`, turnId: 'turn-1' })
    await pause(50)
    expect(posts(w.calls, '/v1/deliveries/engaged')).toEqual([])
  })

  test('a child that exits is restarted with backoff; a superseded one is not', OPTIONS, async ($, on) => {
    const clock = mock.clock(on)
    const w = world(on)
    await $.session.start({ cwd: 'C:/work', surface: 'terminal', isInteractive: true })
    await until(() => w.children.length === 1, 'spawn')
    w.children[0]!.write(line(hello))
    w.children[0]!.end()
    await clock.settle()
    expect(w.children.length).toBe(1)
    await clock.advance(1_000)
    await until(() => w.children.length === 2, 'restart')

    w.children[1]!.write(line({ type: 'superseded' }))
    w.children[1]!.end()
    await clock.settle()
    await clock.advance(60_000)
    await pause(50)
    expect(w.children.length).toBe(2)
  })
})

describe('tools', () => {
  test('a tool call posts its args, without the envelope, as the attached session', OPTIONS, async ($, on) => {
    const w = world(on)
    await $.session.start({ cwd: 'C:/work', surface: 'terminal', isInteractive: true })
    await until(() => w.children.length === 1, 'spawn')
    w.children[0]!.write(line(hello))
    await until(() => w.registered.length === 2, 'tools registered')
    const ran = await $.tool.call({ tool: 'mcp__irori__send', to: 'bob', text: 'hi' } as never)
    expect(String((ran as { result?: unknown }).result)).toBe('Sent: msg_01 to bob.')
    const sent = posts(w.calls, '/v1/tools/send')
    expect(sent.length).toBe(1)
    expect(JSON.parse(sent[0]!.body!)).toEqual({ args: { to: 'bob', text: 'hi' } })
    expect(sent[0]!.headers['X-Irori-Attach']).toBe('secret-1')
  })

  test('before hello a tool call says the session is not attached', OPTIONS, async ($, on) => {
    world(on)
    const ran = await $.tool.call({ tool: 'mcp__irori__send', to: 'bob', text: 'hi' } as never)
    expect(String((ran as { result?: unknown }).result)).toMatch(/^Error: this session isn't attached to Irori/)
  })
})

describe('/irori', () => {
  test('a command the model or a plugin runs cannot change the policy', OPTIONS, async ($, on) => {
    const w = world(on)
    await $.session.start({ cwd: 'C:/work', surface: 'terminal', isInteractive: true })
    await until(() => w.children.length === 1, 'spawn')
    w.children[0]!.write(line(hello))
    await until(() => w.registered.length === 2, 'tools registered')
    for (const args of ['mode allowlist', 'block grok-1', 'allow bob', 'deny bob', 'unblock grok-1', 'pause', 'resume']) {
      const ran = await $.command.run(irori(args))
      expect(ran.text).toContain('only changes when a person types')
    }
    expect(w.calls.filter(c => c.url.endsWith('/v1/policy'))).toEqual([])
    const status = await $.command.run(irori(''))
    expect(status.text).toContain('irori mod 0.2.0')
    expect(status.text).toContain('Attached as api work')
  })

  test('a person typing /irori block reads the policy, edits it and posts it as composer', OPTIONS, async ($, on) => {
    const w = world(on)
    await $.session.start({ cwd: 'C:/work', surface: 'terminal', isInteractive: true })
    await until(() => w.children.length === 1, 'spawn')
    w.children[0]!.write(line(hello))
    await until(() => w.registered.length === 2, 'tools registered')
    const ran = await $.command.run(irori('block grok-1', PERSON))
    expect(ran.text).toContain('Blocked grok-1')
    expect(ran.text).toContain('Policy: everyone mode, blocked: grok-1.')
    const policy = w.calls.filter(c => c.url === `${API}/v1/policy`)
    expect(policy.map(c => c.method)).toEqual(['GET', 'POST'])
    expect(policy[0]!.headers['X-Irori-Origin']).toBeUndefined()
    expect(policy[1]!.headers['X-Irori-Origin']).toBe('composer')
    expect(policy[1]!.headers['X-Irori-Attach']).toBe('secret-1')
    expect(JSON.parse(policy[1]!.body!)).toEqual({ mode: 'everyone', block: ['grok-1'], allow: [], paused: false })

    const paused = await $.command.run(irori('pause', PERSON))
    expect(paused.text).toContain('Paused')
    const bad = await $.command.run(irori('mode sometimes', PERSON))
    expect(bad.text).toContain('Use /irori mode everyone or /irori mode allowlist.')
  })
})
