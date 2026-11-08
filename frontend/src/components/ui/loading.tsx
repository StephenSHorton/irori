import type { CSSProperties, ReactNode } from "react"
import {
  Skeleton as SkulletSkeleton,
  SkeletonBox as SkulletBox,
} from "skullet/react"
import { forgetShapes } from "skullet"

import { sweepEndless } from "@/lib/endless-animations"
import { previewFlag } from "@/lib/preview"

// skullet never cancels a skeleton's endless shimmer, which kept every
// skeleton ever shown alive and ran a core flat out (card #765).
sweepEndless()

// Loading is the app's one loading state (card #598). It draws a skeleton
// in the exact shape of what will load, so nothing jumps when it arrives:
// it renders the content from `sample` unseen, measures it and draws a bar
// for each line of text and a block for each control over it. Under the
// hood it is skullet (MIT, skullet/react), which is young (0.0.x), so
// nothing else in the app imports skullet: swap it here and every loading
// state follows (eslint.config.js holds the line).
//
// Colours come from the theme's tokens (index.css, --skullet-*), so every
// theme, light or dark, gets its own; the shimmer and the reveal hold still
// under the OS's Reduce Motion.
//
// While loading, the content is inert and hidden from screen readers, which
// read `label` instead. Keep `sample` free of side effects: it is rendered
// for real (hooks run), just never seen or clickable.
//
// Using it:
// - Draw `sample` from the hook-free *Shape markup in
//   components/common/loading-shapes.tsx, not from real rows fed sample
//   data: it is rendered for real, so their hooks would run unseen.
// - Show it on the first load only, never on a refresh of data already shown.
// - Mark a stand-in for a control with the `skullet-block=""` attribute; a
//   class does nothing.
// - `className` lands on an outer div, so put grid or gap classes on a div
//   inside it.
//
// In the browser preview, ?slow keeps every Loading on its skeleton
// (frontend/PREVIEW.md), to see them all without waiting on a read.

type Shared = {
  // Read to screen readers while loading: "Loading the card".
  label: string
  // Remembers the skeleton's shape under this key, so the next visit's
  // skeleton matches what was last shown (layout only, never text).
  id?: string
  // Shown in place of the content, and stops the skeleton, when set.
  error?: ReactNode
  className?: string
  style?: CSSProperties
}

type FromData<T> = Shared & {
  // Loading while null or undefined.
  data: T | null | undefined
  // Stands in for data to draw the first skeleton.
  sample: T
  // Forces the skeleton even with data (a refetch).
  loading?: boolean
  children: (data: T) => ReactNode
}

type Toggled = Shared & {
  loading: boolean
  data?: never
  sample?: never
  children: ReactNode
}

export type LoadingProps<T> = FromData<T> | Toggled

const slow = () => previewFlag("slow") !== null

export function Loading<T>(props: LoadingProps<T>) {
  const { label, id, error, className, style } = props
  const shared = { label, id, error, className, style }
  const forced = slow() && !error
  if (typeof props.children === "function") {
    const { data, sample, loading, children } = props as FromData<T>
    return (
      <SkulletSkeleton
        {...shared}
        data={forced ? null : data}
        sample={sample}
        loading={loading}
      >
        {children}
      </SkulletSkeleton>
    )
  }
  const { loading, children } = props as Toggled
  return (
    <SkulletSkeleton {...shared} loading={loading || forced}>
      {children}
    </SkulletSkeleton>
  )
}

// LoadingBox is one placeholder block, for a spot with no content to
// measure. Numbers are pixels.
export function LoadingBox(props: {
  width?: number | string
  height?: number | string
  radius?: number | string
  className?: string
}) {
  return <SkulletBox {...props} />
}

// forgetLoadingShapes drops every remembered skeleton shape (sign-out,
// a reset).
export function forgetLoadingShapes() {
  forgetShapes()
}
