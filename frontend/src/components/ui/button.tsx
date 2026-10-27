import { Button as ButtonPrimitive } from "@base-ui/react/button"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "cn"

const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center rounded-lg border border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 active:not-aria-[haspopup]:translate-y-px disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/80",
        outline:
          "border-border bg-background hover:bg-muted hover:text-foreground aria-expanded:bg-muted aria-expanded:text-foreground dark:border-input dark:bg-input/30 dark:hover:bg-input/50",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-[color-mix(in_oklch,var(--secondary),var(--foreground)_5%)] aria-expanded:bg-secondary aria-expanded:text-secondary-foreground",
        ghost:
          "hover:bg-muted hover:text-foreground aria-expanded:bg-muted aria-expanded:text-foreground dark:hover:bg-muted/50",
        destructive:
          "bg-destructive/10 text-destructive hover:bg-destructive/20 focus-visible:border-destructive/40 focus-visible:ring-destructive/20 dark:bg-destructive/20 dark:hover:bg-destructive/30 dark:focus-visible:ring-destructive/40",
        link: "text-primary underline-offset-4 hover:underline",
        hud: "border-[#dde3ee] bg-[rgba(255,255,255,0.92)] text-ink shadow-none hover:bg-[rgba(255,255,255,0.92)] aria-expanded:bg-[rgba(255,255,255,0.92)] aria-expanded:text-ink active:translate-y-0 focus-visible:border-[#dde3ee] focus-visible:ring-0 [&_svg]:!h-auto [&_svg]:!w-auto",
        "hud-icon":
          "border-[#d9e0eb] bg-white text-ink shadow-[0_1px_2px_rgba(30,41,90,0.05)] hover:bg-white active:translate-y-0 focus-visible:border-[#d9e0eb] focus-visible:ring-0 [&_svg]:!h-auto [&_svg]:!w-auto",
        "hud-ghost":
          "border-transparent bg-transparent text-ink shadow-none hover:bg-transparent aria-expanded:bg-transparent active:translate-y-0 focus-visible:border-transparent focus-visible:ring-0 [&_svg]:!h-auto [&_svg]:!w-auto",
        "hud-cam":
          "rounded-[10px] border-transparent bg-transparent text-ink shadow-none hover:bg-[rgba(241,245,249,0.9)] active:translate-y-0 focus-visible:border-transparent focus-visible:ring-0 [&_svg]:!h-auto [&_svg]:!w-auto",
        "hud-row":
          "justify-start border-transparent bg-transparent text-left text-ink shadow-none hover:bg-[rgba(219,234,254,0.55)] aria-expanded:bg-transparent data-[selected=true]:bg-[rgba(219,234,254,0.55)] active:translate-y-0 focus-visible:border-transparent focus-visible:ring-0 [&_svg]:!h-auto [&_svg]:!w-auto",
        "hud-ship":
          "justify-start border-0 bg-[rgba(226,233,245,0.75)] bg-clip-border text-left text-ink shadow-none hover:bg-[rgba(226,233,245,0.75)] active:translate-y-0 focus-visible:border-0 focus-visible:ring-0 [&_svg]:!h-auto [&_svg]:!w-auto",
      },
      size: {
        default:
          "h-8 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2",
        xs: "h-6 gap-1 rounded-[min(var(--radius-md),10px)] px-2 text-xs in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-7 gap-1 rounded-[min(var(--radius-md),12px)] px-2.5 text-[0.8rem] in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-9 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2",
        icon: "size-8",
        "icon-xs":
          "size-6 rounded-[min(var(--radius-md),10px)] in-data-[slot=button-group]:rounded-lg [&_svg:not([class*='size-'])]:size-3",
        "icon-sm":
          "size-7 rounded-[min(var(--radius-md),12px)] in-data-[slot=button-group]:rounded-lg",
        "icon-lg": "size-9",
        "hud-site": "h-[46px] w-[295px] gap-0 rounded-[12px] pr-0 pl-1.5 font-normal",
        "hud-icon": "size-[29px] rounded-lg [&_svg:not([class*='size-'])]:size-[18px]",
        "hud-cam": "h-[35px] w-[38px] rounded-[10px]",
        "hud-bell": "size-[34px] rounded-none",
        "hud-row": "flex h-9 w-full gap-0 rounded-lg px-0.5 font-normal",
        "hud-ship": "h-[113px] w-[282px] gap-0 rounded-[12px] pr-3.5 pl-3 font-normal",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  return (
    <ButtonPrimitive
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
