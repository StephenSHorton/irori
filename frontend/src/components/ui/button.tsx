import { Button as ButtonPrimitive } from "@base-ui/react/button"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "cn"

const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center rounded-lg border border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-[color,background-color,border-color,transform] duration-100 outline-none select-none focus-visible:ring-1 focus-visible:ring-ring active:not-aria-[haspopup]:scale-[0.97] disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        outline:
          "border-border bg-transparent hover:bg-accent hover:text-accent-foreground aria-expanded:bg-accent aria-expanded:text-accent-foreground",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-secondary/80 aria-expanded:bg-secondary aria-expanded:text-secondary-foreground",
        ghost:
          "text-muted-foreground hover:bg-accent hover:text-accent-foreground aria-expanded:bg-accent aria-expanded:text-accent-foreground",
        destructive:
          "bg-destructive text-destructive-foreground hover:bg-destructive/90 focus-visible:ring-destructive",
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
          "h-9 gap-2 px-4 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3",
        xs: "h-6 gap-1 rounded-[min(var(--radius-md),10px)] px-2 text-xs in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-8 gap-1.5 rounded-[min(var(--radius-md),12px)] px-3 text-xs in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-9 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2",
        icon: "size-9",
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
