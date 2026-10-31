// Pure helper tests. Run with: bun test (from mod/; bunfig.toml roots it here).
// The engine-driven suite is tests/irori.test.tsx: claude plugin test mod.
import { describe, expect, test } from 'bun:test'

import {
  appendArgs,
  appendsToEngage,
  bandText,
  bareToolName,
  describePolicy,
  editPolicy,
  heldReminder,
  nextBackoff,
  titleToSet,
  parseArgs,
  takeEvents,
  toolArgs,
  validSpec,
} from '../hooks/lib'
import type { Policy, TrackedAppend } from '../types'

const base: Policy = { mode: 'everyone', block: [], allow: [], paused: false }

describe('takeEvents', () => {
  test('keeps the unfinished tail and skips keepalives and junk', () => {
    const { events, rest } = takeEvents('{"type":"hello","v":1}\n\n   \nnot json\n{"broken"\n{"type":"inbox","unread":2}\n{"type":"del')
    expect(events).toEqual([{ type: 'hello', v: 1 }, { type: 'inbox', unread: 2 }])
    expect(rest).toBe('{"type":"del')
  })

  test('a line split across pieces parses once whole', () => {
    let buffer = ''
    const seen: unknown[] = []
    for (const piece of ['{"type":"deli', 'ver","token":"[irori-wake:w1]"}', '\n']) {
      const out = takeEvents(buffer + piece)
      buffer = out.rest
      seen.push(...out.events)
    }
    expect(seen).toEqual([{ type: 'deliver', token: '[irori-wake:w1]' }])
  })

  test('CRLF line ends and objects without a type are handled', () => {
    expect(takeEvents('{"type":"superseded"}\r\n{"x":1}\n').events).toEqual([{ type: 'superseded' }])
  })
})

describe('small helpers', () => {
  test('backoff doubles from 1 s to 16 s', () => {
    const seen = [1000]
    for (let i = 0; i < 6; i++) seen.push(nextBackoff(seen[seen.length - 1]!))
    expect(seen).toEqual([1000, 2000, 4000, 8000, 16000, 16000, 16000])
  })

  test('args keep names with spaces', () => {
    expect(parseArgs('  Block   api work ')).toEqual({ verb: 'block', rest: 'api work' })
    expect(parseArgs('')).toEqual({ verb: '', rest: '' })
  })

  test('tool args drop the envelope', () => {
    expect(toolArgs({ tool: 'mcp__irori__send', tool_use_id: 't1', agentId: 'a', to: 'bob', text: 'hi' })).toEqual({ to: 'bob', text: 'hi' })
    expect(bareToolName('mcp__irori__task_create')).toBe('task_create')
    expect(bareToolName('mcp__other__send')).toBe('')
  })

  test('only specs the engine accepts are registered', () => {
    expect(validSpec({ name: 'send', description: 'x' })).toBe(true)
    expect(validSpec({ name: 'bad name', description: 'x' })).toBe(false)
    expect(validSpec({ name: 'send' })).toBe(false)
    expect(validSpec(null)).toBe(false)
  })

  test('the band text', () => {
    expect(bandText({ status: 'attached', unread: 2, held: 1 })).toBe('irori · 2 unread · 1 held')
    expect(bandText({ status: 'off', error: 'set iroriPath in /plugin settings', unread: 0, held: 0 })).toBe('irori · set iroriPath in /plugin settings')
    expect(bandText({ status: 'connecting', unread: 0, held: 0 })).toBe('irori · connecting…')
  })
})

describe('editPolicy', () => {
  test('mode', () => {
    const out = editPolicy(base, 'mode', 'Allowlist')
    expect('policy' in out && out.policy.mode).toBe('allowlist')
    expect('error' in editPolicy(base, 'mode', 'nobody')).toBe(true)
  })

  test('mode open (the old Full auto) is everyone now', () => {
    const out = editPolicy(base, 'mode', 'OPEN')
    if ('error' in out) throw new Error(out.error)
    expect(out.policy.mode).toBe('everyone')
    const blocked = editPolicy({ ...base, mode: 'open' }, 'block', 'grok-1')
    expect('policy' in blocked && blocked.policy.mode).toBe('everyone')
  })

  test('block, unblock, allow, deny replace case-insensitively and keep the rest', () => {
    let p = base
    for (const [verb, who] of [['block', 'grok-1'], ['block', 'GROK-1'], ['allow', 'bob'], ['allow', 'api work']] as const) {
      const out = editPolicy(p, verb, who)
      if ('error' in out) throw new Error(out.error)
      p = out.policy
    }
    expect(p.block).toEqual(['GROK-1'])
    expect(p.allow).toEqual(['bob', 'api work'])
    const un = editPolicy(p, 'unblock', 'grok-1')
    const de = editPolicy(p, 'deny', 'bob')
    expect('policy' in un && un.policy.block).toEqual([])
    expect('policy' in de && de.policy.allow).toEqual(['api work'])
    expect('error' in editPolicy(p, 'block', '  ')).toBe(true)
  })

  test('pause and resume', () => {
    const paused = editPolicy(base, 'pause', '')
    expect('policy' in paused && paused.policy.paused).toBe(true)
    const resumed = editPolicy({ ...base, paused: true }, 'resume', '')
    expect('policy' in resumed && resumed.policy.paused).toBe(false)
  })

  test('a policy from the Core with null lists still edits', () => {
    const out = editPolicy({ mode: 'everyone', block: null, allow: null, paused: false } as unknown as Policy, 'block', 'x')
    expect('policy' in out && out.policy).toEqual({ mode: 'everyone', block: ['x'], allow: [], paused: false })
  })

  test('describePolicy', () => {
    expect(describePolicy({ ...base, block: ['grok-1'], paused: true })).toBe('Policy: everyone mode, blocked: grok-1 · paused.')
    expect(describePolicy({ ...base, mode: 'allowlist' })).toBe('Policy: allowlist mode, allowed: nobody.')
    expect(describePolicy({ ...base, mode: 'open' })).toBe('Policy: everyone mode, blocked: nobody.')
  })
})

describe('appends', () => {
  test('the append is one user-role text row', () => {
    expect(appendArgs('[irori-append:w1] hi')).toEqual({ message: { type: 'user', content: [{ type: 'text', text: '[irori-append:w1] hi' }] } })
  })

  test('a turn start engages delivered appends not yet engaged or being engaged', () => {
    const a = (token: string, patch: Partial<TrackedAppend>): TrackedAppend => ({
      token,
      deliveryIds: [],
      appended: true,
      delivered: false,
      engaged: false,
      ...patch,
    })
    const list = [
      a('pending', { appended: false }),
      a('appended', {}),
      a('delivered', { delivered: true }),
      a('engaging', { delivered: true, turnId: 't1' }),
      a('engaged', { delivered: true, engaged: true, turnId: 't1' }),
    ]
    expect(appendsToEngage(list).map(x => x.token)).toEqual(['delivered'])
    expect(appendsToEngage([])).toEqual([])
  })
})

describe('titleToSet (card #21)', () => {
  const irori = { name: 'heron', nameSource: 'irori', aiTitle: 'Fix the parser' }
  test("sets Irori's name over no title, its own earlier title, or the ai-title", () => {
    expect(titleToSet(irori, undefined, [])).toBe('heron')
    expect(titleToSet(irori, '', [])).toBe('heron')
    expect(titleToSet(irori, 'otter', ['otter'])).toBe('heron')
    expect(titleToSet(irori, 'Fix the parser', [])).toBe('heron')
  })
  test('leaves a title already set to the name', () => {
    expect(titleToSet(irori, 'heron', [])).toBeUndefined()
  })
  test('never overrides a title someone chose, even before the Core has seen it', () => {
    expect(titleToSet(irori, 'api work', [])).toBeUndefined()
    expect(titleToSet({ ...irori, nameSource: 'user', name: 'api work' }, '', [])).toBeUndefined()
  })
  test('sets nothing without an Irori name', () => {
    expect(titleToSet({ name: 'C--Users', nameSource: '' }, '', [])).toBeUndefined()
    expect(titleToSet({ nameSource: 'irori' }, '', [])).toBeUndefined()
  })
})

describe('heldReminder', () => {
  const me = 'dev-a/me'
  test('names the cards this session holds, once each', () => {
    const r = heldReminder(
      {
        working: [{ short: '68', title: 'Fix it', holders: [me] }, { short: '70', title: 'Not mine', holders: ['dev-a/other'] }],
        blocked: [{ short: '69', title: 'Waiting', holders: ['dev-a/other', me] }],
        needsYou: [{ short: '69', title: 'Waiting', holders: [me] }],
      },
      me,
    )
    expect(r).toContain('#69 "Waiting", #68 "Fix it"')
    expect(r).not.toContain('#70')
    expect(r).toContain('board_block')
  })

  test('says nothing when it holds no card or the board is missing', () => {
    expect(heldReminder({ working: [{ short: '1', holders: ['x'] }] }, me)).toBe('')
    expect(heldReminder(undefined, me)).toBe('')
    expect(heldReminder({ working: [{ short: '1', holders: [me] }] }, undefined)).toBe('')
  })
})
