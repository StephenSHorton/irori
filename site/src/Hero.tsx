import { Component, lazy, Suspense, useEffect, useState, type ReactNode, type RefObject, useRef } from 'react'

const HeroScene = lazy(() => import('./scene/HeroScene'))

const SOFTWARE_GL = /swiftshader|llvmpipe|softpipe|software|basic render|microsoft basic/i

/**
 * True only for a hardware WebGL context. Software rasterisers (SwiftShader,
 * llvmpipe, the Microsoft Basic Render Driver in VMs and RDP sessions) can
 * create a context but run the scene at a few frames a second, so they get
 * the static picture instead.
 */
function hasFastWebGL() {
  try {
    const c = document.createElement('canvas')
    const opts = { failIfMajorPerformanceCaveat: true }
    const gl = (c.getContext('webgl2', opts) ?? c.getContext('webgl', opts)) as WebGLRenderingContext | null
    if (!gl) return false
    const info = gl.getExtension('WEBGL_debug_renderer_info')
    const renderer = String(gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER) ?? '')
    gl.getExtension('WEBGL_lose_context')?.loseContext()
    return !SOFTWARE_GL.test(renderer)
  } catch {
    return false
  }
}

/** Whether the element is on screen, so the render loop can stop when it is not. */
function useOnScreen(ref: RefObject<HTMLElement | null>) {
  const [on, setOn] = useState(true)
  useEffect(() => {
    const el = ref.current
    if (!el || typeof IntersectionObserver === 'undefined') return
    const io = new IntersectionObserver(([e]) => setOn(e.isIntersecting), { rootMargin: '80px 0px' })
    io.observe(el)
    return () => io.disconnect()
  }, [ref])
  return on
}

function useReducedMotion() {
  const query = '(prefers-reduced-motion: reduce)'
  const [reduced, setReduced] = useState(() => typeof window !== 'undefined' && window.matchMedia(query).matches)
  useEffect(() => {
    const mq = window.matchMedia(query)
    const on = () => setReduced(mq.matches)
    mq.addEventListener('change', on)
    return () => mq.removeEventListener('change', on)
  }, [])
  return reduced
}

/** A flat CSS picture of the board: shown while the scene loads and when WebGL is missing. */
export function StaticHero() {
  const chips = [
    [8, 10],
    [44, 4],
    [79, 10],
    [79, 77],
    [44, 83],
    [8, 77],
  ]
  return (
    <div className="hero-static" aria-hidden>
      <div className="board">
        <div className="pit" />
        {chips.map(([l, t]) => (
          <div key={`${l}-${t}`} className="chip" style={{ left: `${l}%`, top: `${t}%` }} />
        ))}
      </div>
    </div>
  )
}

class SceneBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() {
    return { failed: true }
  }
  render() {
    return this.state.failed ? <StaticHero /> : this.props.children
  }
}

export function Hero() {
  const reduced = useReducedMotion()
  const [webgl, setWebgl] = useState<boolean | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  const onScreen = useOnScreen(ref)
  useEffect(() => setWebgl(hasFastWebGL()), [])

  return (
    <div ref={ref} className="absolute inset-0">
      <StaticHero />
      {webgl ? (
        <SceneBoundary>
          <Suspense fallback={null}>
            <div className="absolute inset-0">
              <HeroScene still={reduced} paused={!onScreen} />
            </div>
          </Suspense>
        </SceneBoundary>
      ) : null}
    </div>
  )
}
