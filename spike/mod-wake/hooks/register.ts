// Irori spike: prove $.process.spawn + $.prompt.submit, and see what session.append carries.
// Load it only with --plugin-dir; it logs to <os tmpdir>/irori-spike.log (the mod sandbox has no `process`).
import type { Register } from 'claude-code'

const APPEND = 'const fs=require("fs"),os=require("os"),p=require("path");fs.appendFileSync(p.join(os.tmpdir(),"irori-spike.log"),fs.readFileSync(0,"utf8"))'

// The mod loader only lets `$` be passed to functions declared at the top of the file.
async function note($: any, line: string) {
  await $.process.run(['node', '-e', APPEND], {
    stdin: JSON.stringify({ t: new Date().toISOString(), line }) + '\n',
  }).catch(() => {})
}

let woke = false
// Tells module instances apart: if two values show up, the module loaded twice.
const INSTANCE = Math.random().toString(36).slice(2, 8)

export const register: Register = on => {

  on('session.start', async ($, e, next) => {
    const started = await next(e)
    await note($, `session.start instance=${INSTANCE} session=${await $.session.id()} cwd=${(e as any).cwd}`)
    void (async () => {
      try {
        const child = $.process.spawn({
          argv: ['node', '-e', 'setTimeout(() => console.log("WAKE"), 4000); setInterval(() => {}, 1 << 30)'],
        })
        for await (const { stream, text } of child) {
          await note($, `spawn instance=${INSTANCE} ${stream}: ${text.trim()}`)
          if (stream === 'stdout' && text.includes('WAKE') && !woke) {
            woke = true
            await $.prompt.submit({ text: 'IRORI_SPIKE wake. Reply with exactly: irori-spike-pong' })
              .then(() => note($, 'prompt.submit resolved'))
              .catch((err: unknown) => note($, `prompt.submit rejected: ${String(err)}`))
          }
        }
        await note($, 'spawn loop ended')
      } catch (err) {
        await note($, `spawn failed: ${String(err)}`)
      }
    })()
    return started
  })

  on('session.append', async ($, e, next) => {
    const r = await next(e)
    const keys = Object.keys((e as any)?.message ?? e ?? {})
    await note($, `session.append keys=${keys.join(',')} json=${JSON.stringify(e).slice(0, 300)}`)
    return r
  })

  on('turn.start', async ($, e, next) => {
    const r = await next(e)
    await note($, `turn.start text=${String((e as any).text ?? '').slice(0, 80)}`)
    return r
  })

  on('turn.complete', async ($, e, next) => {
    const r = await next(e)
    await note($, `turn.complete`)
    return r
  })
}
