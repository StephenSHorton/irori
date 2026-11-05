# Talk: threads, channels, DMs and Spaces: design

*2026-11-03, card #382 (from Stephen's threads #381 and #383). Mockups: [docs/img/talk](../img/talk) (talk-1 … talk-6; made from branch `scabious/382-talk-mockups`, `/?mockup=talk&view=N`, not merged). Stephen's answers: #382 b1–b4.*

Irori has card threads, ask threads that end when resolved, and agent send/reply messages that only show in transcripts. Nothing lasts, and nothing lets people and agents talk without a card. Talk adds one place for conversation with three kinds in it, and Spaces, a layer above boards that holds people, boards and channels together.

## 1. Decisions (Stephen, 2026-11-03)

| # | Decision |
|---|---|
| b1 | **Talk is a sidebar item** under Home. One list: Threads, Channels, Direct messages. It replaces Home's Threads panel and the MESSAGE cards in Needs you (a message to you is a DM). Home keeps only what needs you. |
| b2 | **Threads are only top-level.** A channel message can start a thread; the thread goes in the Threads list and the channel shows a one-line pointer to it. No replies hidden under messages. |
| b3 | **Agents are never cut off from a conversation.** An "Agents answer now" toggle on the composer and on each thread decides whether an agent is woken or handed it now. Off, nobody is woken or handed it, but agents in that channel or thread see it at their next turn and may reply when they have something relevant or a correction. Fixes #378 / irori#6. |
| b4 | **Spaces sit above boards.** A space is a group of people that holds many boards and channels; sharing and the receive setting (decision 24) are set once per space. Personal is you and your agents. |

## 2. The three kinds

| | Thread | Channel | DM |
|---|---|---|---|
| Lifetime | Open until resolved; resolved ones leave the list, stay searchable | Permanent, no resolve | Permanent |
| Belongs to | A space (a card's own thread stays on the card) | A space | Two parties (person↔agent, person↔person, agent↔agent); not listed under any space (§5 says where its rows live) |
| Started by | You (Home's ask box, Talk's new-thread button), or from a channel message | Anyone in the space | Anyone |
| Agents woken | As today, gated by "Agents answer now" | Only by @mention | Every message wakes the other side |
| Shown | Talk › Threads | Talk › Channels | Talk › Direct messages; agent↔agent DMs read-only |

Today's ask threads (#116, decision 38) *are* Talk threads: same rows, same hand-out, new home. Card threads stay on the card.

## 3. Who hears what

- **"Agents answer now"** defaults on when the text names nobody or names an agent, and off when it names only people. The person can flip it before sending, or on an open thread.
  - **On:** today's rules. Naming an agent sends it to that agent (decision 39). Naming nobody hands a thread to one free agent (decision 38). In a channel, naming nobody wakes nobody.
  - **Off:** no wake and no hand-out. Named people get a desktop card.
- **Next-turn reading.** Every agent that's a member of a channel, or watches a thread, gets "since you last read" for it on its next wake: at most 5 lines inline and a count, plus `talk_read` for the rest. It replies only if it has something to add or correct. The plugin wording says so, and that a reply is optional.
  - *Built (#391):* the reading is a `reading` notice sent next_turn when a post lands. For a thread, "since" is after the agent's own last post there. For a channel it's after the agent's `talk.read` cursor and its own last post. A newer reading replaces one still waiting, so the agent sees one per conversation.
- **Membership.** Agents join a channel when someone @-mentions them or adds them, and leave with `talk_leave`. A gone agent stays listed, struck through, and isn't delivered to.
- The hop limit, rate limit, Held and Pause all apply unchanged.

## 4. Work stays on cards

Any message has "Make a card" (desktop menu, phone long-press). The card's `from` is the message, the message shows a chip linking the card, and the card links back. Agents are told: work goes on a card (`board_create from=<message>`), talk is for talking.

## 5. Spaces

- **The space is the unit of sharing.** v0.5 gives each shared board its own log, file and connections (v0.5 §2.1). A space takes that role: `spaces/<spaceId>.db` holds the space's members, boards, cards, channels and threads, and travels only on that space's connections. Boards become a field inside a space, so one invite shares every board and channel in it. *Amended by §10 (#517):* people now join a space by being picked as a friend, not by a space invite.
- **Personal** is the private log (v0.4) under a new name. It never replicates to other people.
- **Migration.** Each existing shared board becomes a one-board space with the same members and id lineage. Nothing changes for its members until someone adds a second board or a channel to it.
- **Settings move up.** Membership, owners, invites and removal (v0.5 §4), and the receive setting, are per space. **This amends decision 24**, which sets the receive setting per board and per person: it becomes per space and per person. A per-board override isn't kept in v1.
- **Profile.** Each person has a name, a colour and an avatar, and everyone sees them in boards, channels and DMs. Their agents show the colour as a stripe. This uses #385's pattern for person-wide settings: a private row written only by the person (`person.color`, last write wins), copied as a field on that person's `member.profile` in each space log. The field sits outside personSig, and only the person's certified devices can write a profile that counts. Each viewer's screen places the colours (`coord::devcolor::place_people`). New person-wide settings (avatar, status) use the same pattern. The avatar (#393) is a `person.avatar` row: a PNG, JPEG or WebP data URL of at most 32 KB (`coord::person_avatar`), which the window shrinks to a 128px square (WebP) before writing; it rides on `member.profile` as `avatar`, outside personSig like `color`.
- **Where DM rows live.** A DM between two people rides on a hidden two-person space. *Amended by §10 (#517):* that space is the pair's **friend space**, made when one invites the other as a friend, not on the first message, and it can hold the boards the two make together. It isn't listed in the switcher: DMs show in Talk whichever space is picked. A DM with an agent stays on your own devices' log.
- **The sidebar** gets a space switcher at the top. Home, Talk and Boards then show that space, and "All spaces" shows Needs you across everything. (#393: the pick is per device, kept in the window. Personal shows your private cards and threads and no shared boards; a shared space shows its cards, threads and boards; All spaces, the default, is what the window showed before.)

## 6. Data (rows)

New kinds, all in the space log, so they replicate with it and pass the allow-list (v0.5 §2.2):

```
talk.channel.created  {channelId, spaceId, name, topic?, by}
talk.channel.edited   {channelId, name?, topic?, archived?}
talk.member           {convId, member: {kind: person|agent, key}, op: join|leave, by}
talk.post             {postId, convId, text, mentions[], agentsNow: bool, threadFrom?: postId, attach[]?, by, at}
talk.read             {convId, upTo: postId, by}           (own devices only, never on a shared space log)
talk.card             {postId, taskId}                     (a card made from a message)
```

- A thread's `convId` is its task id, so ask rows fold unchanged. A channel's is its `channelId`, and a DM's is the pair's DM id.
- `message.sent` stays for agent send/reply. Agent↔agent messages are mirrored as read-only DM posts for the person.
- Edits and deletes are out of v1.

## 7. Agent tools

- `talk_post {conv, text, mention?}` posts to a channel, DM or thread.
- `talk_read {conv, since?}` reads history, in pages.
- `talk_list` lists your channels, DMs and threads with unread counts.
- `talk_join` / `talk_leave` join or leave a channel.
- `board_create from=<postId>` makes a card from a message.
- `send`/`reply` keep working. A `send` to a person is a DM post.

## 8. Screens

The desktop gets the Talk page: the list beside the conversation, members with live status, an @ picker that shows each agent's status, the "Agents answer now" toggle, and "Make a card". The thread toggle isn't drawn in the mockups (talk-4 shows the composer's). The phone gets a Talk tab in the bottom bar, with long-press actions. Only mentions and DMs notify on the phone; channel posts only bump the unread count. There's also a space switcher and a profile page.

## 9. Build order (cards)

1. Talk rows and fold: channels, posts, members and reads in the private log, with `talk_*` tools. Mac and PC.
2. Talk page on the desktop. Threads move from Home's panel into it, and the MESSAGE cards become DMs.
3. "Agents answer now" plus next-turn reading in the plugin's wake text, which fixes #378 (closes irori#6).
4. Spaces: the space log replaces the per-board log, existing shared boards migrate, and settings move to the space.
5. Profile: name, colour (#385) and avatar, and the space switcher.
6. Person↔person DMs over two-person spaces.
7. The phone's Talk tab and notifications.
8. Release, with screenshots on the card.

## 10. Friends, not board invites (card #517, proposed 2026-11-05)

*Waiting on Stephen's go on #517 before building. Amends decisions 32 and 35 B4 and §5 above; keeps decision 41.*

Stephen (#515) expected friends to replace board invites: invite a person once, then pick them when you make a board. Today the Friends list is only the people you share a board with, "+ Invite a friend" opens Boards, and New board always makes a private board, so every new shared board needs a fresh invite.

### 10.1 The friend invite

- **One link and five words, tied to no board.** Profile › "+ Invite a friend" makes a friend invite. The screens, the words, the fingerprints and decision 41's wording are today's invite's; only the text changes ("Become friends with Alice"). The link is still pasted: there's no `irori-invite:` link handler, so the paste field moves to the profile ("Got a friend link?") and Boards loses its "Join with an invite" tab.
- **It makes the pair's friend space.** Opening the invite makes a hidden two-person space on the inviter (an ordinary space with a generated name and tag), then runs the existing invite on it unchanged. If the invite is cancelled, fails or expires, the empty space is forgotten.
- **The marker.** `board.created` and the private `board.joined` get an optional `kind: "friend"` field. Older readers ignore it. `BoardSummary` carries it to the window.
- **Both are owners.** Only an owner's `board.member.added` counts, so the invite's admit writes the friend as an owner of a friend space. Either friend can then make boards with the other.
- **Hidden where spaces are listed**: the space switcher, the profile's Spaces column, sidebar pins of the space itself, the `taken` tag list and share targets skip friend spaces. Boards made *inside* a friend space show like any other board.
- **Talk.** The friend space is the hidden two-person space §5 puts DMs on. Moving person↔person DMs onto it stays build-order step 6, a card of its own.

### 10.2 Who's a friend

A friend is a person you share any space with: your friend spaces and the shared boards you already have. That keeps today's collaborators in the Friends list and pickable, so nobody has to re-invite people they already work with. (Alternative, for Stephen: only people you've friend-invited.)

### 10.3 New board: "Who's on it?"

New board asks **Who's on it?**: *Just me*, or one or more friends.

- **Just me** makes a private board, as today.
- **Friends picked**: the board goes into the space whose active members are exactly you and those friends. With one friend, that's your friend space, so the board is a `space.board.created` row in it (#392's path): nothing new is sent, no invite runs, and it reaches the friend with the space. Among several matches, the friend space wins, then a space you own.
- **No matching space** (two or more friends together for the first time, or a friend you only share an old board with): Irori makes a new space for exactly those people, with no invite, by an **offer** over a space you already share with each friend:
  1. The maker creates the space and sends each friend a sealed `space.offer {spaceId, name, tag, maker pin}` on the connection of a space they share, sealed like the board secret frame.
  2. The friend's device answers with a **fresh pin**: its current key and epoch, with itself as the root device. The pin must come from the friend: a pin copied from another space names the device that joined that space, which may be off or revoked, and then no device of the friend could ever certify into the new space.
  3. The maker checks the offerer's key against the one pinned in the shared space, writes `board.member.added` for that pin, and sends the space's rows and secret as today's invite bundle does. The friend joins it as `bundle::join` does (minus the ticket checks), returns its device cert and profile rows, and both sides open the space.

  It is today's invite steps 5–6 over a connection that is already authenticated both ways, so it needs no words. A friend whose devices are all offline gets it when one comes online; until then the board shows them as "invited, not yet joined".
- Adding a board to an existing space still waits until every member device reads spaces (`space_not_ready`), as #392 does.

### 10.4 Board invites leave the normal UI

Friend invites are the only invites people see. The board settings' "Invite someone", the Boards dialog's "Join with an invite" tab, Settings › People and invites' per-board "Invite…" and join field, and the texts that send people to board invites ("join one with an invite", "Coming back needs a new invite", "Invite someone from its board page") go or are reworded. The Core, Tauri and agent board invite stay, for tests, agents and old links.

### 10.5 Build order

1. Friend invite and friend space, both owners, the Friends list from real friends, the profile's invite and paste field.
2. Who's on it? with one friend into the friend space. This alone meets #517's done-when.
3. Board invites out of the UI. A two-person test in `crates/core/tests/five_cores.rs`: friend invite → wait for `space_not_ready` → New board with that friend → the friend gets it with no invite.
4. The offer for new spaces with several friends. It's the one new protocol piece and goes last, so it can become its own card if wanted.
