# Talk: threads, channels, DMs and Spaces: design

*2026-11-03, card #382 (from Stephen's threads #381 and #383). Mockups: [docs/img/talk](../img/talk) (talk-1 … talk-6; made from branch `scabious/382-talk-mockups`, `/?mockup=talk&view=N`, not merged). Stephen's answers: #382 b1–b4. Amended 2026-11-05 for card #558 ([spaces-boards.md](spaces-boards.md)).*

Irori has card threads, ask threads that end when resolved, and agent send/reply messages that only show in transcripts. Nothing lasts, and nothing lets people and agents talk without a card. Talk adds one place for conversation with three kinds in it, and Spaces. A space is a group of people: it holds their devices, their boards and all their Talk. A board is only a list of tasks.

## 1. Decisions (Stephen, 2026-11-03)

| # | Decision |
|---|---|
| b1 | **Talk is a sidebar item** under Home. One list: Threads, Channels, Direct messages. It replaces Home's Threads panel and the MESSAGE cards in Needs you (a message to you is a DM). Home keeps only what needs you. |
| b2 | **Threads are only top-level.** A channel message can start a thread; the thread goes in the Threads list and the channel shows a one-line pointer to it. No replies hidden under messages. |
| b3 | **Agents are never cut off from a conversation.** An "Agents answer now" toggle on the composer and on each thread decides whether an agent is woken or handed it now. Off, nobody is woken or handed it, but agents in that channel or thread see it at their next turn and may reply when they have something relevant or a correction. Fixes #378 / irori#6. |
| b4 | **A space is the people.** It holds many boards, its channels and its threads; sharing and the receive setting (decision 24) are set once per space. A board is only a task list. Personal is you and your agents. `spaceId` is the first board's id, kept for storage (#558). |

## 2. The three kinds

| | Thread | Channel | DM |
|---|---|---|---|
| Lifetime | Open until resolved; resolved ones leave the list, stay searchable | Permanent, no resolve | Permanent |
| Belongs to | A space, never a board inside it (a card's own thread stays on the card) | A space | Two parties (person↔agent, person↔person, agent↔agent); not listed under any space (§5 says where its rows live) |
| Started by | You (Home's ask box, Talk's new-thread button), or from a channel message | Anyone in the space | Anyone |
| Agents woken | As today, gated by "Agents answer now" | Only by @mention | Every message wakes the other side |
| Shown | Talk › Threads | Talk › Channels | Talk › Direct messages; agent↔agent DMs read-only |

Today's ask threads (#116, decision 38) *are* Talk threads: same rows, same hand-out, new home. Card threads stay on the card, and Talk lists them on no board.

A thread started from a message in a space's channel lives in that space's log, so its members and their agents get it (#558). It records its channel (`ask.channel`, on a space's log too) and names no repo, since each member's device has its own checkouts. The channel's pointer carries the tag (`mlg#12`): a bare `#12` reads across every log and may be another card. With "My threads" private in that space, it stays in your own log and the channel gets no pointer, because nobody else could open it. Threads started from a space's channel before #558 stay in the private log, listed under Personal.

No `talk.*` row names a board, so a board inside a space changes none of this.

## 3. Who hears what

- **"Agents answer now"** defaults on when the text names nobody or names an agent, and off when it names only people. The person can flip it before sending, or on an open thread.
  - **On:** today's rules. Naming an agent sends it to that agent (decision 39). Naming nobody hands a thread to one free agent (decision 38). In a channel, naming nobody wakes nobody. *Since card #512:* a thread goes out at once whoever it names: the agents it names are woken, and an idle agent with Offer it cards on, or with none free an agent Irori starts, answers it; nobody waits on the named ones. *Since card #631:* a thread an agent opens (`talk_thread`) is held by that agent; the agents it names are woken, but nothing is handed out or started for it until the person writes in it.
  - **Off:** no wake and no hand-out. Named people get a desktop card.
- **Next-turn reading.** Every agent that's a member of a channel, or watches a thread, gets "since you last read" for it on its next wake: at most 5 lines inline and a count, plus `talk_read` for the rest. It replies only if it has something to add or correct. The plugin wording says so, and that a reply is optional.
  - *Built (#391):* the reading is a `reading` notice sent next_turn when a post lands. For a thread, "since" is after the agent's own last post there. For a channel it's after the agent's `talk.read` cursor and its own last post. A newer reading replaces one still waiting, so the agent sees one per conversation.
- **Membership.** Agents join a channel when someone @-mentions them or adds them, and leave with `talk_leave`. A gone agent stays listed, struck through, and isn't delivered to.
- The hop limit, rate limit, Held and Pause all apply unchanged.

## 4. Work stays on cards

Any message has "Make a card" (desktop menu, phone long-press). The message shows a chip linking the card. In a space's channel the card is made in that space, on its first board for now, and the chip reaches its members (a `talk.card` row in the space's log). A private card's chip stays on your own devices. Agents are told: work goes on a card, talk is for talking. `board_create from=` takes a thread's ref or id, not a post's; linking a post to a card is the window's.

## 5. Spaces

- **The space is the unit of sharing.** v0.5 gives each shared board its own log, file and connections (v0.5 §2.1). A space takes that role: `spaces/<spaceId>.db` holds the space's members, boards, cards, channels and threads, and travels only on that space's connections. Boards are a field inside a space (`task.created.board`, a `space.board.created` id, empty for the space's first board), so one invite shares every board and channel in it. Each board numbers its own cards (spaces-boards.md §2).
- **Personal** is the private log (v0.4) under a new name. It never replicates to other people.
- **Migration.** Each existing shared board becomes a one-board space with the same members and id lineage. Nothing changes for its members until someone adds a second board or a channel to it.
- **Settings move up.** Membership, owners, invites and removal (v0.5 §4), and the receive setting, are per space. **This amends decision 24**, which sets the receive setting per board and per person: it becomes per space and per person. A per-board override isn't kept in v1.
- **Profile.** Each person has a name, a colour and an avatar, and everyone sees them in boards, channels and DMs. Their agents show the colour as a stripe. This uses #385's pattern for person-wide settings: a private row written only by the person (`person.color`, last write wins), copied as a field on that person's `member.profile` in each space log. The field sits outside personSig, and only the person's certified devices can write a profile that counts. Each viewer's screen places the colours (`coord::devcolor::place_people`). New person-wide settings (avatar, status) use the same pattern. The avatar (#393) is a `person.avatar` row: a PNG, JPEG or WebP data URL of at most 32 KB (`coord::person_avatar`), which the window shrinks to a 128px square (WebP) before writing; it rides on `member.profile` as `avatar`, outside personSig like `color`.
- **Where DM rows live.** A DM between two people rides on a hidden two-person space, created on the first message. It has no boards and never gets any (the no-board rule, spaces-boards.md §1), and needs no invite, because the two already share a space. It isn't listed in the switcher. DMs show in Talk under All spaces and Personal, not under a shared space. A DM with an agent stays on your own devices' log.
- **The sidebar** gets a space switcher at the top. Talk and Boards then show that space; the Dashboard always shows every space and Needs you across all of them (#1001 took "All spaces" out of the picker). (#393: the pick is per device, kept in the window. Personal shows your private cards and threads and no shared boards; a shared space shows its cards, its threads, your private threads about it, and its boards; All spaces, the default, shows everything, every log's threads included (#558). A private thread about a space you left moves to Personal. A link to a thread or channel outside the pick still opens it.)

## 6. Data (rows)

New kinds. A space's channels keep theirs in the space log, so they replicate with it and pass the allow-list (v0.5 §2.2). Personal's channels, DMs with an agent and read cursors stay in the private log:

```
talk.channel.created  {channelId, spaceId, name, topic?, by}
talk.channel.edited   {channelId, name?, topic?, archived?}
talk.member           {convId, member: {kind: person|agent, key}, op: join|leave, by}
talk.post             {postId, convId, text, mentions[], agentsNow: bool, threadFrom?: postId, attach[]?, by, at}
talk.read             {convId, upTo: postId, by}           (own devices only, never on a shared space log)
talk.card             {postId, taskId}                     (a card made from a message; in the space log only when the card is in that space)
```

- `spaceId` is always the space's id, never a board's inside it; empty is Personal.
- A thread's `convId` is its task id, so ask rows fold unchanged. A channel's is its `channelId`, and a DM's is the pair's DM id.
- `message.sent` stays for agent send/reply. Agent↔agent messages are mirrored as read-only DM posts for the person.
- Edits and deletes are out of v1.

## 7. Agent tools

- `talk_post {conv, text, mention?, space?}` posts to a channel, DM or thread.
- `talk_thread {text, conv?, mention?, space?, agent?, watch?}` opens a thread with the person (card #631), in Personal or a space, about a channel when `conv` names one (a pointer post goes there). The agent holds it, the agents it names are woken, and no agent is handed or started for it, even after its opener ends, until the person writes in it ([spawned-agents.md](spawned-agents.md) §7). *Card #1027:* `agent="new"` starts a fresh agent to hold the thread (counted and refused as `agent_start` is, decision 51), `agent=<name>` hands it to that live agent; the opener then hears the replies only with `watch=true`.
- `talk_read {conv, since?, space?}` reads history, in pages.
- `talk_list` lists your channels and DMs, and the threads you're on in the private log and every space's, with unread counts.
- `talk_join` / `talk_leave {conv, space?}` join or leave a channel.
- `space` takes a space's tag or id. An inner board's tag means its space. It picks where a channel name is looked up, and where a bare `#12` is. Without it a bare `#12` that two logs hold is refused naming both, never read as the private log's; `mlg#12` and a thread's id work from anywhere (#558).
- Reading a space's thread adds the agent to nothing; its first post there puts it on the space.
- `send`/`reply` keep working. A `send` to a person is a DM post.

## 8. Screens

The desktop gets the Talk page: the list beside the conversation, members with live status, an @ picker that shows each agent's status, the "Agents answer now" toggle, and "Make a card". The thread toggle isn't drawn in the mockups (talk-4 shows the composer's). The phone gets a Talk tab in the bottom bar, with long-press actions. Only mentions and DMs notify on the phone; channel posts only bump the unread count. There's also a space switcher and a profile page.

## 9. Build order (cards)

1. Talk rows and fold: channels, posts, members and reads in the private log, with `talk_*` tools. Mac and PC.
2. Talk page on the desktop. Threads move from Home's panel into it, and the MESSAGE cards become DMs.
3. "Agents answer now" plus next-turn reading in the plugin's wake text, which fixes #378 (closes irori#6).
4. Spaces: the space log replaces the per-board log, existing shared boards migrate, and settings move to the space. Boards inside a space, and threads that follow the space, are #558.
5. Profile: name, colour (#385) and avatar, and the space switcher.
6. Person↔person DMs over two-person spaces (#538).
7. The phone's Talk tab and notifications.
8. Release, with screenshots on the card.
