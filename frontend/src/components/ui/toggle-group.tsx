import * as React from "react"
import { Toggle as TogglePrimitive } from "@base-ui/react/toggle"
import { ToggleGroup as ToggleGroupPrimitive } from "@base-ui/react/toggle-group"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "cn"

const toggleGroupVariants = cva(
  "group/toggle-group inline-flex w-fit items-center text-muted-foreground data-vertical:flex-col data-vertical:items-stretch",
  {
    variants: {
      variant: {
        /* Segmented, like the tabs list. */
        default: "gap-0 rounded-lg bg-muted p-[3px]",
        /* Separate pills, for filter chips. */
        chips: "flex-wrap gap-1.5",
      },
      size: {
        default: "",
        sm: "",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

const toggleGroupItemVariants = cva(
  "group/toggle-group-item inline-flex shrink-0 items-center justify-center gap-1.5 border border-transparent font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          "rounded-md text-foreground/60 hover:text-foreground data-pressed:bg-background data-pressed:text-foreground data-pressed:shadow-sm dark:data-pressed:border-input dark:data-pressed:bg-input/30",
        chips:
          "rounded-4xl border-border bg-transparent text-foreground hover:bg-muted data-pressed:border-transparent data-pressed:bg-accent data-pressed:text-accent-foreground",
      },
      size: {
        default: "h-[calc(2rem-6px)] px-2 text-sm",
        sm: "h-[calc(1.75rem-6px)] px-1.5 text-[0.8rem] [&_svg:not([class*='size-'])]:size-3.5",
      },
    },
    compoundVariants: [
      { variant: "chips", size: "default", className: "h-7 px-3" },
      { variant: "chips", size: "sm", className: "h-6 px-2.5" },
    ],
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

type ToggleGroupStyle = VariantProps<typeof toggleGroupItemVariants>

const ToggleGroupContext = React.createContext<ToggleGroupStyle>({
  variant: "default",
  size: "default",
})

function ToggleGroup<Value extends string>({
  className,
  variant = "default",
  size = "default",
  allowEmpty = false,
  multiple,
  onValueChange,
  children,
  ...props
}: ToggleGroupPrimitive.Props<Value> &
  VariantProps<typeof toggleGroupVariants> & {
    /** Single-select groups keep one item pressed unless this is true. */
    allowEmpty?: boolean
  }) {
  return (
    <ToggleGroupPrimitive
      data-slot="toggle-group"
      data-variant={variant}
      data-size={size}
      className={cn(toggleGroupVariants({ variant, size }), className)}
      multiple={multiple}
      onValueChange={(next, eventDetails) => {
        /* A segmented switch or view toggle should never end up with
         * nothing selected: ignore the click that would unpress the last. */
        if (!multiple && !allowEmpty && next.length === 0) {
          eventDetails.cancel()
          return
        }
        onValueChange?.(next, eventDetails)
      }}
      {...props}
    >
      <ToggleGroupContext.Provider value={{ variant, size }}>
        {children}
      </ToggleGroupContext.Provider>
    </ToggleGroupPrimitive>
  )
}

function ToggleGroupItem<Value extends string>({
  className,
  variant,
  size,
  ...props
}: TogglePrimitive.Props<Value> & ToggleGroupStyle) {
  const context = React.useContext(ToggleGroupContext)
  const resolvedVariant = variant ?? context.variant
  const resolvedSize = size ?? context.size
  return (
    <TogglePrimitive
      data-slot="toggle-group-item"
      data-variant={resolvedVariant}
      data-size={resolvedSize}
      className={cn(
        toggleGroupItemVariants({ variant: resolvedVariant, size: resolvedSize }),
        className
      )}
      {...props}
    />
  )
}

export { ToggleGroup, ToggleGroupItem, toggleGroupVariants, toggleGroupItemVariants }
