"use client"

import * as React from "react"
import { Radio as RadioPrimitive } from "@base-ui/react/radio"
import { RadioGroup as RadioGroupPrimitive } from "@base-ui/react/radio-group"
import { cn } from "cn"

type RadioGroupVariant = "default" | "chips"

const RadioGroupContext = React.createContext<RadioGroupVariant>("default")

function RadioGroup<Value>({
  className,
  variant = "default",
  children,
  ...props
}: RadioGroupPrimitive.Props<Value> & { variant?: RadioGroupVariant }) {
  return (
    <RadioGroupPrimitive
      data-slot="radio-group"
      data-variant={variant}
      className={cn(
        "grid w-full gap-2.5 data-[variant=chips]:flex data-[variant=chips]:flex-wrap data-[variant=chips]:gap-1.5",
        className
      )}
      {...props}
    >
      <RadioGroupContext.Provider value={variant}>
        {children}
      </RadioGroupContext.Provider>
    </RadioGroupPrimitive>
  )
}

function RadioGroupItem<Value>({
  className,
  children,
  ...props
}: RadioPrimitive.Root.Props<Value>) {
  const variant = React.useContext(RadioGroupContext)

  /* Chips carry their own text; the default dot does not, so wrap it in a
   * Label for its text. */
  if (variant === "chips") {
    return (
      <RadioPrimitive.Root
        data-slot="radio-group-item"
        data-variant="chips"
        className={cn(
          "inline-flex h-7 shrink-0 items-center justify-center gap-1.5 rounded-4xl border border-border bg-transparent px-3 text-sm font-medium whitespace-nowrap text-foreground transition-all outline-none select-none hover:bg-muted focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 data-checked:border-transparent data-checked:bg-accent data-checked:text-accent-foreground data-disabled:pointer-events-none data-disabled:opacity-50 aria-invalid:border-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
          className
        )}
        {...props}
      >
        {children}
      </RadioPrimitive.Root>
    )
  }

  return (
    <RadioPrimitive.Root
      data-slot="radio-group-item"
      data-variant="default"
      className={cn(
        "group/radio-group-item peer relative flex aspect-square size-4 shrink-0 items-center justify-center rounded-full border border-muted-foreground/80 bg-background outline-none after:absolute after:-inset-x-3 after:-inset-y-2 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 data-checked:border-primary data-checked:bg-primary data-checked:text-primary-foreground data-disabled:cursor-not-allowed data-disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:bg-input/30",
        className
      )}
      {...props}
    >
      <RadioPrimitive.Indicator
        data-slot="radio-group-indicator"
        className="flex size-4 items-center justify-center"
      >
        <span className="size-1.5 rounded-full bg-primary-foreground" />
      </RadioPrimitive.Indicator>
    </RadioPrimitive.Root>
  )
}

export { RadioGroup, RadioGroupItem }
