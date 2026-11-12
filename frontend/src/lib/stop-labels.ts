// The words for the two switches (cards #665, #773, #787), in one place:
// `here` stops this machine (Pause agents in the code,
// AgentsService.PauseSet), `everywhere` stops every machine (Wind down in
// the code, WindDownService). The tray keeps its own copy in
// app/src/shell/tray_text.rs. Neither switch says "Pause" or "Wind down"
// to the person. `who` is the choice of agents either Stop reaches
// (decision 71), worded like the sidebar's Yours and On cards.

const agents = (n: number) => `${n} agent${n === 1 ? "" : "s"}`

export const stopLabels = {
  // This machine's switch: the header's Stop here.
  here: {
    name: "Stop here",
    on: "Stopped here",
    onShort: "Stopped",
    title:
      "Stop work on this machine: its agents write their hand-offs and leave their cards, and your other machines take them",
    titleCards:
      "Stop the agents Irori started on this machine: they write their hand-offs and leave their cards, your other machines take them, and nothing starts here until you start it again",
    titleYours:
      "Ask your own sessions on this machine to write their hand-offs and leave their cards. Nothing is held: Irori never starts them",
    choose: "Choose who Stop here reaches",
    onTitle: (at: string, by: string) =>
      `Stopped here since ${at}${by ? ` by ${by}` : ""}. ` +
      "This machine's agents write their hand-offs and leave their cards, your other machines take them, and nothing starts here until you start it again. Click to start here.",
    onAria: (at: string) => `Stopped here since ${at}. Start here`,
    start: "Start here",
    startTitle:
      "Start this machine again: hand-offs not asked for yet aren't, and work starts here again",
  },
  // Every machine's switch: the header's Stop everywhere and its dialog.
  everywhere: {
    name: "Stop everywhere",
    on: "Stopped everywhere",
    onShort: "Stopped",
    title:
      "Every agent on every machine writes its hand-off and leaves its card, and nothing starts anywhere until you start again",
    onTitle: (at: string, machines: string) =>
      `Stopped everywhere since ${at}, on ${machines}. Nothing starts anywhere until you start again. Click to start everywhere: agents not reached yet aren't asked, and work resumes.`,
    onAria: (at: string) => `Stopped everywhere since ${at}. Start everywhere`,
    explain:
      "Every agent on every machine writes its hand-off and leaves its card, and nothing starts anywhere until you start again.",
    stop: (n: number) => `Stop ${agents(n)}`,
    start: "Start everywhere",
    startTitle:
      "Start everywhere: agents not reached yet aren't asked, the stops it set lift and work resumes",
    palette: "Stop everywhere",
    paletteOff: "Start everywhere",
  },
  // Who either Stop reaches (decision 71): both ticked unless this
  // computer kept another choice.
  who: {
    legend: "Who stops",
    cards: (n: number) => `On cards (${n})`,
    cardsHint:
      "Agents Irori started for cards: they hand off, and nothing starts until you start again",
    yours: (n: number) => `Yours (${n})`,
    yoursHint:
      "Your own sessions: each hands off once. Nothing is held, since Irori never starts them",
    none: "Tick at least one",
    stopYours: (n: number) => `Ask ${n} of yours to hand off`,
  },
  // Ends a Yours-only run (it held nothing, so there is no Start).
  callOff: "Call off",
  callOffTitle:
    "Your sessions not reached yet aren't asked; the ones handing off finish",
  calledOff: "called off",
  // The dialog, for either switch's run.
  how: "Each agent finishes the step in hand, pushes its work, posts a hand-off on each card it holds and leaves it, releases its leases and answers you. Nothing is cut: a busy agent stops at its next safe point.",
  since: (at: string) => `Stopped since ${at}`,
  summary: (done: number, n: number) => `${done} of ${n} handed off`,
  // After the run: started again before it was through, or still stopped.
  startedAgain: "started again",
  stoppedEverywhere: "stopped everywhere",
  stopRow: "Stop",
  stopRowTitle: "Cut its running turn and ask it to hand off now",
  rows: {
    waiting: "Not reached yet",
    winding: "Handing off",
    done: "Handed off",
    no_answer: "No answer",
    // Card #787: started again before it reached the agent.
    canceled: "Called off",
  } as Record<string, string>,
  // Home's one line for a run (card #484).
  line: {
    noneReached: (n: number) => `Stopping reached none of ${agents(n)}`,
    running: (n: number, done: number) =>
      `Stopping ${agents(n)}, ${done} handed off so far`,
    done: (n: number, done: number) =>
      `Stopped ${agents(n)}, ${done} handed off`,
    // A Yours-only run: nothing was stopped, only asked.
    yoursNoneReached: (n: number) =>
      `Asking ${n} of yours to hand off reached none`,
    yoursRunning: (n: number, done: number) =>
      `Asking ${n} of yours to hand off, ${done} handed off so far`,
    yoursDone: (n: number, done: number) =>
      `Asked ${n} of yours to hand off, ${done} handed off`,
    openTitle: "Show each agent's answer",
  },
  machines: (names: string[]) => {
    if (names.length === 0) return "your machines"
    if (names.length === 1) return names[0]
    return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`
  },
}
