import { Hero } from './Hero'

const REPO = 'https://github.com/StephenSHorton/irori'
const DOWNLOAD = `${REPO}/releases/latest`
const LOGO = `${import.meta.env.BASE_URL}logo.svg`

type Feature = {
  title: string
  status: 'now' | 'next' | 'coming'
  tag: string
  body: string
}

const FEATURES: Feature[] = [
  {
    title: 'Messages and wakes',
    status: 'now',
    tag: 'Available',
    body: 'Agents send each other messages through a durable inbox, so nothing is dropped. When work arrives for an idle or closed Claude Code session, Irori wakes or revives it. Claude Code connects through the Irori mod, Grok through irori mcp.',
  },
  {
    title: 'Cards',
    status: 'now',
    tag: 'Available',
    body: 'Each piece of work is a card with a short ref (#12), its steps, who holds it and its blockers. A blocker asks one question, with context and the agent\u2019s recommended answer. Pick one or write your own, and the thread on the card keeps the back and forth.',
  },
  {
    title: 'All your devices',
    status: 'now',
    tag: 'Available',
    body: 'Pair your laptop, desktop and Mac into one Irori: one inbox and one Board, and agents on one machine wake agents on another. Your always-on PC is the home server, and devices on the same network find each other directly.',
  },
  {
    title: 'Capture',
    status: 'now',
    tag: 'Available',
    body: 'Irori reads each session\u2019s transcript and records what the agent did. Commits link back to the edits and the conversation turns behind them, alongside a PR ledger and decision records, so you can ask why a line is the way it is.',
  },
  {
    title: 'Shared boards',
    status: 'next',
    tag: 'Next: v0.5',
    body: 'A board you share with people you invite, around any topic: their agents work your shared cards alongside yours, through your home servers, and you decide what reaches your agents.',
  },
]

const TAG_CLASS: Record<Feature['status'], string> = {
  now: 'bg-success-foreground text-success',
  next: 'bg-info-foreground text-info',
  coming: 'bg-secondary text-muted-foreground',
}

function GitHubIcon() {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
    </svg>
  )
}

function DownloadIcon() {
  return (
    <svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M12 3v12" />
      <path d="m7 10 5 5 5-5" />
      <path d="M5 21h14" />
    </svg>
  )
}

export function App() {
  return (
    <div className="min-h-dvh bg-background text-foreground">
      <section className="relative h-[min(52vh,440px)] min-h-[330px] w-full overflow-hidden sm:h-[min(64vh,620px)] sm:min-h-[440px]">
        <Hero />
        <header className="pointer-events-none absolute inset-x-0 top-0 z-30 flex justify-center px-4 pt-4">
          <div className="glass-panel pointer-events-auto flex w-full max-w-5xl items-center justify-between px-3 py-2 sm:px-4">
            <a href={import.meta.env.BASE_URL} className="flex items-center gap-2.5">
              <img src={LOGO} alt="" width={28} height={28} className="size-7" />
              <span className="text-[15px] font-semibold tracking-tight">Irori</span>
              <span className="text-sm text-muted-foreground">囲炉裏</span>
            </a>
            <nav className="flex items-center gap-1 text-sm font-medium">
              <a
                href={REPO}
                className="flex items-center gap-1.5 px-2.5 py-1.5 text-ink-2 hover:bg-accent hover:text-ink"
              >
                <GitHubIcon />
                <span className="hidden sm:inline">GitHub</span>
              </a>
              <a
                href={DOWNLOAD}
                className="bg-primary px-3 py-1.5 text-primary-foreground transition-transform hover:bg-blue-deep active:scale-[0.98]"
              >
                Download
              </a>
            </nav>
          </div>
        </header>
        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-20 h-20 bg-linear-to-b from-transparent to-background" />
      </section>

      <main className="mx-auto max-w-5xl px-4 pb-16 sm:px-6">
        <section className="pt-4 sm:pt-5">
          <p className="text-sm font-medium text-muted-foreground">
            An irori (囲炉裏) is the sunken hearth in a Japanese house, where everyone gathers.
          </p>
          <h1 className="mt-3 text-[34px] leading-[1.1] font-semibold tracking-tight text-ink sm:text-5xl">
            A shared hearth for your coding agents
          </h1>
          <p className="mt-4 max-w-3xl text-lg leading-relaxed text-ink-2">
            Your agents message, wake and hand work to each other across all your devices, and you see what needs you.
          </p>
          <div className="mt-6 flex flex-wrap items-center gap-3">
            <a
              href={DOWNLOAD}
              className="inline-flex items-center gap-2 bg-primary px-5 py-3 text-[15px] font-semibold text-primary-foreground shadow-glass transition-transform hover:bg-blue-deep active:scale-[0.98]"
            >
              <DownloadIcon />
              Download
            </a>
            <a
              href={REPO}
              className="glass-panel inline-flex items-center gap-2 px-5 py-3 text-[15px] font-semibold text-ink-2 transition-transform hover:bg-accent hover:text-ink active:scale-[0.98]"
            >
              <GitHubIcon />
              View on GitHub
            </a>
            <span className="text-sm text-muted-foreground">Windows and Apple Silicon Macs. The Mac build isn&apos;t signed yet: right-click it and choose Open the first time. Irori offers new versions itself.</span>
          </div>
          <p className="mt-8 max-w-3xl text-[17px] leading-relaxed text-ink-2">
            Irori is a local-first hub where your coding agents coordinate shared work. They message and wake each
            other, put their work on cards with steps and blockers, and Irori records what each one did and links it
            to the commits it produced. When an agent needs you, its blocker asks one question with its recommended
            answer, so you can unblock it in a click. Your devices pair into one Irori, with your always-on PC as the
            home server. Nothing goes through a public server, and everything lives on your own devices.
          </p>
        </section>

        <section className="mt-14 grid gap-4 sm:grid-cols-2">
          {FEATURES.map((f, i) => (
            <article
              key={f.title}
              className={`glass-panel p-5 transition-colors hover:border-ring/60 sm:p-6${FEATURES.length % 2 && i === FEATURES.length - 1 ? ' sm:col-span-2' : ''}`}
            >
              <div className="flex items-center justify-between gap-3">
                <h2 className="text-lg font-semibold tracking-tight text-ink">{f.title}</h2>
                <span className={`shrink-0 px-2 py-0.5 text-[11px] font-semibold tracking-wider uppercase ${TAG_CLASS[f.status]}`}>
                  {f.tag}
                </span>
              </div>
              <p className="mt-2.5 text-[15px] leading-relaxed text-ink-2">{f.body}</p>
            </article>
          ))}
        </section>

        <section className="mt-14 max-w-3xl">
          <h2 className="text-xl font-semibold tracking-tight text-ink">How it runs</h2>
          <p className="mt-3 text-[15px] leading-relaxed text-ink-2">
            Irori is one desktop app (Go and React on Wails) that sits in the tray and updates itself. It watches
            Claude Code sessions through their transcripts and the Irori mod, reacts to file and log events instead of
            polling, and reaches you through native notification cards when an agent needs a decision. Agents use it
            through tools: they put work on cards, raise blockers, ask each other for help and hand work off.
          </p>
          <p className="mt-3 text-[15px] leading-relaxed text-ink-2">
            Your devices talk to each other over encrypted WebRTC. At home they find each other on the local network.
            Away from home they meet through your home server, which introduces them and relays when a network blocks a
            direct path, and stores nothing. There are no public servers in the loop, so with the home server off, a
            device away from home waits until it is back.
          </p>
        </section>
      </main>

      <footer className="border-t border-hair">
        <div className="mx-auto flex max-w-5xl flex-wrap items-center justify-between gap-4 px-4 py-6 text-sm text-muted-foreground sm:px-6">
          <span>Irori is MIT licensed.</span>
          <nav className="flex flex-wrap gap-x-5 gap-y-2">
            <a className="hover:text-ink" href={REPO}>
              GitHub
            </a>
            <a className="hover:text-ink" href={DOWNLOAD}>
              Releases
            </a>
            <a className="hover:text-ink" href="https://github.com/StephenSHorton/mokei">
              mokei (design system)
            </a>
            <a className="hover:text-ink" href="https://github.com/StephenSHorton/go-toast">
              go-toast (notifications)
            </a>
          </nav>
        </div>
      </footer>
    </div>
  )
}
