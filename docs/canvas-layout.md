# Canvas layout: why a card sits where it does

The Board's canvas (Board › Canvas) is a map of who is doing what. This is
the policy for where things go on it, as rules with names, so the canvas
can say why a card is where it is (hover a card: "Why it's here"). Card
#776 wrote it and moved the layout onto ELK; card #587 made the map.

The code follows this page. The weights are in
`frontend/src/components/canvas/weights.ts`, the ELK graph in `elk.ts`, the
lines in `route.ts`. Change a weight there and here together.

## 1. Pulls: which area a card is in, and what it sits next to

Pulls are structure. They decide a card's area and its neighbours, and they
always come before prominence.

| Pull | What it does |
| --- | --- |
| **Holder** | A card sits in the hub of its first holder that is here (running, on a device that's online), under the agent's pill. You are a hub too when you hold a card. |
| **Nobody on it** | A card with no holder here sits in the amber "Nobody on these" area. |
| **Subtask** | A subtask in the same area sits under its parent, with a line down to it. A subtask held by someone else stays in its holder's hub; its line crosses over. |
| **Blocker** | An open blocker sits right under its card. |
| **Talking** | Two agents writing to each other on a card in the last few minutes: their hubs sit next to each other, so the line between them is short. |
| **Done** | A done card goes to the Done area, faded, at the end. It has no blockers or agent lines. |

Agents that are here but hold no card of their own (watching, talking, a
second holder) share the "Also here" strip, with lines to their cards.

## 2. Prominence: what comes first

Inside an area, cards go in reading order (left to right, row by row) by
**prominence**, most prominent first. A card's prominence is the sum of the
weights of what is true of it:

| Weight | Value | When |
| --- | --- | --- |
| `needsYou` | +100 | It waits on your answer or your Done. |
| `live` | +40 | An agent that is here is working on it right now. |
| `recent` | +20 | It changed in the last hour. |
| `stale` | −30 | Nothing has happened on it for a while. |

Ties go by card number (oldest first), then id. A parent and its subtasks
move as one: a tree counts as much as its most prominent card, and each
subtask is ordered among its siblings the same way.

Areas are ordered the same way, and packed in rows about as wide as the
window (2.2 to 1):

| Area weight | Value | When |
| --- | --- | --- |
| `you` | +1000 | Your own hub leads. |
| top card | its prominence | A hub counts as much as its most prominent card. |
| `working` | +40 | The hub's agent is working now. |
| `idle` | −20 | The hub's agent is idle. |

Ties go by the number of cards, then the agent's name. Then come, in this
order: the cards nobody is on, the "Also here" strip, and Done.

## 3. Fades

Done cards fade and go last. Stale cards go to the back of their area
(`stale`). An idle agent's hub falls behind the working ones (`idle`).

## 4. Why it's here

Hovering a card shows its top three reasons, strongest first: where it is
(its area, then its parent), then what put it forward or back, with "Kept
where you put it" first when the person placed it. The texts are
`reasons()` in `weights.ts`.

## 5. How it is laid out

[ELK](https://eclipse.dev/elk/) (elkjs, EPL-2.0) lays the board out:

- Each area is an ELK compound node, laid out on its own
  (`hierarchyHandling: SEPARATE_CHILDREN`) by `layered`, top to bottom.
  Subtask lines are its edges, so a subtask sits under its parent and ELK
  routes the line orthogonally around the other cards. Cards with no line
  between them are packed in rows (`separateConnectedComponents`,
  `aspectRatio` 1.6 for a hub, 2.4 for the big areas), so no area grows into
  a tall column.
- The areas are packed by `rectpacking` in the order of section 2.
- Every list goes to ELK sorted by these rules, never in the order the cards
  arrived, and `randomSeed` is fixed. The same board lays out the same on
  every device.
- Agent pills and blockers stay out of ELK. A hub's padding leaves room for
  its pill at the top. A card with blockers is that much taller to ELK, and
  its blockers are put under it afterwards (`followers`), as after a drag.

The canvas lays out again only when something the layout depends on changes
(`layoutKey`: the cards, their areas, parents, prominence, blockers, agents
and talk lines), never for a title, steps or a clock tick. The last picture
stays up while the next one is worked out.

A card the person dragged keeps its spot (`withPlaced`, kept in coord) while
it lies inside its own area; one that would land elsewhere goes back to the
layout. Tidy up puts every card back.

ELK runs in a web worker (`elk-worker.ts`): elkjs's API (about 10 KB) is in
the canvas's lazy chunk, and its 1.6 MB engine is a file of its own that
loads only when the canvas is opened. The window's CSP
(`default-src 'self'`) allows it because it is served from the app; a
`blob:` or inline worker would be refused.

## 6. Lines

Every line is drawn on points from `route.ts`, orthogonal with rounded
corners:

- A line inside an area uses ELK's route while it still fits: it starts on
  its source's border, ends on its target's, and passes through no card,
  blocker or pill where they are now.
- Anything else is routed in the window: a line between areas (a second
  holder, a subtask in another hub, agents talking), or one a dragged card
  broke. The router finds the shortest path with few bends on a grid made of
  the lines just outside each nearby node (12 px out, half the gap between
  two cards). It leaves and enters by whichever sides suit where the two
  ends actually are, never through another node.
- A blocker's line leaves its left side and goes up the strip its inset
  leaves clear under the card, so it never crosses the blockers above it.
- A holder line inside its agent's own hub is not drawn, hover or not: the
  hub's box already says who holds what, and the line used to cut through the
  cards in between (#774). A holder line to a card in another area is
  drawn. Watcher lines show only while you hover the agent or the card.

## 7. Not done yet

- Prominence sets order, not size: a card that needs you comes first, but it
  is not drawn bigger.
- There is no weight yet for how long a card has waited on you, or for a
  card's priority.
- If ELK ever proves too rigid, WebCola (constraint layout) is the
  alternative to compare; ask before switching.
