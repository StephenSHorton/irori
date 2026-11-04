# Work and personal: what keeps them apart

*2026-11-04, card #477. Stephen has a work computer running its own Irori: work cards, background agents, his work Claude subscription. He hasn't linked it to his personal devices (Mac, PC, phone) because nothing has shown that the split holds: personal agents must never take or run work cards, and work agents must never run personal ones. This page says what holds today and what doesn't, from the code on main @ e89fecc5. Then it proposes what to build. No product code changed; the tests are in §6.*

## 1. The short answer

- **Irori has no idea of "work" or of a Claude subscription.** A spawn runs `claude` from PATH with the app's inherited environment (`crates/window/src/spawn.rs:129-135`, `roster.rs:455-470`), so it uses whatever login that machine has. Nothing ties a card, a board, a space or a device to an account.
- **On one person's linked devices, the only thing that keeps them apart is where a card was made.** A card is spawned, and handed out, only by the device its `created_by` key names. That is a rule about where the card was typed, not about what it's for. Everything else that gives an agent work (assign, claim, asks, threads, review requests, mentions) crosses devices freely.
- **Linking as a device copies everything both ways.** The whole private log replicates to every device of the person. Only `policy.set` and `member.joined` stay on one device (`crates/store/src/kinds.rs:94-97`). Every work card's title, objective, comments and repo paths would land on the personal disks, and every personal card on the work computer. No rule about spawning or hand-outs can undo that.
- **Joining as another person is the only way that works today.** The work Irori is then a second person. Its private board never leaves it, and its spawns stay on its devices and its login. But a shared space between the two people still leaks in a few places: room asks, quiet threads, reviews, and assign or claim (§3). Cards on a space never get background agents at all.

## 2. Linking the work computer as another device

### 2.1 What it shares

Everything in the private log, on every device: cards, comments, steps, blockers and leases; `task.spawn` and `task.spawned`; `spawn.set` and `agent.offer`; presence and the roster; `repo.seen` (where each repo is cloned); and the person's `receive`, `autonomy` and `threads` settings. A `board.joined` row also makes every device of the person open and sync every shared board and space the person is on (`crates/core/src/service/boards_own.rs:283-380`).

### 2.2 What holds

| Rule | Where | Proved by |
|---|---|---|
| A card is spawned only by the device its `created_by` names (`user:<device>` for the person, `<device>/<session>` for an agent). The request always names that device, and only it acts on the request. | `hands_out`, `crates/coord/src/board/offers.rs:496-501`; `cards_to_spawn`, `crates/coord/src/board/spawn.rs:464-490`; `spawn_for` writes `device: me`, `crates/core/src/service/spawn.rs:859` | **new** `separation_test::a_card_made_on_one_linked_device_never_spawns_on_another` |
| The hand-out sweep for an unheld card also runs only on that device. | `sweep_offers`, `offers.rs:569-577` | **new** `separation_test::a_card_made_on_one_linked_device_is_never_handed_out_by_another` |
| A spawn needs this device's checkout of the card's repo. Without one it is refused, with the reason on the card, and no other device is tried. | `spawn_folder` and `NoFolder`, `core/src/service/spawn.rs:874-886, 946-966` | **new** `spawn::a_card_whose_repo_isnt_on_this_device_is_refused_not_spawned` |
| "Not done", a blocker answer, a crash respawn and the restart cleanup all go back through the same rule, so they stay on the card's own device. | `back_on_board`, `cards.rs:1395-1430`; `spawn.rs:1064-1078` | `spawn_paths.rs`, `spawn_test.rs` (one device) |
| A spawned agent is only handed its own card and that card's subtasks. | `may_hand_to`, `coord/src/board/spawn.rs:145-157`; `check_free`, `offers.rs:159-175` | `spawn_test::a_spawned_agent_is_never_offered_a_second_card` |
| A terminal agent gets hand-outs only with **Offer it cards** on, which is off by default. | `core/src/service.rs:1065-1075` | `offer_opt_in_test` |
| With the private board's spawning switch on (the app's default), no card is handed to idle agents at all. | `offers.rs:365-372, 569-577` | `offer_opt_in_test::a_spawning_board_never_hands_a_new_card_to_an_idle_agent` |
| Revive asks the person first, then runs on the host's own login. | `crates/coord/src/approvals.rs:533-597` | |

So **if every work card is made on the work computer and every personal card on a personal device, the background agents never cross.** That holds as long as nobody turns on Offer it cards, nobody assigns across, and nobody posts asks.

### 2.3 What doesn't hold

- **A work card typed on the Mac runs on the Mac.** It is `user:<mac>`, so the Mac spawns it on the personal login, if it has the repo cloned and trusted. The same goes for subtasks from a split done on the Mac (`subtasks.rs:360-377`) and for a main-red card a personal agent reports (`main_red.rs:162-173`). The reverse holds too.
- **Repo paths aren't per device.** `repo.seen` carries no device (`crates/capture/src/rows.rs:598-603`), so `repo_roots` can return the other machine's path (`capture/src/service.rs:693-703`). On the Mac a work card then fails as "not trusted" rather than "no checkout". If that path happens to exist and is trusted, it runs. On the work computer, a personal path can sort first and make a real work spawn fail.
- **One spawn setting for every device.** The switch, model, effort and permission mode are a single replicated `spawn.set` row (`core/src/service/spawn.rs:139-182`). Only the cap is per device. Turning spawning off on any device turns it off everywhere, and that brings back the cross-device hand-outs below.
- **Offer it cards can be set from the other side.** `agent.offer` replicates and is accepted from any of the person's devices (`spawn.rs:581-590`), so the Mac can turn it on for a work agent.
- **Hand-outs pick from every device.** With the switch off, the pool is the whole roster (`core/src/service.rs:3176-3188`). The repo filter applies only when both the card and the agent have a repo (`offers.rs:256-258`), and the machine rank only sorts (`offers.rs:738-758`). So an opted-in personal agent sitting in `~` can be handed a work card. `offers_test::a_card_with_no_machine_signal_still_goes_to_the_one_idle_longest` shows a card going to the other machine.
- **Asks and threads cross whether or not spawning is on.** An ask can be owned by any of the person's devices (`ask_owner`, `asks.rs:996-1039`), and `free_agents` has no device filter (`asks.rs:1051-1100`). An ask that names agents skips the Offer it cards check. `ask_test::test_the_computer_hands_a_phone_ask_out_and_on` shows a phone ask handed out by a computer.
- **Deliberate moves have no device check.**
  - `board_assign` puts any card on any agent and wakes it. The fold adds the holder without `check_free`, so a spawned agent can be put on a second card (`tasks.rs:296-329`, `fold.rs:676-679`).
  - `claim_task` has no device check either (`tasks.rs:259-293`).
  - A private review request wakes any agent (`review.rs:301-345`).
  - Mentions wake any agent (`mention.rs:33-57`), and a thread naming `#N` reaches that card's holders (`threads.rs:88-116`).
- **The planned cross-device spawn would make it worse.** `spawned-agents.md` §2 and §9 (slice 11, not built) have the card's device pick another device that has the repo and start the agent there. Built as written, a card made on the Mac could start on the work computer, and the reverse.

Fixing all of that still leaves every work card's text on the personal devices (§1).

## 3. Joining as another person, on one shared space

The work Irori, never paired, has its own person: a fresh data dir makes its own `person.json` (`crates/identity/src/person.rs`). The personal side invites it to a space, and the invite only refuses a guest who is the same person (`crates/invite/src/flow.rs:441`).

### 3.1 What holds

- **The work private board never goes to the space.** Private rows are never written to a board log (`kinds.rs:278`). A card reaches the space only when:
  - an agent creates it there with `board=`, under that person's autonomy setting;
  - someone uses Share to board;
  - an agent adds a subtask to a card already on the space;
  - someone posts a room ask there.
- **Private spawns stay on the work devices and the work login**, since §2.2 applies inside each person.
- **Open cards on the space go only to their author's agents (0.11.1).** Only the device that made the card hands it out, and only to agents of the card's person (`offers.rs:496-525`). Tested by `boards2_test::open_cards_go_to_the_authors_agents` and `a_hand_out_from_another_device_counts_nowhere`, and through two Cores by `bridge2::an_open_card_never_reaches_the_other_persons_agents`.
- **Another person can't start an agent for your card.**
  - A board log refuses `task.spawn`, `task.spawned`, `spawn.set` and `agent.offer` outright, on append and on sync (`crates/store/src/kinds.rs:167-181`, `store/src/board.rs:254-258`).
  - Behind that, the fold counts a spawn row on a board only from a device of the card's author (`spawn_writer_ok`, `coord/src/board/spawn.rs:170-197`).
  - The step "spawn" mark that asks for helper agents is refused from another person (`boards2_test::another_persons_spawn_mark_on_a_card_is_refused`).
  - The **new** `boards2_test::another_persons_card_is_never_spawned_on_your_side` turns spawning on for both sides of a space. It checks that each side lists only its own person's card, and that a spawn request for the other person's card is refused from the sweep, from an agent, and by sync.
- **A merge desk can't live on a shared board** (`desk.rs:289-292`).

### 3.2 What doesn't hold

- **Cards on a space never get a background agent.** Only the private board's coord has the spawning switch; board coords are built without it (`core/src/service/boards.rs:1310-1413`), and board logs refuse spawn rows (above). A work card put on the space is served only by an author's terminal agent with Offer it cards on, or by an agent told to take it.
- **Room asks go to every member's free agents** (`free_agents`, `asks.rs:1051-1100`). From the asking side, the board's offers check is true for any other member's agent (`boards.rs:1389-1403`). Tested as intended by `bridge2::a_board_ask_reaches_the_other_persons_free_agents`.
- **Quiet threads**: with "Agents answer now" on, a thread nobody answers in 45 s goes to one idle agent of any member (`threads.rs:12-34`).
- **Reviews cross people on purpose.** With no reviewer named, a review goes to a free agent of *another* person, and naming your own is refused (`review.rs:183-202, 239-243`). A work PR would be reviewed by a personal agent.
- **Assign and claim cross.** Any agent may assign any card on the space to the other person's agent (`tasks.rs:295-330`). An agent told to take a card can claim one of the other person's cards (`tasks.rs:259-294`).
- **Only the sender checks the author rule.** The receiving device delivers an open-card hand-out, ask or review to its agent without re-checking the person or Offer it cards (`core/src/service/bridge.rs:322-522`). Both sides on 0.11.1 or later is fine. An older build on one side would hand out across.
- **Subtasks change the author.** A personal agent's subtask on a work card is the personal person's card, and it goes to personal agents.

### 3.3 What still crosses, by design

Shared cards and their threads; comments and mentions (bridged per the receive setting); Talk channels made in the space; board notes; mirrored leases; and a reduced card for each agent (name, kind, status).

## 4. What spaces (#392) scope today

A space is a shared board (spaceId = boardId, `spaces.md` §1). Its three settings are the person's own rows in their private log, keyed by the space's id. They apply to all of that person's devices, never to one device, and only the person can set them (`crates/inbox/src/bridge.rs:316-390`).

- **Receive** (`receive.set`) says how other people's rows reach your agents: wake now, or wait for the next turn, and which ones are held for you.
  - Open (the default): nothing is held.
  - Ask me: wakes, assignments and lease requests are held.
  - Closed: rows arrive at the next turn, and only rows to sessions that bypass the inbox are held.
  - It never drops a row, and it doesn't change who is in a hand-out. It also falls back to a person-wide setting.
- **Autonomy** (`autonomy.set`) says what your agents may do on the space. Under Ask me, `board_create` becomes a request. Sharing a private card is refused under Ask me and done straight away under Autonomous (`crates/tools/src/shared.rs:428-520`).
- **Threads** (`threads.set`): Private puts the threads you start into your own log, so only your agents hear them (`crates/window/src/threads.rs:154-291`).

None of them scopes devices, subscriptions or spawning. A space today says who sees what, not who runs what.

## 5. Options

**A. Separate people, a "keep agents apart" switch on the space, and space cards spawn on their author's devices.** *(recommended)*
- The work Irori joins as another person (no linking).
- A per-space switch, Keep agents apart, makes everything that hands out work go to the author's own agents only, as 0.11.1 did for open cards. That covers room asks, quiet threads, unnamed reviews, cross-person assign and claim, and subtasks.
- The receiving device re-checks the author rule, so it isn't up to the sender's version.
- Cards on a space then spawn the way private cards do: on the device that made them, with that person's spawn setting. So work cards on the space get work background agents.
- Comments, mentions as notifications, Talk and notes still cross, so he sees and talks about both.
- Medium. It needs one setting row, filters in `free_agents`, `open_reviewer`, `assign` and `claim_task`, and checks in the bridge's plans. The board coord gets the spawning switch.
- The spawn rows (`task.spawn`, `task.spawned`) also have to be let into board logs. An older Irori refuses them, and one refused row stalls that writer's whole stream on the older peer. So they go behind a hello gate, as `space.board.*` did (`spaces.md` §4).

**B. Same as A without space spawning.**
- Work cards stay on the work private board, where they already spawn. The space holds only what he wants to see from his personal devices: shared cards that work agents take only when told.
- Small, but work cards on the space don't move by themselves.

**C. Link as a device and pin each device to spaces.**
- Cards carry a space, each device gets a list of spaces it may run, and spawn, hand-out, ask, assign, claim and review all filter by it.
- The spawn setting and Offer it cards become per device, and `repo.seen` gains a device.
- Large, and every work card's text is still copied to the personal devices unless the sync itself learns to filter by space, which is a much larger change to the mesh.

**D. Nothing to build.**
- He joins as another person and sets the space to Receive: Closed and Threads: Private.
- He posts no room asks and names reviewers.
- Work cards stay on the work private board.
- The leaks in §3.2 stay; they are avoided by habit, not by the code.

## 6. Tests

New, on branch `worktree-477-work-personal-separation`:

- `crates/coord/tests/separation_test.rs`:
  - `a_card_made_on_one_linked_device_never_spawns_on_another`: in both directions, for a card the person typed and one an agent posted, and again after a replay.
  - `a_card_made_on_one_linked_device_is_never_handed_out_by_another`: with free agents present and repeated sweeps.
  - Both fail when `hands_out` is forced true (checked by hand).
- `crates/coord/tests/boards2_test.rs`, `another_persons_card_is_never_spawned_on_your_side`: on a space with spawning on for both sides, each lists only its own person's card. A spawn request for the other person's card is refused from the sweep, from an agent and by sync, so it counts on neither side. It fails when the store lets `task.spawn` into board logs (checked by hand).
- `crates/core/tests/spawn.rs`, `a_card_whose_repo_isnt_on_this_device_is_refused_not_spawned`: refused here with "no checkout of …", nothing started, the card left unheld.

Run them with `cargo test -p irori-coord --test separation_test --test boards2_test` and `cargo test -p irori-core --test spawn`.

The breaks are not locked in by new tests. The existing tests named in §2.3 and §3.2 already show them.
