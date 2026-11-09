// Pure helper tests. Run with: bun test (from mod/; bunfig.toml roots it here).
// The engine-driven suite is tests/irori.test.tsx: claude plugin test mod.
import { describe, expect, test } from 'bun:test'

import {
  AppendGate,
  applyPause,
  pausedFromBoard,
  pausedRefusal,
  appendArgs,
  appendsToEngage,
  bandText,
  bareToolName,
  describePolicy,
  editPolicy,
  deviceReminder,
  heldReminder,
  sideReminder,
  nextBackoff,
  titleToSet,
  parseArgs,
  takeEvents,
  toolArgs,
  validSpec,
  MAX_USAGE_WINDOWS,
  measureMovedLimits,
  permissionAnswer,
  permissionBody,
  reportsPermission,
  usageBody,
  accountFromConfig,
  claudeConfigPath,
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

  test('pause is gone, resume lifts the old Pause all', () => {
    const paused = editPolicy(base, 'pause', '')
    expect('error' in paused && paused.error).toContain('Stop here')
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
    // Card #509: ready work asks to merge, never waits on the PR.
    expect(r).toContain('Can I merge PR #N')
    expect(r).toContain('never sit on an external wait for a merge')
  })

  test('says nothing when it holds no card or the board is missing', () => {
    expect(heldReminder({ working: [{ short: '1', holders: ['x'] }] }, me)).toBe('')
    expect(heldReminder(undefined, me)).toBe('')
    expect(heldReminder({ working: [{ short: '1', holders: [me] }] }, undefined)).toBe('')
  })
})

describe('AppendGate (card #197)', () => {
  test('idle resolves at once when no step streams', async () => {
    const g = new AppendGate()
    expect(g.streaming).toBe(false)
    await g.idle()
  })
  test('waiters go on in order when the last step ends', async () => {
    const g = new AppendGate()
    const order: string[] = []
    g.enter()
    g.enter()
    expect(g.streaming).toBe(true)
    const a = g.idle().then(() => order.push('a'))
    const b = g.idle().then(() => order.push('b'))
    g.leave()
    await Promise.resolve()
    expect(order).toEqual([])
    g.leave()
    await Promise.all([a, b])
    expect(order).toEqual(['a', 'b'])
    expect(g.streaming).toBe(false)
    g.leave()
    expect(g.streaming).toBe(false)
  })
})

describe('Pause (card #207)', () => {
  const me = 'dev1/aaaa'
  test('applyPause adds each paused card once and drops resumed ones', () => {
    let list = applyPause([], { pause: ['#7'], from: 'Stephen' })
    list = applyPause(list, { pause: ['#7', '#9'], from: 'Stephen' })
    expect(list).toEqual([
      { ref: '#7', by: 'Stephen' },
      { ref: '#9', by: 'Stephen' },
    ])
    expect(applyPause(list, { resume: ['#7'] })).toEqual([{ ref: '#9', by: 'Stephen' }])
    expect(applyPause(list, {})).toEqual(list)
  })
  test('pausedRefusal refuses other tools, never Irori\'s', () => {
    const paused = [{ ref: '#7', by: 'Stephen' }]
    expect(pausedRefusal([], 'Bash')).toBe('')
    expect(pausedRefusal(paused, 'mcp__irori__board_comment')).toBe('')
    const why = pausedRefusal(paused, 'Bash')
    expect(why).toContain('Stephen paused #7')
    expect(why).toContain('board_comment')
  })
  test('pausedFromBoard keeps the paused cards this session holds', () => {
    const board = {
      working: [
        { short: '7', paused: true, holders: [me] },
        { short: '8', paused: true, holders: ['dev1/other'] },
        { short: '9', holders: [me] },
      ],
      blocked: [{ short: '7', paused: true, holders: [me] }],
    }
    expect(pausedFromBoard(board, me, [{ ref: '#7', by: 'Stephen' }])).toEqual([{ ref: '#7', by: 'Stephen' }])
    expect(pausedFromBoard(board, me)).toEqual([{ ref: '#7', by: 'The person' }])
    expect(pausedFromBoard(undefined, me)).toEqual([])
  })
})

describe('sideReminder', () => {
  test('names the sides this device runs, from the board view', () => {
    const r = sideReminder({ sideLine: 'This device runs the Work side. Cards on Personal are only read here, never taken.' })
    expect(r).toBe('Irori: This device runs the Work side. Cards on Personal are only read here, never taken.')
  })

  test('says nothing when the device runs every side', () => {
    expect(sideReminder({})).toBe('')
    expect(sideReminder({ sideLine: '  ' })).toBe('')
    expect(sideReminder(undefined)).toBe('')
  })
})

describe('deviceReminder', () => {
  test('names the tool and says a failure means check, not retry', () => {
    const r = deviceReminder()
    expect(r).toContain('device_free')
    expect(r).toContain('not to retry in a loop')
  })
})

describe('Claude usage (card #599)', () => {
  test('posts only when the rate-limit windows moved', () => {
    expect(measureMovedLimits(['context', 'rateLimits'])).toBe(true)
    expect(measureMovedLimits(['context', 'cost'])).toBe(false)
    expect(measureMovedLimits(undefined)).toBe(false)
  })

  test('the account is the login record\'s ids, never the email (card #1040)', () => {
    const rec = (o: unknown) => JSON.stringify({ oauthAccount: o })
    expect(accountFromConfig(rec({ accountUuid: 'a1', organizationUuid: 'o1', emailAddress: 'x@y.z' }))).toBe('a1/o1')
    expect(accountFromConfig(rec({ accountUuid: ' a1 ' }))).toBe('a1')
    expect(accountFromConfig(rec({ accountUuid: '' }))).toBeUndefined()
    expect(accountFromConfig(rec({ organizationUuid: 'o1' }))).toBeUndefined()
    expect(accountFromConfig(rec(null))).toBeUndefined()
    expect(accountFromConfig('{}')).toBeUndefined()
    expect(accountFromConfig('junk')).toBeUndefined()
    expect(accountFromConfig(undefined)).toBeUndefined()
    expect(usageBody([{ kind: 'five_hour', percentUsed: 1 }], 'a1')).toEqual({
      rateLimits: [{ kind: 'five_hour', percentUsed: 1 }],
      account: 'a1',
    })
    expect(usageBody([{ kind: 'five_hour', percentUsed: 1 }])).toEqual({ rateLimits: [{ kind: 'five_hour', percentUsed: 1 }] })
  })

  test('the login record is in CLAUDE_CONFIG_DIR, else the home directory', () => {
    expect(claudeConfigPath('C:\\cfg\\', 'C:\\Users\\p')).toBe('C:\\cfg/.claude.json')
    expect(claudeConfigPath(undefined, '/home/p/')).toBe('/home/p/.claude.json')
    expect(claudeConfigPath('  ', '/home/p')).toBe('/home/p/.claude.json')
    expect(claudeConfigPath(undefined, undefined)).toBeNull()
  })

  test('the body keeps well-formed windows as Claude Code gives them', () => {
    expect(
      usageBody([
        { kind: 'five_hour', percentUsed: 23.5, resetsAt: '2026-11-06T03:00:00.000Z' },
        { kind: 'seven_day', percentUsed: 41.2 },
        { kind: 'spend_limit', percentUsed: 104, resetsAt: '' },
      ]),
    ).toEqual({
      rateLimits: [
        { kind: 'five_hour', percentUsed: 23.5, resetsAt: '2026-11-06T03:00:00.000Z' },
        { kind: 'seven_day', percentUsed: 41.2 },
        { kind: 'spend_limit', percentUsed: 104 },
      ],
    })
  })

  test('nothing to send is null, junk rows drop and the count is capped', () => {
    expect(usageBody([])).toBeNull()
    expect(usageBody(undefined)).toBeNull()
    expect(usageBody([null, 'x', { kind: '', percentUsed: 1 }, { kind: 'a', percentUsed: Number.NaN }, { kind: 'b' }])).toBeNull()
    const many = Array.from({ length: 12 }, (_, i) => ({ kind: `k${i}`, percentUsed: i }))
    expect(usageBody(many)?.rateLimits.length).toBe(MAX_USAGE_WINDOWS)
  })
})

describe('permission prompts (GitHub #322)', () => {
  test('a session that skips permissions is not reported', () => {
    expect(reportsPermission('bypassPermissions')).toBe(false)
    expect(reportsPermission('default')).toBe(true)
    expect(reportsPermission(undefined)).toBe(true)
  })

  test('the body names the session, tool and input', () => {
    expect(permissionBody('i1', { session_id: 's', tool_name: 'Bash', tool_input: { command: 'ls' }, permission_mode: 'plan' })).toEqual({
      id: 'i1',
      sessionId: 's',
      toolName: 'Bash',
      toolInput: { command: 'ls' },
      permissionMode: 'plan',
      permissionSuggestions: [],
    })
  })

  test('the answer is pending, nothing, or the decision inside the hook output', () => {
    expect(permissionAnswer({ decision: 'pending' })).toEqual({ kind: 'pending' })
    expect(permissionAnswer({ decision: 'none' })).toEqual({ kind: 'none' })
    expect(permissionAnswer(undefined)).toEqual({ kind: 'none' })
    expect(permissionAnswer({ decision: 'deny' })).toEqual({ kind: 'none' })
    const output = { hookSpecificOutput: { hookEventName: 'PermissionRequest', decision: { behavior: 'deny', message: 'no' } } }
    expect(permissionAnswer({ decision: 'deny', output })).toEqual({ kind: 'decide', decision: { behavior: 'deny', message: 'no' } })
  })
})
