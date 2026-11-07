import { useEffect, useRef, useState } from "react"
import { cn } from "cn"

import { meterColor, MetersPanel } from "@/components/meters/meters-panel"
import { Sparkline } from "@/components/meters/sparkline"
import { Popover, PopoverTrigger } from "@/components/ui/popover"
import {
  diskByName,
  diskName,
  fmtBits,
  fmtPct,
  gpuByName,
  memPct,
  metersHere,
  metersOpenAtStart,
  pickDisk,
  pickGpu,
  RING,
  useMeters,
  type MetersSample,
} from "@/lib/meters"
import { stripLine, useNow, useUsage } from "@/lib/usage"

// MetersStrip is the top bar's hardware meters (card #599): CPU, memory,
// GPU, network and disk, each with its last two minutes, and how much of
// Claude's plan is left. Desktop only: hidden under the phone width and
// absent on the phone. More meters show as the header widens, so it never
// crowds the title or the actions. A click opens the panel.
export function MetersStrip({ crowded = false }: { crowded?: boolean }) {
  if (!metersHere()) return null
  return <Strip fit={crowded ? FIT_CROWDED : FIT} />
}

// What shows at which header width (AppHeader's @container/header, so the
// sidebar, open or folded, is already taken off): CPU and memory first,
// then Claude, the GPU, then network and disk. A board's page (crowded)
// has the board's name, line and actions in the header, so each needs
// more room there.
type Fit = { strip: string; claude: string; gpu: string; more: string }
const FIT: Fit = {
  strip: "@max-[500px]/header:hidden!",
  claude: "hidden @min-[600px]/header:flex",
  gpu: "hidden @min-[760px]/header:flex",
  more: "hidden @min-[1000px]/header:flex",
}
const FIT_CROWDED: Fit = {
  strip: "@max-[880px]/header:hidden!",
  claude: "hidden @min-[1200px]/header:flex",
  gpu: "hidden @min-[1300px]/header:flex",
  more: "hidden @min-[1560px]/header:flex",
}

const net = (s: MetersSample) => s.net.rxBitsPerS + s.net.txBitsPerS

function Strip({ fit }: { fit: Fit }) {
  const samples = useMeters()
  const usage = useUsage()
  const now = useNow()
  const last = samples.at(-1)
  // Picked over the whole window, so the meter stays on one device.
  const gpu = pickGpu(samples)
  const disk = pickDisk(samples)
  const claude = stripLine(usage, now)

  // The panel closes when its trigger is hidden (the header got too
  // narrow), so it is never left pinned to nothing.
  const [open, setOpen] = useState(metersOpenAtStart)
  const trigger = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    const el = trigger.current
    if (!open || !el) return
    const check = () => {
      if (el.getClientRects().length === 0) setOpen(false)
    }
    const ro = new ResizeObserver(check)
    ro.observe(el)
    window.addEventListener("resize", check)
    return () => {
      ro.disconnect()
      window.removeEventListener("resize", check)
    }
  }, [open])

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        ref={trigger}
        aria-label="Hardware meters and Claude usage left"
        className={cn(
          "mr-1 hidden h-8 shrink-0 items-center gap-3 rounded-[min(var(--radius-md),12px)] px-2 text-xs text-muted-foreground tabular-nums outline-none hover:bg-accent hover:text-accent-foreground focus-visible:ring-1 focus-visible:ring-ring aria-expanded:bg-accent aria-expanded:text-accent-foreground md:flex",
          fit.strip
        )}
      >
        <Item
          label="CPU"
          color={meterColor.cpu}
          values={samples.map((s) => s.cpu.pct)}
          value={fmtPct(last?.cpu.pct)}
        />
        <Item
          label="Mem"
          color={meterColor.mem}
          values={samples.map((s) => memPct(s.mem))}
          value={fmtPct(last ? memPct(last.mem) : null)}
        />
        {gpu != null && (
          <Item
            className={fit.gpu}
            label="GPU"
            title={gpu}
            color={meterColor.gpu}
            values={samples.map((s) => gpuByName(s, gpu)?.utilPct ?? null)}
            value={fmtPct(last && gpuByName(last, gpu)?.utilPct)}
          />
        )}
        {last && (
          <Item
            className={fit.more}
            label="Net"
            color={meterColor.net}
            values={samples.map(net)}
            max="auto"
            value={fmtBits(net(last))}
            wide
          />
        )}
        {disk != null && (
          <Item
            className={fit.more}
            label="Disk"
            title={diskName(disk)}
            color={meterColor.disk}
            values={samples.map((s) => diskByName(s, disk)?.activePct ?? null)}
            value={fmtPct(last && diskByName(last, disk)?.activePct)}
          />
        )}
        {claude && (
          <span
            title={`Claude usage: ${claude.title}`}
            className={cn(
              "items-center gap-1 border-l border-border pl-3 whitespace-nowrap",
              fit.claude
            )}
          >
            <span>Claude</span>
            <span className="text-foreground/80">{claude.text}</span>
          </span>
        )}
      </PopoverTrigger>
      <MetersPanel samples={samples} usage={usage} now={now} />
    </Popover>
  )
}

// Item is one meter in the strip: its name, its tiny graph, its reading.
function Item({
  label,
  title,
  color,
  values,
  max = 100,
  value,
  wide,
  className,
}: {
  label: string
  title?: string
  color: string
  values: readonly (number | null)[]
  max?: number | "auto"
  value: string
  wide?: boolean
  className?: string
}) {
  return (
    <span
      title={title}
      className={cn("flex items-center gap-1 whitespace-nowrap", className)}
    >
      <span>{label}</span>
      <span style={{ color }} className="flex">
        <Sparkline
          values={values}
          slots={RING}
          max={max}
          width={28}
          height={12}
        />
      </span>
      <span
        className={cn(
          "text-right text-foreground/80",
          wide ? "min-w-[9ch]" : "min-w-[4ch]"
        )}
      >
        {value}
      </span>
    </span>
  )
}
