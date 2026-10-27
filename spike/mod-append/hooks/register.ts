// Irori spike 04: next-turn delivery with $.session.append. After the first
// turn completes it appends a note (no turn should start), waits, then wakes
// the session and asks whether the model saw the note. Logs to
// <os tmpdir>/irori-append-spike.log. Load only with --plugin-dir.
import type { Register } from 'claude-code'

const APPEND = 'const fs=require("fs"),os=require("os"),p=require("path");fs.appendFileSync(p.join(os.tmpdir(),"irori-append-spike.log"),fs.readFileSync(0,"utf8"))'

async function note($: any, line: string) {
  await $.process.run(['node', '-e', APPEND], {
    stdin: JSON.stringify({ t: new Date().toISOString(), line }) + '\n',
  }).catch(() => {})
}

let step = 0

async function appendThenAsk($: any) {
  try {
    const r = await $.session.append({
      message: { type: 'user', content: [{ type: 'text', text: '<irori-note from="you" card="t1">The secret word is PELICAN-42. No reply needed.</irori-note>' }] },
    })
    await note($, `append result=${JSON.stringify(r).slice(0, 300)}`)
  } catch (err) {
    await note($, `append threw: ${String(err)}`)
  }
  await new Promise(res => setTimeout(res, 8000))
  await note($, 'asking')
  await $.prompt.submit({ text: 'IRORI_APPEND_SPIKE: without using any tools, what secret word, if any, reached you in an Irori note since your last reply? Reply with just the word, or NONE.' })
    .then(() => note($, 'submit resolved'))
    .catch((err: unknown) => note($, `submit rejected: ${String(err)}`))
}

export const register: Register = on => {
  on('turn.start', async ($, e, next) => {
    const r = await next(e)
    await note($, `turn.start step=${step} text=${String((e as any).text ?? '').slice(0, 80)}`)
    return r
  })
  on('turn.complete', async ($, e, next) => {
    const r = await next(e)
    step++
    await note($, `turn.complete step=${step}`)
    if (step === 1) void appendThenAsk($)
    return r
  })
}
