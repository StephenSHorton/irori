// Card #765: skullet (0.0.3, behind every Loading) runs each skeleton
// bone's shimmer as an endless Web Animation (iterations: Infinity) and
// never cancels it when the bone leaves the page: a skeleton giving way to
// its content, or a redraw with fewer bones. The document timeline keeps
// every running animation, and each one keeps its removed skeleton alive,
// the sample content it measured with it, and Blink services them all on
// every frame. After 60 card opens on a copy of the PC's board the window
// held 165k detached nodes and its renderer ran a full core at rest; the
// live app on the PC grew 17 MB a minute to 6 GB over a day.
//
// sweepEndless watches the endless animations started on skullet's bones
// and cancels each one once its bone is no longer in the page, which lets
// the bone and its skeleton go. It checks every couple of seconds, and only
// while it watches something, so with no skeleton on screen nothing runs.

// Bone is the attribute skullet puts on each bone it animates.
export const Bone = "skullet-rect"
// SweepMs is how often watched animations are checked.
export const SweepMs = 2000

type Target = { isConnected: boolean; hasAttribute(name: string): boolean }
type Anim = {
  effect: { target?: Target | null } | null
  playState: string
  cancel(): void
}
type Timers = {
  set(fn: () => void, ms: number): unknown
  clear(id: unknown): void
}

// endlessSweeper is the watcher, with its timers given (tests pass fakes).
export function endlessSweeper(
  timers: Timers = {
    set: (fn, ms) => setInterval(fn, ms),
    clear: (id) => clearInterval(id as ReturnType<typeof setInterval>),
  },
  every = SweepMs
) {
  const watched = new Set<Anim>()
  let timer: unknown = null

  // sweep cancels the animations whose bone has left the page, forgets the
  // ones that ended by themselves, and stops checking once none are left.
  function sweep(): number {
    let cancelled = 0
    for (const a of watched) {
      const target = a.effect?.target
      if (a.playState === "idle" || a.playState === "finished") {
        watched.delete(a)
      } else if (!target?.isConnected) {
        a.cancel()
        watched.delete(a)
        cancelled++
      }
    }
    if (!watched.size && timer !== null) {
      timers.clear(timer)
      timer = null
    }
    return cancelled
  }

  return {
    // watch takes an animation just started on el with these options, and
    // keeps it if it is an endless one on a skullet bone.
    watch(el: Target, options: unknown, a: Anim) {
      const endless =
        typeof options === "object" &&
        options !== null &&
        (options as { iterations?: number }).iterations === Infinity
      if (!endless || !el.hasAttribute(Bone)) return
      watched.add(a)
      timer ??= timers.set(sweep, every)
    },
    sweep,
    size: () => watched.size,
  }
}

const sweeper = endlessSweeper()
let installed = false

// sweepEndless starts watching: every Element.animate from then on passes
// through it. Once per page; a no-op where the DOM has no animate.
export function sweepEndless() {
  if (installed || typeof Element === "undefined") return
  const animate = Element.prototype.animate
  if (typeof animate !== "function") return
  installed = true
  Element.prototype.animate = function (
    this: Element,
    keyframes: Keyframe[] | PropertyIndexedKeyframes | null,
    options?: number | KeyframeAnimationOptions
  ): Animation {
    const a = animate.call(this, keyframes, options)
    sweeper.watch(this, options, a as unknown as Anim)
    return a
  }
}
