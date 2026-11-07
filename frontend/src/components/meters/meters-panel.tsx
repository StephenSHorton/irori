import type { CSSProperties, ReactNode } from "react"
import { cn } from "cn"

import { Sparkline } from "@/components/meters/sparkline"
import { PopoverContent, PopoverTitle } from "@/components/ui/popover"
import {
  diskByName,
  diskName,
  fmtByteRate,
  fmtCpu,
  fmtGpu,
  fmtGpuMemory,
  fmtMemory,
  fmtNet,
  fmtPct,
  gpuByName,
  memPct,
  RING,
  type MetersSample,
} from "@/lib/meters"
import {
  hasUsage,
  percentLeft,
  resetLine,
  sortWindows,
  staleLine,
  windowLabel,
  type UsageLimits,
} from "@/lib/usage"

// The meters' colours (design/irori-ds.css), Task Manager's hues.
export const meterColor = {
  cpu: "var(--meter-cpu, var(--chart-1))",
  mem: "var(--meter-mem, var(--chart-4))",
  disk: "var(--meter-disk, var(--chart-2))",
  net: "var(--meter-net, var(--destructive))",
  gpu: "var(--meter-gpu, var(--chart-4))",
} as const

// MetersPanel is what the top bar's meters open (card #599): how much of
// Claude's plan is left first, as it is what people come for, then this
// computer's last two minutes, one row per meter like Task Manager's left
// column.
export function MetersPanel({
  samples,
  usage,
  now,
}: {
  samples: readonly MetersSample[]
  usage: UsageLimits
  now: number
}) {
  const last = samples.at(-1)
  return (
    <PopoverContent
      align="end"
      className="glass-solid max-h-[min(85vh,44rem)] w-[22rem] gap-0 overflow-y-auto p-0"
    >
      <UsageSection usage={usage} now={now} />
      <PopoverTitle className="border-t border-border/70 px-3 pt-2.5 pb-1.5 text-xs font-medium text-muted-foreground">
        This computer
      </PopoverTitle>
      {!last ? (
        <p className="px-3 pb-3 text-xs text-muted-foreground">
          No reading yet.
        </p>
      ) : (
        <div className="flex flex-col pb-1.5">
          <Row
            color={meterColor.cpu}
            values={samples.map((s) => s.cpu.pct)}
            title="CPU"
            lines={[fmtCpu(last.cpu)]}
          />
          <Row
            color={meterColor.mem}
            values={samples.map((s) => memPct(s.mem))}
            title="Memory"
            lines={[fmtMemory(last.mem)]}
          />
          {last.disks.map((d) => (
            <Row
              key={`disk:${d.name}`}
              color={meterColor.disk}
              values={samples.map(
                (s) => diskByName(s, d.name)?.activePct ?? null
              )}
              title={diskName(d.name)}
              lines={[
                `${fmtPct(d.activePct)} active`,
                `R: ${fmtByteRate(d.readBytesPerS)} W: ${fmtByteRate(d.writeBytesPerS)}`,
              ]}
            />
          ))}
          <Row
            color={meterColor.net}
            values={samples.map((s) => s.net.rxBitsPerS + s.net.txBitsPerS)}
            max="auto"
            title="Network"
            lines={[fmtNet(last.net)]}
          />
          {last.gpus.map((g, i) => {
            const mem = fmtGpuMemory(g)
            return (
              <Row
                key={`gpu:${g.name}`}
                color={meterColor.gpu}
                values={samples.map(
                  (s) => gpuByName(s, g.name)?.utilPct ?? null
                )}
                title={`GPU ${i}`}
                lines={[g.name, mem ? `${fmtGpu(g)} · ${mem}` : fmtGpu(g)]}
              />
            )
          })}
        </div>
      )}
    </PopoverContent>
  )
}

// Row is one meter: its graph in a frame on the left, its name and
// readings on the right.
function Row({
  color,
  values,
  max = 100,
  title,
  lines,
}: {
  color: string
  values: readonly (number | null)[]
  max?: number | "auto"
  title: ReactNode
  lines: string[]
}) {
  return (
    <div className="flex items-center gap-3 px-3 py-1">
      <div
        className="flex h-12 w-[4.5rem] shrink-0 items-end overflow-hidden border bg-background/40"
        style={
          {
            color,
            borderColor: `color-mix(in oklch, ${color} 55%, transparent)`,
          } as CSSProperties
        }
      >
        <Sparkline
          values={values}
          slots={RING}
          max={max}
          width={72}
          height={46}
        />
      </div>
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm leading-snug font-medium text-ink">
          {title}
        </span>
        {lines.map((l, i) => (
          <span
            key={i}
            className="truncate text-xs leading-snug text-muted-foreground tabular-nums"
            title={l}
          >
            {l}
          </span>
        ))}
      </div>
    </div>
  )
}

// UsageSection is how much of each plan window is left: a bar, when it
// starts over, and when the reading was taken if that was a while ago.
function UsageSection({ usage, now }: { usage: UsageLimits; now: number }) {
  const stale = staleLine(usage.observedAtMs, now)
  return (
    <section className="px-3 pt-3 pb-3">
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <h3 className="font-medium text-muted-foreground">Claude usage left</h3>
        {stale && hasUsage(usage) && (
          <span className="text-muted-foreground tabular-nums">{stale}</span>
        )}
      </div>
      {!hasUsage(usage) ? (
        <p className="mt-1.5 text-xs text-muted-foreground">
          No reading yet: appears after a Claude Code reply on this computer
        </p>
      ) : (
        <div className="mt-2 flex flex-col gap-2.5">
          {sortWindows(usage.windows).map((w) => {
            const left = percentLeft(w, now)
            const reset = resetLine(w.resetsAtMs, now)
            return (
              <div key={w.kind} className="flex flex-col gap-1">
                <div className="flex items-baseline justify-between gap-2 text-xs">
                  <span className="font-medium text-ink">
                    {windowLabel(w.kind)}
                  </span>
                  <span className="text-ink tabular-nums">{left}% left</span>
                </div>
                <div
                  aria-hidden
                  className="h-1.5 overflow-hidden rounded-full bg-muted"
                >
                  <div
                    className={cn(
                      "h-full rounded-full",
                      left <= 10
                        ? "bg-destructive"
                        : left <= 25
                          ? "bg-warning"
                          : "bg-primary"
                    )}
                    style={{ width: `${left}%` }}
                  />
                </div>
                {reset && (
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {reset}
                  </span>
                )}
              </div>
            )
          })}
        </div>
      )}
    </section>
  )
}
