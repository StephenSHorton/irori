// Counts the window's calls to the Rust side by command, for the lists line
// in irori.log (window-log.ts, card #1142). Wrapping the bindings' `commands`
// is the one place every call goes through: Tauri defines
// `window.__TAURI_INTERNALS__.invoke` non-writable, so it can't be wrapped.
// This module imports nothing, so host.ts can use it and window-log.ts can
// read it without a cycle.

type Listener = (command: string) => void

let listener: Listener | null = null

// onIpc sends each counted call to fn, and returns the way to stop.
export function onIpc(fn: Listener): () => void {
  listener = fn
  return () => {
    if (listener === fn) listener = null
  }
}

// commandName is the Rust command a bindings function calls: boardCardsById
// is board_cards_by_id.
export function commandName(fn: string): string {
  return fn.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`)
}

// counted wraps every function on a bindings object so each call is counted
// under its command name, and passes the call through untouched.
export function counted<T extends object>(commands: T): T {
  return new Proxy(commands, {
    get(target, key, receiver) {
      const v = Reflect.get(target, key, receiver)
      if (typeof v !== "function" || typeof key !== "string") return v
      const name = commandName(key)
      return (...args: unknown[]) => {
        listener?.(name)
        return (v as (...a: unknown[]) => unknown).apply(target, args)
      }
    },
  })
}
