# Work and personal: what keeps them apart

*2026-11-04, card #477. Stephen has a work computer running its own Irori: work cards, background agents, his work Claude subscription. He hasn't linked it to his personal devices (Mac, PC, phone) because nothing has shown that the split holds: personal agents must never take or run work cards, and work agents must never run personal ones. This page says what holds today and what doesn't, from the code on main @ e89fecc5. Then it proposes what to build. No product code changed; the tests are in §6.*

## 1. The short answer

- **Irori has no idea of "work" or of a Claude subscription.** A spawn runs `claude` from PATH with the app's inherited environment (`crates/window/src/spawn.rs:129-135`, `roster.rs:455-470`), so it uses whatever login that machine has. Nothing ties a card, a board, a space or a device to an account.
- **On one person's linked devices, the only thing that keeps them apart is where a card was made.** A card is spawned, and handed out, only by the device its `created_by` key names. That is a rule about where the card was typed, not about what it's for. Everything else that gives an agent work (assign, claim, asks, threads, review requests, mentions) crosses devices freely.
- **Linking as a device copies everything both ways.** The whole private log replicates to every device of the person. Only `policy.set`, `member.joined`, `board.cardkey`, `board.pin` and `board.forgotten` stay on one device (`crates/store/src/kinds.rs:92-95, 305-309`). Every work card's title, objective, comments and repo paths would land on the personal disks, and every personal card on the work computer. No rule about spawning or hand-outs can undo that.
- **Joining as another person is the only way that works today.** The work Irori is then a second person. Its private board never leaves it, and its spawns stay on its devices and its login. But a shared space between the two people still leaks in a few places: room asks, quiet threads, reviews, and assign or claim (§3). Cards on a space never get background agents at all.

## 2. Linking the work computer as another device

### 2.1 What it shares

Everything in the private log, on every device: cards, comments, steps, blockers and leases; `task.spawn` and `task.spawned`; `spawn.set` and `agent.offer`; presence and the roster; `repo.seen` (where each repo is cloned); and the person's `receive`, `autonomy` and `threads` settings. A `board.joined` row also makes every device of the person open and sync every shared board and space the person is on (`crates/core/src/service/boards_own.rs:283-380`).

### 2.2 What holds

| Rule | Where | Proved by |
|---|---|---|
| A card is spawned only by the device its `created_by` names (`user:<device>` for the person, `<device>/<session>` for an agent). The request always names that device, and only it acts on the request. | `hands_out`, `crates/coord/src/board/offers.rs:496-501`; `cards_to_spawn`, `crates/coord/src/board/spawn.rs:464-490`; `spawn_for` writes `device: me`, `crates/core/src/service/spawn.rs:859` | **new** `separation_test::a_card_made_on_one_linked_device_never_spawns_on_another` |
| The hand-out sweep for an unheld card also runs only on that device. | `sweep_offers`, `offers.rs:569-577` | **new** `separation_test::a_card_made_on_one_linked_device_is_never_handed_out_by_another` |
| A spawn needs a known checkout of the card's repo. When no linked device has seen the repo it is refused, with the reason on the card, and no other device is tried. A repo seen only on another linked device is refused the same way (fixed in #493, §2.3). | `spawn_folder` and `NoFolder`, `core/src/service/spawn.rs`; the `repo.seen` fold, `capture/src/service.rs` | **new** `spawn::a_card_whose_repo_isnt_on_this_device_is_refused_not_spawned`, `spawn::a_repo_seen_only_on_a_peer_is_refused_not_spawned_there` |
| "Not done", a blocker answer, a crash respawn and the restart cleanup all go back through the same rule, so they stay on the card's own device. | `back_on_board`, `cards.rs:1395-1430`; `spawn.rs:1064-1078` | `spawn_paths.rs`, `spawn_test.rs` (one device) |
| A spawned agent is only handed its own card and that card's subtasks. | `may_hand_to`, `coord/src/board/spawn.rs:145-157`; `check_free`, `offers.rs:159-175` | `spawn_test::a_spawned_agent_is_never_offered_a_second_card` |
| A terminal agent gets hand-outs only with **Offer it cards** on, which is off by default. | `core/src/service.rs:1065-1075` | `offer_opt_in_test` |
| With the private board's spawning switch on (the app's default), no card is handed to idle agents at all. | `offers.rs:365-372, 569-577` | `offer_opt_in_test::a_spawning_board_never_hands_a_new_card_to_an_idle_agent` |
| Revive asks the person first, then runs on the host's own login. | `crates/coord/src/approvals.rs:533-597` | |

So **if every work card is made on the work computer and every personal card on a personal device, the background agents never cross.** That holds as long as nobody turns on Offer it cards, nobody assigns across, and nobody posts asks.

### 2.3 What doesn't hold

- **A work card typed on the Mac runs on the Mac.** It is `user:<mac>`, so the Mac spawns it on the personal login, if it has the repo cloned and trusted. The same goes for subtasks from a split done on the Mac (`subtasks.rs:360-377`) and for a main-red card a personal agent reports (`main_red.rs:162-173`). The reverse holds too.
- ~~**Repo paths aren't per device.**~~ **Fixed in #493:** the fold keeps only this device's `repo.seen` rows in `repo_roots` and the rest in `roots_elsewhere`, and `spawn_folder` takes the first of them that is a directory, so a repo seen only on another device is refused as "no checkout of …" and the #467 retry guard holds (`capture_service::a_peers_repo_seen_is_not_a_checkout_here`, `spawn::a_repo_seen_only_on_a_peer_is_refused_not_spawned_there`). What it was: `repo.seen` carries no device (`crates/capture/src/rows.rs:598-603`), so `repo_roots` can return the other machine's path (`Capture::repo_roots`, `capture/src/service.rs`). On the Mac a work card then fails as "not trusted" rather than "no checkout". If that path happens to exist and is trusted, it runs. On the work computer, a personal path can sort first and make a real work spawn fail.
- ~~**One spawn setting for every device.**~~ **Fixed (#492).** The switch, model, effort and permission mode were a single replicated `spawn.set` row (`core/src/service/spawn.rs:139-182`), so turning spawning off on any device turned it off everywhere, and that brought back the cross-device hand-outs below. Now each device counts only the `spawn.set` rows it signed (`Event.device_id`), and its rows name it (`device`). A device with no row of its own yet takes the newest row from any device once (the one an older build ran on) and writes it as its own, so the upgrade changes nothing; after that, rows from other devices, old builds' included, don't change it. A device that has never had a setting also starts from another device's this way, once. Tests: `crates/core/tests/it/spawn_settings_devices.rs`.
- ~~**Offer it cards can be set from the other side.**~~ **Fixed (#492).** `agent.offer` replicated and was accepted from any of the person's devices (`spawn.rs:581-590`), so the Mac could turn it on for a work agent. Now an `agent.offer` row counts only when the agent's own device signed it, `set_agent_offer` refuses another device's agent, and the app shows the switch read-only for an agent on another device ("It runs on …: turn this on or off there"). An offer an older build wrote for another device's agent no longer counts after the upgrade. Tests: `crates/core/tests/it/spawn_settings_devices.rs`.
- **Hand-outs pick from every device.** With the switch off, the pool is the whole roster (`ask_agents`, `core/src/service.rs:3229-3240`, which feeds asks too). The repo filter applies only when both the card and the agent have a repo (`offers.rs:256-258`), and the machine rank only sorts (`offers.rs:738-758`). So an opted-in personal agent sitting in `~` can be handed a work card. `offers_test::a_card_with_no_machine_signal_still_goes_to_the_one_idle_longest` shows a card going to the other machine.
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
  - A board log refuses `spawn.set` and `agent.offer` outright, on append and on sync (`crates/store/src/kinds.rs`, `store/src/board.rs`). Since #455 it takes `task.spawn` and `task.spawned`, behind the hello gate, but a row counts only from the card author's devices (`Fold::spawn_writer_ok`), and a device starts only its own person's cards (`Fold::mine_to_start`).
  - Behind that, the fold counts a spawn row on a board only from a device of the card's author (`spawn_writer_ok`, `coord/src/board/spawn.rs:170-197`).
  - The step "spawn" mark that asks for helper agents is refused from another person (`boards2_test::another_persons_spawn_mark_on_a_card_is_refused`).
  - The **new** `boards2_test::another_persons_card_is_never_spawned_on_your_side` turns spawning on for both sides of a space. It checks that each side lists only its own person's card, and that a spawn request for the other person's card is refused from the sweep, from an agent, and by sync.
- **A merge desk can't live on a shared board** (`desk.rs:289-292`).

### 3.2 What doesn't hold

- ~~**Cards on a space never get a background agent.**~~ **Changed by #455:** a person's own cards on a board whose switch is on get an agent on the device that made them (spawned-agents.md part 11). Another member's card is never started on your side (above).
- **Room asks go to every member's free agents** (`free_agents`, `asks.rs:1051-1100`). From the asking side, the board's offers check is true for any other member's agent (`boards.rs:1389-1403`). Tested as intended by `bridge2::a_board_ask_reaches_the_other_persons_free_agents`.
- **Threads**: with "Agents answer now" on, a thread goes out at once (card #512, no 45 s wait any more): an idle agent of any member with Offer it cards on takes it first; with none free, the poster's device starts the thread's own agent (`thread_sweep` in `crates/coord/src/board/threads.rs`; `sweep_board_spawns`).
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

- `crates/coord/tests/it/separation_test.rs`:
  - `a_card_made_on_one_linked_device_never_spawns_on_another`: in both directions, for a card the person typed and one an agent posted, and again after a replay.
  - `a_card_made_on_one_linked_device_is_never_handed_out_by_another`: with free agents present and repeated sweeps.
  - Both fail when `hands_out` is forced true (checked by hand).
- `crates/coord/tests/it/boards2_test.rs`, `another_persons_card_is_never_spawned_on_your_side`: on a space with spawning on for both sides, each lists only its own person's card. Since #455 the board log takes the row, but a request for the other person's card, from the sweep or from an agent, counts on neither side and no device is asked to start it.
- `crates/core/tests/it/spawn.rs`, `a_card_whose_repo_isnt_on_this_device_is_refused_not_spawned`: refused here with "no checkout of …", nothing started, the card left unheld. It covers a repo no device has seen. A repo seen only on another linked device: `a_repo_seen_only_on_a_peer_is_refused_not_spawned_there`, refused as "no checkout of …" though the peer's path exists here and is trusted, nothing started, and past the retry wait no more `task.spawn` rows (#493, §7.4). In capture, `capture_service::a_peers_repo_seen_is_not_a_checkout_here`: a peer's row leaves `repo_roots` and `repos()` empty, and this device's own row then shows.

Run them with `cargo test -p irori-coord --test it -- separation_test:: boards2_test::` and `cargo test -p irori-core --test it -- --exact` with the names under `spawn::` (a plain `spawn::` filter also runs `ask_spawn::`).

The breaks are not locked in by new tests. The existing tests named in §2.3 and §3.2 already show them.

## 7. Stephen chose C (2026-11-04): what it means in the code

*Checked against main @ bdb613d9. Line numbers below are current there.*

### 7.1 Two ways to read "cards carry a space"

- **(i) The Work space is a real space: its own log, opened only by pinned devices.** A one-person space can be made today (`create_board`, `crates/core/src/service/boards.rs:961-1047`, needs no guest). The card's space is the log it lives in. A device pinned out never opens that log or holds its secret, so **work card text never reaches the personal disks**, which is better than §5 C promised. The cost: cards on a space must spawn, which is most of A's machinery. The board coord gets the spawning switch, and `task.spawn` / `task.spawned` are let into board logs behind a hello gate (`kinds.rs:167-171`, gate like `SPACES_PROTOCOL`, `boardmesh.rs:569`).
- **(ii) A space label on private-log cards, and devices filter on it.** This reuses the private spawn path, so no gate is needed. But every card's text still replicates everywhere. Every creation path (split, main_red, share, tools, UI, phone) has to copy the label, and any path that forgets drops the card into "any device".

Both code readers leaned (i). The pick is blocker b2 on #477.

### 7.2 What C's option text did not name

- **Work content isn't only cards.** `capture.turn`, `capture.edit`, `capture.summary` (work agents' full turns: prompts, tool calls, edits), `commit.seen`, `pr.ledger`, `desk.*`, notifications, `agent.status`, asks, Talk and notes are private-log rows keyed by repo or agent, not by card (`kinds.rs:183-191`, `capture/src/rows.rs:52-61`). Under (i) or (ii) they replicate whole once he links.
- **Agents can read across subscriptions.** The work computer holds the private log, so a work agent can read every personal card, note and transcript through `board`, `show`, `search`, `overview` and `timeline` (`crates/tools/src/explore.rs`, `capturetools.rs`). A personal agent can read work `capture.*` turns the same way. That sends one side's content to the other side's Claude subscription, which is worse than copies on disk.
- **Filtering the private log's sync is the wrong layer.** Each writer's stream must have no gaps (`store/src/db.rs:1355-1359`), and a changed payload at a held `(device, seq)` is a fork that stops that device for good (`db.rs:1306-1322`). A separate log per space is the mechanism that already exists.
- **Who writes the pins.** "No pin row means every space", with each device writing its own row, means the work computer can't keep the Mac out of Work. Under (i) the device list has to belong to the space (a `devices` field written by the creating device), and board secrets go only to listed devices (`send_board_secrets_of`, `crates/mesh/src/ownboards.rs:99-130`). The receiving side drops a secret it isn't listed for, so the rule doesn't rest on the sender's version.
- **Holding a space and running it are two things.** The phone may open Work so he can read it, but it can't spawn. A Work card typed on the phone needs a run device, so a pin is two levels: hold, and run.
- **The default space.** `board_create` with no `board=` and the app's new card both write to the private log (`crates/tools/src/board.rs:826-840`, `crates/window/src/new_card.rs:41-49`). On the work computer the default has to be Work, for agents and for the app.
- **Existing work cards.** Pairing keeps the device's stream (`crates/pair/src/pairer.rs:1184-1235`), so today's work cards in the work computer's private log replicate the moment it links. Either move them into the Work space and scrub the private copies, or start the work Irori from a fresh data dir.
- **Mixed versions.** An older Irori ignores pins and opens every space from `board.joined`. Separation holds only once every device runs the new build, so pinning should be refused until every own device reports support.
- **Still no Claude account.** The run pin is a stand-in for the subscription. Nothing checks the machine's `claude` login.

### 7.3 Corrections to the analysis by the code readers

- The receive-side re-check for linked devices belongs in the inbox (`host_hold`, `crates/inbox/src/mesh.rs:128`; `hold()`, `policy.rs:73`). `bridge.rs` is cross-person only (`bridge.rs:131-137`).
- New private-log kinds and fields need no hello gate: the private log stores unknown kinds and fields (`kinds.rs:20-23`, `db.rs:1185-1200`). Only spawn rows entering board logs need one.

### 7.4 Two slices that hold under (i), (ii) or A

- **Per-device spawn setting and Offer it cards.** Built in #492. Keep `spawn.set` and `agent.offer` as kinds, and count a row only for the device that signed it (`Event.device_id`). `load_spawn` (`core/src/service/spawn.rs:565-607`) takes this device's id. On the first load it falls back to the newest row from any device and seeds an own-device row, so the upgrade changes nothing. `agent.offer` counts only from the agent's own device, and `set_agent_offer` (`spawn.rs:741-770`) refuses another device's agent. The app's Offer toggle is read-only for remote agents. Don't flip either kind to Local: `db.rs:1193-1197` would refuse an older peer's replicated rows and stall its stream.
- **Per-device repo checkouts** (done in #493). The `repo.seen` fold (`KIND_REPO_SEEN` in `apply`, `capture/src/service.rs`) puts only `ev.device_id == me` rows into `repo_roots`, and keeps others in `roots_elsewhere`. There is no payload change and no migration of rows. One upgrade gap is closed in the repo pass: before #493, `note_root` wrote no row for a path a peer had already logged, so a device with a checkout at the same path as a peer's held only the peer's row. Once per start, the repo pass checks each peer root that this device holds no row for; if it is a repo here, `note_root` writes this device's own row with its own key (`capture_service::a_peers_root_that_is_a_checkout_here_too_becomes_ours_at_the_repo_pass`). `spawn_folder` (`core/src/service/spawn.rs`) takes the first root that is a directory. A repo seen only elsewhere then fails as `NoFolder`, which the #467 retry guard catches (today it fails as "not trusted", so that guard never engages). Risk: a device whose id changes treats its own old rows as foreign until capture sees the root again. Test gotcha: peer `repo.seen` rows fold only at replay, so insert them before `Capture::new`.

Spawning, hand-outs, asks, assign, claim, review, mentions and the sync scope wait on b2.

### 7.5 Stephen's answer on b2 (2026-11-04): who runs what, not who reads what

He doesn't mind personal devices reading work cards or the reverse. He wants agents told they're separate, and nothing to pick up work across that line unless asked. So the build is the smallest reading, (ii):

- **A side on every card, and a side list on every device.** Cards carry a `space` label (a new field on `task.created`; a subtask, split or main_red card takes its parent's). Each device has a replicated row naming the sides it runs, set on that device. With no row, a device runs everything, so nothing changes until he pins. A new card gets the side of the device it's typed on, both from the app and from `board_create`. No sync change and no hello gate (private log).
- **Automatic picks respect the side.** That covers spawn and hand-out (`hands_out`, `cards_to_spawn`, `free_for`), asks (`ask_owner`, `free_agents`), quiet threads and unnamed reviews: Irori only picks agents on a device that runs the card's side. The receiving device checks again in the inbox (`host_hold`, `policy.rs` `hold()`).
- **"Unless asked" stays open.** If the person assigns, starts an agent, or @-mentions an agent across the line, it goes through. An agent doing the same on its own (assign, claim, join) is refused with the reason, and the reason says to ask the person.
- **Agents are told.** A spawn brief and a session's Irori instructions say which side its device runs, and that cards on the other side are only read, never taken, unless the person asks.

Reads across stay as they are (§7.2's transcript and search points are accepted).
