import { Component, lazy, Suspense, useEffect, useState, type ReactNode } from 'react'

const HeroScene = lazy(() => import('./scene/HeroScene'))

function hasWebGL() {
  try {
    const c = document.createElement('canvas')
    return Boolean(c.getContext('webgl2') ?? c.getContext('webgl'))
  } catch {
    return false
  }
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
  useEffect(() => setWebgl(hasWebGL()), [])

  return (
    <div className="absolute inset-0">
      <StaticHero />
      {webgl ? (
        <SceneBoundary>
          <Suspense fallback={null}>
            <div className="absolute inset-0">
              <HeroScene still={reduced} />
            </div>
          </Suspense>
        </SceneBoundary>
      ) : null}
    </div>
  )
}
