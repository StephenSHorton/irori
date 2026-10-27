import { Html, OrthographicCamera, PerformanceMonitor } from '@react-three/drei'
import { useFrame, useThree } from '@react-three/fiber'
import { Lights, Matte, PostFX, RoundCyl, SceneCanvas, SoftBox } from 'mokei/clay'
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  AdditiveBlending,
  BoxGeometry,
  CanvasTexture,
  Color,
  CylinderGeometry,
  ExtrudeGeometry,
  MeshBasicMaterial,
  MeshLambertMaterial,
  Path,
  Shape,
  SphereGeometry,
  SpriteMaterial,
  SRGBColorSpace,
  TubeGeometry,
  type Group,
  type Mesh,
  type OrthographicCamera as OrthoCam,
  type PointLight,
  type Sprite,
} from 'three'
import {
  arcCurve,
  BOARD_A_HALF,
  BOARD_B,
  LOOP,
  MESSAGES,
  NODES,
  PIT,
  PULSE_SPEED,
  ROUTES,
  STILL_T,
  STORY,
  TOP,
  TRACE_Y,
  routeCurve,
  storyCard,
  type NodeId,
  type P2,
} from './layout'

const DEG = Math.PI / 180
const TRACE = '#b3c4f2'
const PAD = '#9fb3ee'
const COOL = new Color('#3b82f6')
const WARM = new Color('#ff8a3d')
const LED_WORK = new Color('#34c77b')
const LED_BLOCKED = new Color('#f2b705')
const LED_IDLE = new Color('#cbd5e1')

/** Which agent sits on each chip. The chip top is tinted by kind. */
const KIND: Record<Exclude<NodeId, 'hearth'>, 'claude' | 'grok'> = {
  n0: 'claude',
  n1: 'grok',
  n2: 'claude',
  n3: 'grok',
  n4: 'claude',
  n5: 'claude',
  m0: 'grok',
  m1: 'claude',
}
const KIND_TOP = { claude: '#e2825a', grok: '#4a5468' }

/* ------------------------------------------------------------------ */

/** Stable DOM home for the Html cards, so drei never re-parents them mid-mount. */
const overlay: { current: HTMLDivElement } = { current: null as unknown as HTMLDivElement }

/** Frame rate (averaged over a few seconds) below which the scene stops animating. */
const MIN_FPS = 24

export default function HeroScene({ still, paused }: { still: boolean; paused: boolean }) {
  // A slow GPU that passed the WebGL check still gets a sharp picture, just not a moving one.
  const [slow, setSlow] = useState(false)
  const frozen = still || slow
  const frameloop = paused ? 'never' : frozen ? 'demand' : 'always'
  return (
    <div className="absolute inset-0" aria-hidden>
      <SceneCanvas quality="medium" frameloop={frameloop} style={{ position: 'absolute', inset: 0 }}>
        <PerformanceMonitor bounds={() => [MIN_FPS, 10_000]} flipflops={1} onDecline={() => setSlow(true)}>
          <World still={frozen} />
        </PerformanceMonitor>
        <Kick frameloop={frameloop} />
      </SceneCanvas>
      <div
        ref={(el) => {
          if (el) overlay.current = el
        }}
        className="pointer-events-none absolute inset-0 overflow-hidden"
      />
    </div>
  )
}

/**
 * Outside the always-on loop, a resize or a resume from pause renders a single
 * frame, and the Html cards read the camera before Rig moves it. Render twice.
 */
function Kick({ frameloop }: { frameloop: string }) {
  const invalidate = useThree((s) => s.invalidate)
  const size = useThree((s) => s.size)
  useEffect(() => {
    if (frameloop !== 'demand') return
    invalidate()
    const id = window.setTimeout(() => invalidate(), 120)
    return () => window.clearTimeout(id)
  }, [frameloop, size.width, size.height, invalidate])
  return null
}

/** Loop time. Reduced motion pins it to one frame that tells the story. */
function useLoopTime(still: boolean) {
  return (elapsed: number) => (still ? STILL_T : elapsed % LOOP)
}

function World({ still }: { still: boolean }) {
  const loopT = useLoopTime(still)
  const curves = useMemo(() => {
    const map = new Map<string, { curve: ReturnType<typeof routeCurve>; length: number }>()
    for (const r of ROUTES) {
      const curve = routeCurve(r)
      map.set(r.id, { curve, length: curve.getLength() })
    }
    return map
  }, [])
  const answerArrives = STORY.answerT0 + curves.get(STORY.answerRoute)!.length / PULSE_SPEED
  const invalidate = useThree((s) => s.invalidate)
  useLayoutEffect(() => {
    if (still) {
      // Html cards position themselves after the first frame; render a couple.
      const id = window.setTimeout(() => invalidate(), 120)
      return () => window.clearTimeout(id)
    }
  }, [still, invalidate])

  return (
    <>
      <Rig still={still} />
      <Lights />
      <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, -0.001, 0]} receiveShadow>
        <planeGeometry args={[400, 400]} />
        <meshLambertMaterial color="#e9eef8" />
      </mesh>
      <BoardA />
      <SoftBox
        size={[BOARD_B.half * 2, TOP, BOARD_B.half * 2]}
        r={0.12}
        material="base"
        position={[BOARD_B.center[0], TOP / 2, BOARD_B.center[1]]}
      />
      <Traces />
      <Hearth loopT={loopT} />
      <Nodes loopT={loopT} curves={curves} answerArrives={answerArrives} />
      <Pulses loopT={loopT} curves={curves} />
      <Cards loopT={loopT} answerArrives={answerArrives} />
      <Html portal={overlay} position={[-BOARD_A_HALF + 1.1, TOP, BOARD_A_HALF - 0.2]} center zIndexRange={[10, 0]}>
        <div className="scene-tag wide-only">laptop</div>
      </Html>
      <Html portal={overlay}
        position={[BOARD_B.center[0] + 1.6, TOP, BOARD_B.center[1] + BOARD_B.half - 0.2]}
        center
        zIndexRange={[10, 0]}
      >
        <div className="scene-tag wide-only">desk-pc</div>
      </Html>
      <PostFX />
    </>
  )
}

/* ------------------------------------------------------------------ */
/* Camera: locked orthographic view with a slow drift.                 */
/* ------------------------------------------------------------------ */

function Rig({ still }: { still: boolean }) {
  const ref = useRef<OrthoCam>(null)
  const size = useThree((s) => s.size)
  useFrame(({ clock }) => {
    const cam = ref.current
    if (!cam) return
    const t = still ? 0 : clock.elapsedTime
    const wide = size.width / size.height > 1.2
    // Narrow screens frame the laptop board plus the cable, so the answer pulse stays in view.
    const tx = wide ? 3.9 : 2.0
    const tz = wide ? -2.4 : -1.5
    const az = (34 + Math.sin(t * 0.07) * 6) * DEG
    const el = (41 + Math.sin(t * 0.045) * 2.5) * DEG
    const dist = 60
    cam.position.set(tx + dist * Math.cos(el) * Math.sin(az), dist * Math.sin(el), tz + dist * Math.cos(el) * Math.cos(az))
    cam.lookAt(tx, 0, tz)
    const spanW = wide ? 27 : 15.5
    const spanH = wide ? 13.8 : 13.5
    cam.zoom = Math.min(size.width / spanW, size.height / spanH)
    cam.updateProjectionMatrix()
  })
  return <OrthographicCamera ref={ref} makeDefault near={-100} far={300} position={[40, 36, 40]} zoom={30} />
}

/* ------------------------------------------------------------------ */
/* Boards and traces.                                                  */
/* ------------------------------------------------------------------ */

function roundedRect(shape: Shape | Path, half: number, r: number) {
  shape.moveTo(-half + r, -half)
  shape.lineTo(half - r, -half)
  shape.quadraticCurveTo(half, -half, half, -half + r)
  shape.lineTo(half, half - r)
  shape.quadraticCurveTo(half, half, half - r, half)
  shape.lineTo(-half + r, half)
  shape.quadraticCurveTo(-half, half, -half, half - r)
  shape.lineTo(-half, -half + r)
  shape.quadraticCurveTo(-half, -half, -half + r, -half)
}

/** The laptop's board: one rounded slab with a square pit cut for the hearth. */
function BoardA() {
  const geo = useMemo(() => {
    const shape = new Shape()
    roundedRect(shape, BOARD_A_HALF - 0.1, 0.5)
    const hole = new Path()
    roundedRect(hole, PIT + 0.1, 0.12)
    shape.holes.push(hole)
    const bevel = 0.08
    return new ExtrudeGeometry(shape, {
      depth: TOP - bevel * 2,
      bevelEnabled: true,
      bevelThickness: bevel,
      bevelSize: 0.08,
      bevelSegments: 3,
      curveSegments: 6,
    })
  }, [])
  return (
    <mesh geometry={geo} rotation={[-Math.PI / 2, 0, 0]} position={[0, 0.08, 0]} castShadow receiveShadow>
      <Matte material="base" />
    </mesh>
  )
}

const unitBox = new BoxGeometry(1, 1, 1)
const padGeo = new CylinderGeometry(0.15, 0.15, 0.04, 16)

function segments(pts: P2[]) {
  const out: { pos: [number, number, number]; scale: [number, number, number] }[] = []
  const w = 0.13
  for (let i = 0; i < pts.length - 1; i += 1) {
    const [ax, az] = pts[i]
    const [bx, bz] = pts[i + 1]
    out.push({
      pos: [(ax + bx) / 2, TRACE_Y, (az + bz) / 2],
      scale: [Math.abs(bx - ax) + w, 0.03, Math.abs(bz - az) + w],
    })
  }
  return out
}

function Traces() {
  const data = useMemo(() => {
    const segs: ReturnType<typeof segments> = []
    const pads: P2[] = []
    const tubes: TubeGeometry[] = []
    for (const r of ROUTES) {
      for (const line of r.lines) {
        segs.push(...segments(line))
        for (const p of line.slice(1, -1)) pads.push(p)
      }
      if (r.arc) {
        tubes.push(new TubeGeometry(arcCurve(r.arc), 40, 0.07, 8))
        pads.push(r.arc.from, r.arc.to)
      }
    }
    return { segs, pads, tubes }
  }, [])
  const traceMat = useMemo(() => new MeshLambertMaterial({ color: TRACE }), [])
  const padMat = useMemo(() => new MeshLambertMaterial({ color: PAD }), [])
  return (
    <group>
      {data.segs.map((s, i) => (
        <mesh key={i} geometry={unitBox} material={traceMat} position={s.pos} scale={s.scale} receiveShadow />
      ))}
      {data.pads.map((p, i) => (
        <mesh key={`p${i}`} geometry={padGeo} material={padMat} position={[p[0], TRACE_Y + 0.01, p[1]]} />
      ))}
      {data.tubes.map((g, i) => (
        <mesh key={`t${i}`} geometry={g} material={traceMat} castShadow />
      ))}
    </group>
  )
}

/* ------------------------------------------------------------------ */
/* The hearth: a sunken square with embers and a warm glow.            */
/* ------------------------------------------------------------------ */

let glowTexture: CanvasTexture | null = null
function getGlowTexture() {
  if (glowTexture) return glowTexture
  const c = document.createElement('canvas')
  c.width = c.height = 128
  const g = c.getContext('2d')!
  const grad = g.createRadialGradient(64, 64, 0, 64, 64, 64)
  grad.addColorStop(0, 'rgba(255,255,255,1)')
  grad.addColorStop(0.3, 'rgba(255,255,255,0.45)')
  grad.addColorStop(1, 'rgba(255,255,255,0)')
  g.fillStyle = grad
  g.fillRect(0, 0, 128, 128)
  glowTexture = new CanvasTexture(c)
  glowTexture.colorSpace = SRGBColorSpace
  return glowTexture
}

const COALS: [number, number, number][] = [
  [-0.45, -0.3, 0.34],
  [0.3, -0.42, 0.3],
  [0.05, 0.1, 0.4],
  [-0.35, 0.42, 0.28],
  [0.48, 0.3, 0.32],
  [-0.7, 0.02, 0.24],
  [0.72, -0.08, 0.22],
  [0.1, 0.7, 0.24],
  [-0.1, -0.72, 0.24],
]

function Hearth({ loopT }: { loopT: (e: number) => number }) {
  const light = useRef<PointLight>(null)
  const glow = useRef<Sprite>(null)
  const sparks = useRef<(Mesh | null)[]>([])
  const coalMat = useMemo(
    () => new MeshLambertMaterial({ color: '#8a3414', emissive: '#ff5a14', emissiveIntensity: 0.9 }),
    [],
  )
  const coalMat2 = useMemo(
    () => new MeshLambertMaterial({ color: '#5a2410', emissive: '#ff7a2f', emissiveIntensity: 0.6 }),
    [],
  )
  const sparkMat = useMemo(() => new MeshBasicMaterial({ color: '#ffb066', toneMapped: false }), [])
  const sparkGeo = useMemo(() => new SphereGeometry(0.045, 8, 6), [])
  const warmArrivals = useMemo(() => {
    // times at which a warm pulse reaches the hearth
    return MESSAGES.filter((m) => m.warm).map((m) => {
      const r = ROUTES.find((x) => x.id === m.route)!
      return m.t0 + routeCurve(r).getLength() / PULSE_SPEED
    })
  }, [])

  useFrame(({ clock }) => {
    const e = clock.elapsedTime
    const t = loopT(e)
    let flash = 0
    for (const ta of warmArrivals) {
      const d = t - ta
      if (d >= 0 && d < 0.8) flash = Math.max(flash, 1 - d / 0.8)
    }
    const flicker = 0.85 + Math.sin(e * 7.3) * 0.06 + Math.sin(e * 13.1) * 0.04
    coalMat.emissiveIntensity = 0.8 * flicker + flash * 0.5
    coalMat2.emissiveIntensity = 0.55 * (2 - flicker) + flash * 0.3
    if (light.current) light.current.intensity = (7 + flash * 5) * flicker
    if (glow.current) {
      const s = 4.6 + flash * 0.9 + Math.sin(e * 2.1) * 0.12
      glow.current.scale.set(s, s, 1)
      ;(glow.current.material as SpriteMaterial).opacity = 0.5 + flash * 0.25
    }
    sparks.current.forEach((m, i) => {
      if (!m) return
      const k = (e * 0.32 + i / sparks.current.length) % 1
      const a = i * 2.4
      m.position.set(Math.cos(a) * 0.5 * (1 - k * 0.4), TOP - 0.1 + k * 2.2, Math.sin(a) * 0.5 * (1 - k * 0.4))
      const sc = Math.sin(k * Math.PI)
      m.scale.setScalar(sc)
    })
  })

  const frameW = 0.34
  const fh = 0.12
  const outer = PIT + frameW
  return (
    <group>
      {/* wooden frame on the board around the pit */}
      <SoftBox size={[outer * 2, fh, frameW]} r={0.05} unsafeColor="#d8a76e" position={[0, TOP + fh / 2, -PIT - frameW / 2]} />
      <SoftBox size={[outer * 2, fh, frameW]} r={0.05} unsafeColor="#d8a76e" position={[0, TOP + fh / 2, PIT + frameW / 2]} />
      <SoftBox size={[frameW, fh, PIT * 2]} r={0.05} unsafeColor="#d8a76e" position={[-PIT - frameW / 2, TOP + fh / 2, 0]} />
      <SoftBox size={[frameW, fh, PIT * 2]} r={0.05} unsafeColor="#d8a76e" position={[PIT + frameW / 2, TOP + fh / 2, 0]} />
      {/* ash bed, sunk below the board */}
      <SoftBox size={[PIT * 2, 0.12, PIT * 2]} r={0.04} unsafeColor="#e6ddd2" position={[0, 0.12, 0]} cast={false} />
      {COALS.map(([x, z, s], i) => (
        <mesh
          key={i}
          geometry={unitBox}
          material={i % 2 ? coalMat2 : coalMat}
          position={[x, 0.18 + s * 0.3, z]}
          rotation={[0.3 * i, 0.7 * i, 0.2 * i]}
          scale={[s, s * 0.7, s * 0.9]}
        />
      ))}
      {/* a small iron kettle on the coals */}
      <RoundCyl radius={0.42} height={0.42} fillet={0.16} material="detail.dark" position={[0.05, 0.5, 0.05]} />
      <RoundCyl radius={0.12} height={0.08} fillet={0.03} material="detail.mid" position={[0.05, 0.74, 0.05]} cast={false} />
      <pointLight ref={light} color="#ff8a3d" intensity={7} distance={6.5} decay={1.6} position={[0, TOP + 0.5, 0]} />
      <sprite ref={glow} position={[0, TOP + 0.25, 0]} scale={[4.6, 4.6, 1]} renderOrder={2}>
        <spriteMaterial
          map={getGlowTexture()}
          color="#ff8a3d"
          transparent
          opacity={0.5}
          depthWrite={false}
          blending={AdditiveBlending}
          toneMapped={false}
        />
      </sprite>
      {Array.from({ length: 7 }, (_, i) => (
        <mesh key={`s${i}`} ref={(m) => void (sparks.current[i] = m)} geometry={sparkGeo} material={sparkMat} />
      ))}
    </group>
  )
}

/* ------------------------------------------------------------------ */
/* Agents: chips on the board with a status light.                     */
/* ------------------------------------------------------------------ */

type Curves = Map<string, { curve: ReturnType<typeof routeCurve>; length: number }>

function Nodes({
  loopT,
  curves,
  answerArrives,
}: {
  loopT: (e: number) => number
  curves: Curves
  answerArrives: number
}) {
  const leds = useRef(new Map<string, MeshBasicMaterial>())
  const glows = useRef(new Map<string, Sprite>())
  const glowMats = useMemo(
    () =>
      new Map(
        NODES.map((n) => [
          n.id,
          new SpriteMaterial({
            map: getGlowTexture(),
            color: LED_WORK.clone(),
            transparent: true,
            opacity: 0.55,
            depthWrite: false,
            blending: AdditiveBlending,
            toneMapped: false,
          }),
        ]),
      ),
    [],
  )
  const ledFor = (id: string) => {
    let m = leds.current.get(id)
    if (!m) {
      m = new MeshBasicMaterial({ color: LED_WORK.clone(), toneMapped: false })
      leds.current.set(id, m)
    }
    return m
  }
  const arrivals = useMemo(
    () =>
      MESSAGES.map((m) => {
        const r = ROUTES.find((x) => x.id === m.route)!
        return { node: (m.reverse ? r.a : r.b) as NodeId, at: m.t0 + curves.get(r.id)!.length / PULSE_SPEED }
      }),
    [curves],
  )
  const pinMat = useMemo(() => new MeshLambertMaterial({ color: '#9aa6bf' }), [])
  const tmp = useMemo(() => new Color(), [])

  useFrame(({ clock }) => {
    const t = loopT(clock.elapsedTime)
    const story = storyCard(t, answerArrives)
    for (const n of NODES) {
      const m = ledFor(n.id)
      let base = n.id === 'm1' || n.id === 'n0' ? LED_IDLE : LED_WORK
      if (n.id === 'n2' && story.state === 'blocked') base = LED_BLOCKED
      let flash = 0
      for (const a of arrivals) {
        const d = t - a.at
        if (a.node === n.id && d >= 0 && d < 0.6) flash = Math.max(flash, 1 - d / 0.6)
      }
      tmp.copy(base).lerp(COOL, flash * 0.8)
      m.color.copy(tmp)
      const blocked = base === LED_BLOCKED
      const g = glows.current.get(n.id)
      const gm = glowMats.get(n.id)!
      gm.color.copy(tmp)
      gm.opacity = blocked ? 0.95 : 0.4 + flash * 0.5
      if (g) {
        const sc = blocked ? 1.7 + Math.sin(t * 6) * 0.25 : 0.7 + flash * 0.5
        g.scale.set(sc, sc, 1)
      }
    }
  })

  return (
    <group>
      {NODES.map((n) => (
        <group key={n.id} position={[n.pos[0], TOP, n.pos[1]]}>
          <SoftBox size={[1.36, 0.34, 1.36]} r={0.12} material="base" position={[0, 0.17, 0]} />
          <SoftBox
            size={[0.86, 0.08, 0.86]}
            r={0.06}
            unsafeColor={KIND_TOP[KIND[n.id]]}
            position={[0, 0.36, 0]}
            cast={false}
          />
          <mesh position={[0.48, 0.4, 0.48]} material={ledFor(n.id)}>
            <sphereGeometry args={[0.12, 12, 8]} />
          </mesh>
          <sprite
            ref={(g) => void (g ? glows.current.set(n.id, g) : glows.current.delete(n.id))}
            position={[0.48, 0.46, 0.48]}
            material={glowMats.get(n.id)}
            renderOrder={3}
          />
          <Html portal={overlay} position={[-0.2, 0, 1.05]} center zIndexRange={[5, 0]}>
            <div className={`scene-kind scene-kind-${KIND[n.id]} wide-only`}>{KIND[n.id]}</div>
          </Html>
          {[-0.4, 0, 0.4].map((o) => (
            <group key={o}>
              <mesh geometry={unitBox} material={pinMat} position={[o, 0.06, 0.74]} scale={[0.14, 0.08, 0.16]} />
              <mesh geometry={unitBox} material={pinMat} position={[o, 0.06, -0.74]} scale={[0.14, 0.08, 0.16]} />
            </group>
          ))}
        </group>
      ))}
    </group>
  )
}

/* ------------------------------------------------------------------ */
/* Pulses: messages travelling along traces.                           */
/* ------------------------------------------------------------------ */

/** Distances (world units) behind the head for the pulse trail. */
const TRAIL = [0.32, 0.62, 0.9]

function Pulses({ loopT, curves }: { loopT: (e: number) => number; curves: Curves }) {
  const groups = useRef<(Group | null)[]>([])
  const geo = useMemo(() => new SphereGeometry(0.22, 14, 10), [])
  const mats = useMemo(
    () => MESSAGES.map((m) => new MeshBasicMaterial({ color: m.warm ? WARM : COOL, toneMapped: false })),
    [],
  )
  const trailMats = useMemo(
    () =>
      MESSAGES.map((m) =>
        TRAIL.map(
          (_, k) =>
            new MeshBasicMaterial({
              color: m.warm ? WARM : COOL,
              transparent: true,
              opacity: 0.55 - k * 0.15,
              depthWrite: false,
              toneMapped: false,
            }),
        ),
      ),
    [],
  )
  const haloMats = useMemo(
    () =>
      MESSAGES.map(
        (m) =>
          new SpriteMaterial({
            map: getGlowTexture(),
            color: m.warm ? WARM : COOL,
            transparent: true,
            opacity: 0.7,
            depthWrite: false,
            blending: AdditiveBlending,
            toneMapped: false,
          }),
      ),
    [],
  )

  useFrame(({ clock }) => {
    const t = loopT(clock.elapsedTime)
    MESSAGES.forEach((m, i) => {
      const g = groups.current[i]
      if (!g) return
      const c = curves.get(m.route)!
      const u = ((t - m.t0) * PULSE_SPEED) / c.length
      if (u < 0 || u > 1) {
        g.visible = false
        return
      }
      g.visible = true
      const p = c.curve.getPointAt(m.reverse ? 1 - u : u)
      g.position.set(p.x, p.y + 0.06, p.z)
      // the trail sits behind the head, in world space relative to the group
      TRAIL.forEach((back, k) => {
        const tm = g.children[2 + k]
        if (!tm) return
        const ut = Math.max(0, u - back / c.length)
        const q = c.curve.getPointAt(m.reverse ? 1 - ut : ut)
        tm.position.set(q.x - p.x, q.y - p.y, q.z - p.z)
      })
    })
  })

  return (
    <group>
      {MESSAGES.map((_, i) => (
        <group key={i} ref={(g) => void (groups.current[i] = g)} visible={false}>
          <mesh geometry={geo} material={mats[i]} />
          <sprite scale={[1.4, 1.4, 1]} material={haloMats[i]} renderOrder={3} />
          {TRAIL.map((_, k) => (
            <mesh key={k} geometry={geo} material={trailMats[i][k]} scale={0.75 - k * 0.2} />
          ))}
        </group>
      ))}
    </group>
  )
}

/* ------------------------------------------------------------------ */
/* Glass cards over some agents.                                       */
/* ------------------------------------------------------------------ */

type CardDef = {
  node: Exclude<NodeId, 'hearth'>
  title: string
  wideOnly?: boolean
  /** Ambient cards: progress loops from..to over the loop. */
  from?: number
  to?: number
  status?: string
}

const CARDS: CardDef[] = [
  { node: 'n2', title: 'Migrate inbox schema' },
  { node: 'n5', title: 'Fix flaky capture test', from: 0.3, to: 0.85, status: 'claude · step 3 of 5', wideOnly: true },
  { node: 'n1', title: 'Review PR #212', from: 0.1, to: 0.6, status: 'grok · reviewing', wideOnly: true },
  { node: 'm1', title: 'Release notes', from: 0.55, to: 0.95, status: 'claude · desk-pc', wideOnly: true },
]

const STATUS = {
  working: 'claude · step 2 of 4',
  blocked: 'Blocked: needs a decision',
  answered: 'Answered from desk-pc',
  done: 'Done',
}

function Cards({ loopT, answerArrives }: { loopT: (e: number) => number; answerArrives: number }) {
  const roots = useRef<(HTMLDivElement | null)[]>([])
  const last = useRef<string[]>([])
  useFrame(({ clock }) => {
    const t = loopT(clock.elapsedTime)
    CARDS.forEach((c, i) => {
      const el = roots.current[i]
      if (!el) return
      const fill = el.querySelector<HTMLDivElement>('.fill')
      const status = el.querySelector<HTMLDivElement>('.status')
      let progress: number
      let state: keyof typeof STATUS = 'working'
      if (c.node === 'n2') {
        const s = storyCard(t, answerArrives)
        progress = s.progress
        state = s.state
      } else {
        const k = t / LOOP
        progress = (c.from ?? 0) + ((c.to ?? 1) - (c.from ?? 0)) * k
      }
      if (fill) fill.style.width = `${Math.round(progress * 1000) / 10}%`
      const text = c.node === 'n2' ? STATUS[state] : (c.status ?? '')
      if (last.current[i] !== state + text) {
        last.current[i] = state + text
        el.dataset.state = state
        if (status) status.textContent = text
      }
    })
  })

  return (
    <>
      {CARDS.map((c, i) => {
        const pos = NODES.find((n) => n.id === c.node)!.pos
        return (
          <Html portal={overlay} key={c.node} position={[pos[0], TOP + 1.7, pos[1]]} center zIndexRange={[20, 10]}>
            <div ref={(el) => void (roots.current[i] = el)} className={`scene-card${c.wideOnly ? ' wide-only' : ''}`}>
              <div className="title">{c.title}</div>
              <div className="status">{c.status ?? STATUS.working}</div>
              <div className="bar">
                <div className="fill" />
              </div>
            </div>
          </Html>
        )
      })}
    </>
  )
}
