// Irori design system gallery (card #259). Browser preview only:
// http://127.0.0.1:9246/?gallery&glass=frost&mode=light
// Renders the shadcn components on a backdrop under one glass style so the
// styles can be compared on real components. Not reachable from the app.
import { useEffect, useState } from "react"
import { BellIcon, CheckIcon, SearchIcon, SparklesIcon } from "lucide-react"

import "./irori-ds.css"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Avatar, AvatarFallback, AvatarGroup } from "@/components/ui/avatar"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Command,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command"
import { Input } from "@/components/ui/input"
import { Kbd } from "@/components/ui/kbd"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Separator } from "@/components/ui/separator"
import { Skeleton } from "@/components/ui/skeleton"
import { Slider } from "@/components/ui/slider"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { glassStyles } from "@/lib/appearance"

export const GLASS_STYLES = glassStyles

function readParams() {
  const p = new URLSearchParams(window.location.search)
  const glass = GLASS_STYLES.some((s) => s.id === p.get("glass"))
    ? p.get("glass")!
    : "frost"
  const mode = p.get("mode") === "dark" ? "dark" : "light"
  return { glass, mode }
}

const agents = [
  {
    name: "sample-otter",
    device: "Macbook Pro (this one)",
    card: "#1 Ship example-app v0.6.0",
    state: "on it",
  },
  {
    name: "sample-heron",
    device: "Macbook Pro (this one)",
    card: "#8 Release notes",
    state: "working",
  },
  {
    name: "sample-wren",
    device: "Desktop Computer",
    card: "#9 Announcement",
    state: "offline",
  },
  { name: "sample-lark", device: "Desktop Computer", card: "—", state: "free" },
]

export default function Gallery() {
  const [{ glass, mode }, setView] = useState(readParams)

  useEffect(() => {
    const html = document.documentElement
    html.dataset.ds = "irori"
    html.dataset.glass = glass
    html.classList.toggle("dark", mode === "dark")
    // index.html paints the opening screen dark inline; the gallery owns the page.
    html.style.background = ""
    html.style.colorScheme = mode
    document.body.style.background = ""
    const url = new URL(window.location.href)
    url.searchParams.set("glass", glass)
    url.searchParams.set("mode", mode)
    window.history.replaceState(null, "", url)
  }, [glass, mode])

  const style = GLASS_STYLES.find((s) => s.id === glass)!

  return (
    <div className="relative min-h-screen overflow-hidden">
      {/* Screenshots: headless Chrome freezes CSS transitions mid-way, so the gallery has none. */}
      <style>{`html[data-ds="irori"] *, html[data-ds="irori"] *::before, html[data-ds="irori"] *::after { transition: none !important; }`}</style>
      <Backdrop />
      <div className="relative mx-auto flex max-w-6xl flex-col gap-6 px-4 py-8 sm:px-8">
        <header className="flex flex-wrap items-end gap-4">
          <div className="w-full sm:w-auto sm:flex-1">
            <p className="text-xs font-semibold tracking-widest text-muted-foreground uppercase">
              Irori design system · draft
            </p>
            <h1 className="text-3xl font-semibold tracking-tight">
              {style.name} <span className="text-muted-foreground">glass</span>
            </h1>
            <p className="text-muted-foreground">{style.note}</p>
          </div>
          <ToggleGroup
            value={[glass]}
            onValueChange={(v: string[]) =>
              v[0] && setView((s) => ({ ...s, glass: v[0] }))
            }
            aria-label="Glass style"
            className="max-w-full overflow-x-auto"
          >
            {GLASS_STYLES.map((s) => (
              <ToggleGroupItem key={s.id} value={s.id}>
                {s.name}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <div className="flex items-center gap-2">
            <Label htmlFor="dark">Dark</Label>
            <Switch
              id="dark"
              checked={mode === "dark"}
              onCheckedChange={(c: boolean) =>
                setView((s) => ({ ...s, mode: c ? "dark" : "light" }))
              }
            />
          </div>
        </header>

        <div className="columns-1 gap-6 md:columns-2 xl:columns-3 [&>*]:mb-6 [&>*]:break-inside-avoid">
          <Card>
            <CardHeader>
              <CardTitle>Ship example-app v0.6.0</CardTitle>
              <CardDescription>
                #1 · sample-org/example-app · sample-otter on it
              </CardDescription>
              <CardAction>
                <Badge variant="destructive">3 decisions</Badge>
              </CardAction>
            </CardHeader>
            <CardContent className="flex flex-col gap-3">
              <Progress value={60} />
              <p className="text-sm text-muted-foreground">
                3 of 5 steps · 60%
              </p>
              {[
                ["Boat batch merged (#206, #207)", true],
                ["Log-flood fix merged (#209)", true],
                ["Release notes up to main", true],
                ["Two-PC co-op check in the temple level", false],
                ["Tag v0.6.0 and publish", false],
              ].map(([t, done], i) => (
                <label key={i} className="flex items-center gap-2 text-sm">
                  <Checkbox defaultChecked={done as boolean} />
                  {t as string}
                </label>
              ))}
            </CardContent>
            <CardFooter className="gap-2">
              <Button>Answer</Button>
              <Button variant="outline">Pin</Button>
              <Button variant="ghost">More</Button>
            </CardFooter>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>
                Ship today, or wait for the two-PC temple check?
              </CardTitle>
              <CardDescription>
                Raised by sample-otter · 18m ago
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <RadioGroup defaultValue="wait" className="gap-3">
                <label className="flex items-start gap-2 text-sm">
                  <RadioGroupItem value="wait" />
                  <span>
                    <span className="font-medium">
                      Wait for the check, ship tonight
                    </span>
                    <span className="block text-muted-foreground">
                      sample-otter recommends this
                    </span>
                  </span>
                </label>
                <label className="flex items-start gap-2 text-sm">
                  <RadioGroupItem value="now" />
                  <span>
                    <span className="font-medium">
                      Ship now, mark temple co-op as experimental
                    </span>
                    <span className="block text-muted-foreground">
                      Option from sample-heron
                    </span>
                  </span>
                </label>
              </RadioGroup>
              <Textarea placeholder="Or answer in your own words" />
            </CardContent>
            <CardFooter className="justify-end gap-2">
              <Button variant="secondary">Later</Button>
              <Button>Answer</Button>
            </CardFooter>
          </Card>

          <Command>
            <CommandInput placeholder="Search cards, boards, agents…" />
            <CommandList>
              <CommandGroup heading="Needs you">
                <CommandItem>
                  <CheckIcon /> #12 Fix the boat clipping through the dock
                </CommandItem>
                <CommandItem>
                  <SparklesIcon /> #3 Two-PC lobby join
                  <CommandShortcut>⌘3</CommandShortcut>
                </CommandItem>
              </CommandGroup>
              <CommandGroup heading="Boards">
                <CommandItem>
                  <SearchIcon /> MLG
                </CommandItem>
              </CommandGroup>
            </CommandList>
          </Command>

          <Alert>
            <BellIcon />
            <AlertTitle>sample-heron sent you a message</AlertTitle>
            <AlertDescription>
              Should a joiner keep their own animals in their own save?
            </AlertDescription>
          </Alert>

          <Alert variant="destructive">
            <AlertTitle>Irori held a message</AlertTitle>
            <AlertDescription>
              sample-grok isn't on your allow list.
            </AlertDescription>
          </Alert>

          <Card>
            <CardHeader>
              <CardTitle>New card</CardTitle>
              <CardDescription>
                Put work on the Board for a free agent.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <div className="flex flex-col gap-2">
                <Label htmlFor="g-title">Title</Label>
                <Input
                  id="g-title"
                  defaultValue="Back up the out/ folder before the re-run"
                />
              </div>
              <div className="flex flex-col gap-2">
                <Label>How soon</Label>
                <Slider defaultValue={[60]} />
              </div>
              <div className="flex items-center justify-between">
                <Label htmlFor="g-wake">Tell them now</Label>
                <Switch id="g-wake" defaultChecked />
              </div>
            </CardContent>
            <CardFooter className="justify-end gap-2">
              <Button variant="outline">Cancel</Button>
              <Button>Create</Button>
            </CardFooter>
          </Card>

          <Tabs defaultValue="agents">
            <TabsList>
              <TabsTrigger value="agents">Agents</TabsTrigger>
              <TabsTrigger value="devices">Devices</TabsTrigger>
              <TabsTrigger value="people">People</TabsTrigger>
            </TabsList>
            <TabsContent value="agents">
              <Card size="sm">
                <CardContent>
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Agent</TableHead>
                        <TableHead className="text-right">State</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {agents.map((a) => (
                        <TableRow key={a.name}>
                          <TableCell>
                            <div className="font-medium">{a.name}</div>
                            <div className="text-xs text-muted-foreground">
                              {a.device} · {a.card}
                            </div>
                          </TableCell>
                          <TableCell className="text-right">
                            <Badge
                              variant={
                                a.state === "offline"
                                  ? "warning"
                                  : a.state === "free"
                                    ? "slate"
                                    : "success"
                              }
                            >
                              {a.state}
                            </Badge>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </CardContent>
              </Card>
            </TabsContent>
          </Tabs>

          <Card>
            <CardHeader>
              <CardTitle>Bits</CardTitle>
              <CardDescription>Badges, people, keys, loading</CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <div className="flex flex-wrap gap-2">
                <Badge>Needs you</Badge>
                <Badge variant="info">Working</Badge>
                <Badge variant="warning">Waiting</Badge>
                <Badge variant="success">Done</Badge>
                <Badge variant="outline">MLG</Badge>
              </div>
              <div className="flex items-center gap-3">
                <AvatarGroup>
                  {["O", "H", "W"].map((n) => (
                    <Avatar key={n}>
                      <AvatarFallback>{n}</AvatarFallback>
                    </Avatar>
                  ))}
                </AvatarGroup>
                <span className="text-sm text-muted-foreground">3 working</span>
                <span className="flex-1" />
                <Kbd>⌘K</Kbd>
              </div>
              <Separator />
              <div className="flex items-center gap-3">
                <Spinner />
                <div className="flex flex-1 flex-col gap-2">
                  <Skeleton className="h-3 w-3/4" />
                  <Skeleton className="h-3 w-1/2" />
                </div>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardContent>
              <Accordion defaultValue={["what"]}>
                <AccordionItem value="what">
                  <AccordionTrigger>What is a board?</AccordionTrigger>
                  <AccordionContent>
                    A project you share: its cards, the people on it and their
                    agents.
                  </AccordionContent>
                </AccordionItem>
                <AccordionItem value="held">
                  <AccordionTrigger>Why was a message held?</AccordionTrigger>
                  <AccordionContent>
                    Its sender isn't on your allow list, or wakes were paused.
                  </AccordionContent>
                </AccordionItem>
              </Accordion>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}

// Something behind the glass: warm light from the hearth, smoke, and a faint
// grid so blur and clarity read at a glance.
function Backdrop() {
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0">
      <div className="absolute inset-0 [background-image:linear-gradient(var(--border)_1px,transparent_1px),linear-gradient(90deg,var(--border)_1px,transparent_1px)] [background-size:48px_48px] opacity-[0.35]" />
      <div className="absolute -top-24 -left-24 size-[34rem] rounded-full bg-[oklch(0.72_0.17_45)] opacity-55 blur-3xl dark:bg-[oklch(0.55_0.18_40)]" />
      <div className="absolute top-[30%] right-[-8rem] size-[30rem] rounded-full bg-[oklch(0.62_0.14_330)] opacity-40 blur-3xl dark:bg-[oklch(0.42_0.14_320)]" />
      <div className="absolute bottom-[-6rem] left-[30%] size-[28rem] rounded-full bg-[oklch(0.7_0.1_190)] opacity-40 blur-3xl dark:bg-[oklch(0.45_0.09_195)]" />
      <div className="absolute top-[12%] left-[45%] h-40 w-72 rotate-12 rounded-3xl bg-[oklch(0.82_0.14_85)] opacity-60 dark:bg-[oklch(0.6_0.13_80)]" />
      <div className="absolute top-[62%] left-[6%] h-48 w-48 rounded-full border-[14px] border-[oklch(0.56_0.17_38)] opacity-50" />
    </div>
  )
}
