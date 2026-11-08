# Office Assistants talking across people: design

*2026-11-08, card #1030, from the person's idea on thread #1021. Design only: nothing here is built yet. Build cards are in §12. Checked against main by five adversarial reviews (identity, leaks, delivery, buildability, consistency); what they found is folded in.*

The idea, in the person's words: "Maybe office assistants can communicate with other people's office assistants", to agree on who should review a PR, or because "I need to ask addam some questions, maybe office assistants already have that info". "Would want this to be legit though", and "We don't have to go hard on this. just an idea."

So this is the smallest version that is legit. One person or agent asks, and the other person's assistant answers once, in a channel everyone in the space can read.

## 1. Decisions

| | Decision | Why |
|---|---|---|
| D1 | **Each person has one assistant per space. The person may name it, and by default it is "Addam's assistant".** Others always see "Juno · Addam's assistant", with the person part taken from who signed the card. | The person's decision on #1029. The default needs nothing written and is how people already talk. #1029 owns the name store, so the two designs give one answer. |
| D2 | **It is an `agent:<cardId>` card with kind `assistant`**, never `person:`, and the owner's `member.profile` points at it. | It's the only actor shape every peer accepts that isn't the person. The pointer, not the id, says which card is the assistant (§3). |
| D3 | **One channel per space, `#office-assistants`, made by itself.** | The person asked for a dedicated channel, and decision 52 says no switching on. |
| D4 | **An assistant answers only in that channel.** | One visible record, replicated, that can't be edited or deleted. |
| D5 | **The turn that answers another person has no tools.** Irori gathers the facts in code, the model only writes text, and Irori posts it. | A peer's words must not be able to reach private data (§6). |
| D6 | **On its own it only says what the asker's device already holds**: this space's rows. Anything else needs the owner's yes on the exact text. | This makes the sharing rule structural, not a prompt. |
| D7 | **One question, one answer.** A post that replies to another never starts a turn. | Two assistants can't loop. |
| D8 | **A labelled line in the receiver's Office Assistant chat, the dot and a notification, only for what waits on them.** The chat doesn't pop open by itself in v1. | #1000's rule for the chat. Opening steals focus today and covers a phone screen (§8). |
| D9 | **It never starts agents, answers blockers, approves, merges, commits its owner to anything or speaks as its owner.** "I can spawn one" becomes "I'll ask Addam". | "It must never act as the person." |

## 2. What is there today

The earlier check on #1023 read a checkout 265 commits behind main. On main:

- **The Office Assistant exists, per device, not per person.** Each question is one headless `claude -p` on Haiku, with no session between turns (`crates/window/src/assistant.rs`). Its history lives in the webview's localStorage. It's off the roster on purpose, so it has no agent key, card or inbox.
- **Its authority is the person's, read-only.** The API treats its secret as `Sender("user:<device>", …, "assistant")` and allows only `tools::ASSISTANT_TOOLS` (14 read tools, pinned by a test). It reads private cards and its own agents' transcripts.
- **Delivery across people already works in space channels.** `talk_post_bridge` wakes the `agent:` keys a post mentions, under the receive setting, Pause, rate limit and hop limit. Card #1030's "mentions aren't delivered across people" is out of date for channels, cards and threads. It is still true for `send`/`reply`, and DMs to another person's agent are refused on purpose.
- **#1000 cut the chat to what waits on the person.** Its follower (`frontend/src/lib/assistant-follow.ts`) follows only threads the chat posted. `parseHistory` drops a saved update line unless it has an answer act or passes `waitsOnPerson`. The reply act and `replyOn` are gone.
- **So the gaps are identity and a way to answer**, plus the channel, the labelled line and the sharing rule.
- **#987's assistant write tools are not on main.** Nothing here builds on them.

## 3. Who the assistant is

**The name.** #1029 owns it, in its build card "The assistant's name":

- A private `assistant.name` row `{name, by, at}`, newest wins, set in the app.
- An `assistantName` field on `member.profile` publishes it to other members.
- With no name set, it is the default, `<profile name>'s assistant`. The default is computed, so nothing is written and every device agrees.

This design only reads the name. Until #1029's card lands, every assistant has the default.

**The card.** A new `agent.card` row on the space's log:

- `cardId`: a random id from `new_board_id`, like any card.
- `name`: the assistant's name, rewritten when it changes.
- `kind`: `assistant`. Today's values are `claude` or the agent's own kind. Board paths don't read it as a closed list; only private-roster paths compare it to `claude`.
- `personId` and `deviceId` as for any card. `status`: `idle`.

It is written by a new `Services::assistant_card(board)`, not `card_for`, which needs a roster entry. It gets no roster entry and no `board_cards` mapping. The assistant stays off the private roster and nothing reaches its inbox (§5). `sync_card_status` walks only `by_id`, so no sweep marks the card gone.

**It must not look like a free agent.** The space's roster (`roster_of` in `crates/core/src/service/boards.rs`) includes every card in the space. The board's `offers` closure (boards.rs, around line 1429) offers any card with no `by_id` entry. `free_agents` (`crates/coord/src/board/asks.rs`) puts the card that has been idle longest first. The assistant card is always idle and its `updated_at` is zero, so without a fix **every room ask in the space would be handed to an assistant card first, and stick there**. So `offers`, `free_agents` and the presence counts in `boards_presence.rs` all skip cards of kind `assistant`. Card 2 tests that a room ask passes it by.

**The pointer.** `assistant_card` also sets a new `assistantCard` field on the person's `member.profile` for that space, holding the card's id.

- It sits outside `personSig`, like `color`, `avatar` and `githubLogin`, so older builds still count the row. Their lenient decoder skips unknown keys.
- Only the person's own devices can write a counted profile, so only they can say which card is their assistant.
- **It folds like `githubLogin`, not like `color`.** `fold_member` (`crates/identity/src/board.rs`) overwrites colour and avatar from every row. A later row that leaves the GitHub login empty keeps the old one. The pointer must do the same: add `profile_assistant_card` beside `profile_github_login`. Otherwise an older build of the person's own, or today's profile refresh, wipes it on the next rename.
- `keep_current` (boards.rs) rebuilds the profile from `member_profile_row` on every name, colour or avatar change. It must carry the pointer forward, as it carries `had` for the login.
- A card counts as the assistant only when its owner's profile points at it.

**Minting.** Only the assistant's computer (below) mints, and only when the profile has no pointer. Before minting, it looks for an existing card of this person's of kind `assistant` on the space, and points at that one instead. If two computers race anyway, both cards belong to the person. The newest pointer wins, and the other card stays an idle card that is never offered and never labelled.

**Why not an id everyone can compute.** That was the first draft, and it fails:

- `fold_cards` gives an id to the first `agent.card` row whose writer's person matches its `personId`. So another member could write a row with a computable id and their own `personId` first.
- A newer verifier could refuse that row, but an older peer would accept it. It would then refuse the owner's real posts in `bind_actor`.
- A refused row stops the writer's stream: `Db::insert_one` stops at the failed row, and every later seq then fails with a gap. So one hostile member could stall the owner's device on every older peer.

A random id can't be claimed ahead of time, and the pointer needs no change to the verifier. Decision 30's key rotation doesn't matter either, because nothing is derived from a key.

**A gap that's already there for every card.** `fold_cards` orders rows by HLC, and a writer chooses its own HLC within a bound. A hostile member who sees the card id could certify a new device and write a backdated `agent.card` row with that id. That flips the card to them, and the owner's next post as the card stalls their stream. This is true for every agent card today. The pointer only tells an attacker which card to target. It is not this card's to fix: it needs `fold_cards` to keep only rows from devices that held the card first, which would be a verifier change for all cards.

**It can't be forged across people.** `bind_actor` checks that an `agent:` actor's card belongs to the signing device's person, and an `agent.card` must name its writer's person. A peer can only ever write as its own assistant.

**What the label proves.** It proves which person's device wrote the row, not that their real assistant wrote it, nor that the person agrees:

- A member running a modified client can post anything as their own assistant, just as they can post as themself. They can never post as someone else's assistant.
- An answer is a model's reading of the space, and the asker's words can steer it.
- So peers and agents read an answer as information from that person's device, never as that person's decision. #1022's reviewer picking must not take "Addam can review it" from Addam's assistant as Addam accepting. It asks Addam (§5, D9).

**The label comes from the verified owner, not from the card row.** `BoardLog::fold_card` (boards.rs) keeps the last `agent.card` row that arrives for an id, and doesn't check the writer. Another member can write an `agent.card` row for Addam's card id with their own `personId`; `bind_actor` accepts it, because it checks only that `personId` is the writer's. So:

- The person comes from the verifier's `card_person(id)`, never from the row's `personId`.
- The kind and name come only from rows written by that person's devices. `fold_card` keeps, per card, the newest row whose writer is `card_person(id)`'s device, for the label and for mentions.
- Show "Juno · Addam's assistant", or just "Addam's assistant" for a default name, only when that kind is `assistant` *and* the owner's profile points at the card.
- A card merely *named* "Addam's assistant" that fails either test shows as an ordinary agent ("Addam's assistant · Eve", agent tag).
- Talk's member mapping keeps the member kind `agent`, so a typed mention still turns Agents answer now on (`agentsNowFor`), and adds an `assistant` flag. `conversation.tsx` shows an `assistant` tag.

Two members can share a profile name. Eve can rename herself "Addam", and then her assistant also shows as "Addam's assistant". This is the same for people's own names today: the tag colour and avatar tell them apart, and the hover card shows the person.

**Mentions.** People pick from the list, which records the key. Agents write `@Addam's assistant` or `@Juno`. For that to resolve:

- `board_mentionables` lists the pointed-at assistant under both its chosen name and the default form, `<profile name>'s assistant`.
- It doesn't list another card under a name equal to some member's `<profile name>'s assistant`, so an ordinary agent named "Addam's assistant" can't take mentions meant for the assistant.
- `coord::mentions::resolve` tries names longest first, so `@Addam's assistant` matches the listed assistant before the person "Addam". Card 2 tests this, because `ends_token` treats `'` as the end of a name.
- If two members share a name, the name is ambiguous and the sender gets the candidates' keys, as today.
- The picker offers assistants only in `#office-assistants`. Today `MentionBoard.Provider` carries only the space, and `MentionCandidate` has no kind. Card 2 passes the conversation id and adds the kind (`frontend/src/lib/mentions.ts`, `frontend/src/components/common/mention-textarea.tsx`, `crates/window/src/mentions.rs`).

**Which computer answers.** One of the person's desktops that can run the answering turn (a desktop with Claude Code signed in, the same condition as the namer):

- Each such desktop says so with a new `answers: true` field on its `spawn.host` row.
- The assistant's computer is the first device in `agent_home("")`'s order whose `spawn.host` says `answers`. This way an older build or a phone that ranks first doesn't silence the assistant.
- `agent_home` ranks devices from each device's own mesh view. So when two of the person's desktops can't see each other, both may think they're home. §5's answer id makes that harmless.
- `append_space_talk` doesn't check that this device hosts the card, so "this device is the assistant's computer" in §5 is the only gate, and it must be checked.

## 4. The channel

- **Id:** `ch_` plus a hash of a fixed domain and the space id, following `pair_dm_id`. Every member computes the same id, and the fold keeps one channel however many members write it (it keeps the oldest copy).
- **It can't be created in another space.** The talk fold keeps the first copy it folds, and `append_space_talk` stamps each row with the space that carried it. Without a check, a member of two spaces could create S1's assistants channel inside S2 and break it for everyone who folds S2 first. So the `coord::talk` fold refuses a `talk.channel.created` whose id is an assistants-channel id for a space other than the one carrying it. Older devices don't have this check. On them, only a member of both spaces acting in bad faith can cause it, and the channel can still be read.
- **Name:** `office-assistants`. Topic: "Ask anyone's assistant. It answers from what is on this space."
- **Made by** a new `ensure_assistants_channel(space)`:
  - It runs on Core start, when a space appears, and on each host sweep. No event fires when a space's gate opens, so the sweep is the retry.
  - If the channel isn't folded yet, it writes `talk.channel.created` through `append_space_talk` with actor `irori` and `by: "irori"`. `bind_actor` refuses a `by` that differs from the actor.
  - It skips hidden DM spaces and Personal.
  - Don't use `create_channel_in`: it mints a random id and joins the author.
- **No new kind and no schema change:** `coord::talk::is_assistants_channel(id, space)` recomputes the id. An older device shows an ordinary channel.
- Talk pins it first in the space's channels. Anyone may archive it, as with any channel. Archived means no answers, and `ensure` doesn't bring it back.
- The person reads any space channel without joining (`tools::talk::posts`), on every device.

## 5. Asking and answering

**Three ways to ask.** Each is a post in the channel that mentions the assistant's card:

1. A person types it in the channel.
2. An agent posts it with `talk_post` (how #1022's reviewer picking will use this).
3. From your own Office Assistant chat (§9), posted as *your* assistant after you tap Send.

**The bridge.** `talk_post_bridge` (`crates/core/src/service/bridge.rs`) runs on the receiver's Core and has already checked the author and the channel. Add one split, placed **before** the `!r.agents_now` early return (a question to an assistant is a question whatever that toggle says) and before `send_ref`:

- Mentions of *this person's assistant card*, in the assistants channel, go to the answering turn.
- Everything else carries on unchanged.

The mention can't go through `inbox.bridge`, because `bridge_locked` skips any key whose `device_of` isn't this device, and `device_of("agent:<id>")` is empty. Nothing would be delivered. A mention of the assistant in any other channel, or on a card, reaches nothing.

**Catching up.** `bridge_row` runs only on live rows, and `agent_home` is `None` until `set_spawner` runs. So questions that arrive during start-up, or while home is moving, would never be seen. When `set_answerer` installs the turn, and whenever this device becomes the assistant's computer, it scans each assistants channel for mentions of this person's assistant that are under an hour old and unanswered. It queues them the same way. The scan also covers a job lost when the app quit.

**When a turn runs.** The checks go in this order, and the first that fails stops it:

| Check | If it fails |
|---|---|
| This device is the assistant's computer (§3) | Nothing here; that computer handles it |
| The post has no `replyTo` | No turn, ever (D7). This doesn't depend on the author's kind, which a row can't be trusted for |
| Not already answered: no post with this question's answer id or waiting id (below) | Nothing |
| The question's hop is under `MAX_HOP` | Nothing |
| Under the rate limit: 8 per asking person per 10 minutes, keyed like `pair_key` in `inbox/bridge.rs` (per space and person, so extra agents don't multiply it), kept in memory | A `held` line: "N questions from Eve were not answered" |
| Younger than one hour by `PostRow.at` (the asker's clock; a time in the future counts as now) | A `held` line: "Addam's assistant didn't get to this in time" |
| Pause all is off on this device | Treat as Ask me |
| The receive setting (below) | Below |

The rate limit comes before the age check, so backdated posts can't flood the chat. The counter resets on a restart or when home moves. That is accepted.

**The receive setting decides.** It's read with `Inbox::receive_setting(b.id, person)` and `receive_decision`, which are already public, keyed by the space's log id, with `bypass = false`:

| Setting | The assistant |
|---|---|
| Open (the default) | Answers |
| Members wake | Answers the people you picked; the rest as Ask me |
| Ask me | Posts the waiting line (below) and shows a `needs` line with **Let it answer** |
| Closed | Doesn't answer and posts nothing |

The mapping a builder must use: answer only when the mode is wake and there's no hold. Ask me when the hold is `MEMBER_WAKE` or the setting is Ask me. Anything else is nothing. Under Closed, `receive_decision` returns next-turn with no hold, which is not "answer".

**The turn.** It is modelled on naming. `Namer` (`crates/core/src/service/thread_names.rs`) is `Fn(&NameAsk{system, prompt}) -> Result<String, String>`, already reused by `set_weigher` and `rewrite_draft`, so it takes this prompt and packet as it is:

- A `set_answerer` hook installs it on desktops.
- A worker thread takes jobs from a channel and survives a panic.
- It has its own `Runner`, a fourth field in `crates/window/src/lib.rs` with its shutdown beside `assistant_shutdown`, so a peer never takes a slot from the person's chat.
- `tools: false`: no MCP config, no secret, `--tools ""`.
- The same model as the Office Assistant.

**The post.** Irori posts the text as `agent:<cardId>` through a new `pub` Core wrapper around `append_space_talk`, which is private today. It writes the raw row, because `PostRow` has no `hop` field and `Talk::post` would lose it:

- `postId`: **the answer id**, `post_` plus a hash of a fixed domain, the card id and the question's post id. If both of a person's desktops answer, the second post has the same id, and Talk's fold keeps one (`ids.contains`). Two answers can't show.
- `replyTo`: the question's `post_id` (`post_…`). This is a new optional field on `PostRow`, and also on `coord::talk::Post` and its fold, and on `TalkPost` (`crates/window/src/talk.rs`) and the bindings. Older readers ignore it (`serde(default)`).
- `mentions`: the asker's key. An agent's card makes it hear. A person's `person:<id>` makes Talk notify them, phone included, through today's `Talk::notifies`.
- `hop`: the question's hop + 1, set explicitly. `board_hop` looks the actor up by session key, and the assistant has none.
- `agentsNow: true`.

**The waiting line.** On Ask me, or when the turn returns `NEEDS_OWNER`, the assistant posts a fixed line instead: "I've asked Addam". It uses the same fields, `waiting: true`, and **the waiting id** (the answer id's hash with a different domain). This tells the asker it's pending. It also lets every device of the owner's build the `needs` line from the channel. A later answer uses the answer id, so it's posted alongside, never blocked by the waiting line.

**Before posting:**

- `scrub::board_reasons`. A match posts nothing and shows a `held` line saying what was held back.
- Replies are capped at 1,500 characters.
- The text goes through §7's normalising.

## 6. What it may share

**The rule: on its own, an assistant says only what the asker's device already holds.** So the feature adds convenience, not access. A clever question can't get anything out, because the turn never held anything else.

| About its owner | On its own | With the owner's yes (§7) |
|---|---|---|
| Open cards on **this** space: title, state, holders, steps, blockers, linked PRs | Yes | n/a |
| Comments on this space's cards | Yes, recent ones | n/a |
| Their agents on this space: name, status | Yes | n/a |
| This channel's recent posts | Yes | n/a |
| Cards on **another** shared space, even one the asker is also in | No | Yes |
| Private cards, comments, threads, waits | No, and not even that they exist | Yes, exact text |
| Personal channels, DMs | No | Yes, exact text |
| Agents' transcripts and turns, notes, commits and PRs not on a space card | No | Yes, exact text |
| Secrets and absolute paths | Never | Never: the scrub refuses them |

**The packet is built by code**, in a new `crates/core/src/service/assistant_answers.rs`:

- **It reads only `b.coord()` for this space's log.** Don't reuse the helpers that already render these fields, because each one crosses into the private log:
  - `tools::board`'s `card_text`/`card_text_in` merge the private log's waits ("A wait on a shared card is in the waiter's own log") and resolve refs through `router()` against private cards.
  - `wait_line` appends a wait's free-text note.
  - `no_agent_line` reads local spawn state.
  - `board_views` rewrites holders into this device's session keys.

  Card 3 tests a space card with a private-log wait on it and a private agent holding it, and checks that neither shows.
- **What it holds, in order:** the owner's open cards first (at most 30), then their agents, then the channel's last 10 posts.
- **How big it gets:** each title and post is clipped to 300 characters, each objective and comment to 500, and the whole packet to 24 KB. Today a person may post up to 512 KB, and the split sits before the inbox's own clipping. So without these clips one member could make every turn cost a full context.
- **The asker's words:** clipped to `MAX_TEXT` and fenced as untrusted, following `inbox::fence`. The system prompt says so.

**The output is parsed by code:** a reason first, then `ANSWER: <text>` or `NEEDS_OWNER: <one line>`. Reason before verdict is what makes Haiku use what it read (see the repo note on this). Anything that doesn't parse counts as `NEEDS_OWNER`.

The system prompt sends to `NEEDS_OWNER` anything that would commit the owner: their time, their review, an approval, a yes or no on their behalf. Because the asker can steer the text, a prompt can't be the only guard. §3's reading rule ("information, never a decision") is the one that holds.

## 7. The owner's OK

The `needs` line shows the question with **Answer this**. Nothing about private work leaves without these steps:

1. The tap starts **a turn of its own, not an ordinary chat turn**, on the device where it was tapped:
   - It has its own system prompt and renderer, with the owner's read tools.
   - The peer's question is in a real fenced slot. `render_prompt` has no such slot today: it writes everything as "The person:".
   - No chat history.
   - No `<<thread>>`, `<<go>>` or ask markers are taught or parsed. Today's chat would auto-go on a `<<go>>` while the panel is open, or offer **Post thread** for an attacker-written draft.
   - It is never stored as a person's turn.
2. The reply is a draft, rendered as plain text like a `from` line, with **Send to Eve's assistant** and **Don't send**. The model can't send it: it has no tool to.
3. On Send, the host posts it as the assistant's card, with `replyTo` and the answer id, through a new Tauri command beside `assistant_ask`. Send is refused if a post with that answer id already exists, so two desktops can't both answer. The scrub still applies.

**"Exactly the text shown" needs defining.** The chat never shows the raw reply. `splitGo` removes go lines, and `plainBlock` turns some invisible characters into spaces but not tag characters (U+E0000 block) or variation selectors. So the host normalises first, then shows and posts that same string:

- Markers are stripped.
- Hidden carriers are stripped: the tag block (U+E0000–E007F), bidi controls, and zero-width and other category Cf characters, except a ZWJ or VS16 inside an emoji sequence. Ordinary emoji survive, and text smuggled in invisible characters doesn't. The stripped string is what's shown and what's posted, so nothing differs between them.
- Links are shown as their full URL.
- The 1,500-character cap is counted after normalising.

The draft is shown in full, never collapsed.

The Send button appears only on a turn started by that tap, so a marker a peer got the model to write elsewhere produces nothing. Each OK covers one answer. There is no standing permission in v1.

**The question isn't the only untrusted text in that turn.** Its read tools bring in other people's words too. `show` and `board` return card titles, objectives and comments written by any member of a shared space, unlabelled. `search` and `events` can return their posts. So a member can plant instructions in a card and then ask a question that leads the owner's turn to read it. The system prompt says everything the tools return from a shared space is untrusted, like the question. The defence that holds is the one above: nothing leaves until the owner has read the exact text.

The owner's OK isn't built on `Request` and `Inner::request`, which share cards. Those keep their action in memory only, assume an agent's next turn, and preview card fields only.

## 8. What the receiver sees

**The labelled line.** A new chat entry: `role: "assistant"` with a new field `from: {person, space, conv, postId, kind}`. `kind` is one of:

| `kind` | When | Comes from | Dot |
|---|---|---|---|
| `answer` | Another person's assistant answered a question you asked (typed, or from your chat, §9) | The channel: an answer whose `replyTo` is your post, or your assistant's | Yes |
| `needs` | A question to your assistant waits on you: **Let it answer** (Ask me) or **Answer this** (`NEEDS_OWNER`) | The channel: your assistant's waiting line with no answer yet | Yes |
| `asked` | Your assistant answered someone on its own | The channel: your assistant's answer | No: a quiet line |
| `held` | The scrub held an answer back, or the rate limit or the hour dropped questions | The assistant's computer only: a Core event, since nothing is posted | No: a quiet line |

For example:

> **From Eve's assistant** · #office-assistants on mlg
> Who should review PR #12?
> *Your assistant answered: "…"* · Open in Talk

**How this fits #1000.** #1000 cut the chat to what waits on the person:

- `needs` waits on them.
- `answer` is the reply to a question they asked. #1000 removed "X replied on your thread" because the thread is where that reply lives. A question you put to someone's assistant is what you're waiting on, and is the person's "labelled line" in so many words.
- `asked` doesn't wait on anyone. But the person asked for this to be visible ("would want this to be legit"), so the owner sees each time their assistant spoke for them. It's a quiet line with no dot. If it turns out to be noise, it can fold to one per conversation. It is never dropped.

**Where lines come from.** Every line except `held` is built from the replicated channel, so every one of the person's computers and the phone get it:

- A new follower, `frontend/src/lib/assistant-talk-follow.ts`, runs on `talk:changed`. That event carries no payload, so it re-reads the person's assistants channels.
- It turns posts that concern this person into lines, with a per-device cursor.
- It needs a host call returning this person's own `assistantCard` per space, to recognise their assistant's posts.
- It doesn't go in `assistant-follow.ts`, which follows only the chat's own threads.
- `held` lines come from a Core event on the assistant's computer and show only there.
- The channel is the permanent record. localStorage is only a view.

**Taps:**

- **Let it answer** runs the tool-less turn (§5) on the device where it was tapped, if that device can run it. The answer id makes a second tap elsewhere harmless.
- **Answer this** is §7.
- A phone shows the line with neither button, since it can't run either turn, and offers Open in Talk.

**Rendering.** A `from` line has its own branch in `Entry`, before the reply branch:

- It never goes through `splitReply`, the `<<thread>>`/`<<go>>` parser or auto-go, because it's someone else's text.
- Its text is plain, clipped to 1,500 characters, with links shown as their URL.

**Surviving a reload.** `parseHistory` rebuilds each line from a fixed list of fields. A `from` line with no whitelist would come back as an ordinary assistant reply. Then its text would go through `splitReply`, so a `<<thread>>` in Eve's text would show **Post thread** as the person. So:

- `parseHistory` whitelists `from` and drops a `from` it can't validate.
- `historyFor` leaves `from` lines out, like `note`.
- `capKeeping` keeps `needs` lines that are still unanswered, so quiet lines can't push them past the 50-line cap.

Card 4 tests that after a reload the line still has `from`, renders no draft and no go button, and isn't in `historyFor`.

**In the model's history:** left out by default. If the person then asks about it, it goes in fenced as another person's words. `render_prompt` must never show it as "The person" or "You".

**Notifications.** Irori posts these in code. The assistant has no `notify` tool, and adding it to `ASSISTANT_TOOLS` would break the read-only rule and title it with the person's own name.

- `answer` notifies through Talk's own mention (§5), so there's no second toast.
- `needs` raises one toast through the `Notifier`, titled "Addam's assistant", with a dedupe key that stays the same per conversation, so a burst is one toast.
- On the phone the toast uses a `talk:` id and opens the channel (`phone_target` → `TAB_TALK`).
- On the desktop it uses a new target tab, `assistant`, which App.tsx's `onOpen` routes to `setAssistantOpen(true)`.

**Opening the chat.** The person said "maybe even open". v1 lights the dot and shows the toast, and clicking the toast opens the chat. Auto-opening is left out:

- `AssistantPanel` focuses its input when it mounts.
- `wire_opener` calls `show()`, raising the window over the person's work.
- On a phone the panel covers the screen.

A later tune: open without focus, only when the window is already in front, the person isn't typing and the window is wide.

## 9. Asking from your own chat

The chat model has no way to post, and keeps it that way:

- When the person says "ask Addam's assistant…", the model answers with a draft marker, parsed like `<<thread>>`, naming the person and the space.
- The chat shows the exact text, normalised as in §7, with **Send**.
- The tap posts it through a host command, as the person's assistant card, with `agentsNow: true`, after `scrub::board_reasons`.

Tap-to-send matters here because the model holds private reads while writing a question that will be public to the space.

`ASSISTANT_TOOLS` and its tests stay exactly as they are.

## 10. Devices, offline, limits

- **The assistant's computer is off or signed out:** the post sits in the channel and no answer comes. On return, §5's catch-up scan answers questions under an hour old, and older ones become a `held` line. A person can tell the assistant's computer is away from that person's presence.
- **Two of the person's desktops can't see each other:** both may answer. The answer id means peers show one.
- **Phone-only people** have no assistant turn, so nothing answers them. They can still ask and read. The phone shows the lines with no buttons.
- **A device catches up after a long time offline:** posts older than an hour stay unanswered, and readable in the channel. Accepted, as elsewhere (decision 48).
- **Removed from a space:** `active` membership already gates posting and the bridge.
- **Cost:** the receiver pays for each Haiku turn, which is why the rate limit stops the turn rather than demoting it. With the packet capped, the worst case is about members × 48 turns an hour of at most ~30 KB in.

## 11. Versions

Nothing needs a gate, and the verifier is unchanged:

- `agent.card` and `talk.channel.created` are existing kinds.
- `kind: "assistant"` is a new value of an existing string.
- `replyTo`, `waiting` and `answers` are optional fields.
- `assistantCard` sits outside `personSig` on `member.profile`.

On an older peer:

- It sees an ordinary channel and an agent card with an agent tag.
- It can ask, but its posts are ordinary.
- An older build of the owner's own may write a profile without `assistantCard`. The pointer survives it, because it folds like `githubLogin`.
- It doesn't refuse an assistants channel created in the wrong space (§4).

## 12. Build plan

All cards come after #1030, in this order. Each is normal weight unless marked, and links here. Every card: tests from the repo's patterns, `sh scripts/check.sh`, scratch builds per CLAUDE.md.

**1. The channel.**
Files: `crates/coord/src/talk.rs` (id, `is_assistants_channel`, the fold's cross-space refusal), `crates/core/src/service/spaces.rs` (`ensure_assistants_channel`, called from start, a new space and the host sweep), `frontend/src/components/talk/*` (pinned first).
Done when: a two-Core test shows both Cores with one channel of the same id in each space; a `talk.channel.created` for S1's id carried by S2 is refused; the channel isn't made in a hidden DM space or Personal; an archived channel stays archived.

**2. The assistant card, pointer and label.**
Files: `crates/identity/src/objects.rs` (`MemberProfile.assistant_card`), `crates/identity/src/board.rs` (fold it like `github_login`, `profile_assistant_card`), `crates/core/src/service/boards.rs` (`assistant_card`, `keep_current` carrying the pointer, `fold_card` keeping the owner's newest row for the label, `offers`), `crates/coord/src/board/asks.rs` (`free_agents`), `crates/core/src/service/boards_presence.rs`, `crates/tools/src/shared.rs` and `crates/core/src/service/bridge.rs` (`board_mentionables`: both names, no name squats), `crates/window/src/mentions.rs`, `frontend/src/lib/mentions.ts`, `frontend/src/components/common/mention-textarea.tsx`, `frontend/src/components/talk/*` (tag, `assistant` flag on the member, kind stays `agent`).
Done when tests show:
- No assistant row has a `person:` actor.
- A room ask passes an assistant card by.
- A profile written without `assistantCard` (an older build, or a rename through `keep_current`) doesn't clear the pointer.
- A kind-`claude` card named "Addam's assistant" isn't labelled as the assistant, and doesn't take `@Addam's assistant`.
- A kind-`assistant` card the profile doesn't point at isn't labelled.
- Another member's `agent.card` row for the card's id doesn't change its label or name.
- `@Addam's assistant` resolves to the assistant, not to the person Addam, both with a chosen name and without.
- The picker offers assistants only in the assistants channel.

The name is the default only. Reading #1029's `assistantName` is a one-line addition on #1029's name card or a light follow-up, so this card doesn't wait on it.

**3. The answering turn.** *(heavy: a security boundary and a delivery path)*
Files: new `crates/core/src/service/assistant_answers.rs` (packet, parse, checks, queue, catch-up scan), `crates/core/src/service/bridge.rs` (the split, before `agents_now`), `crates/core/src/service/spawn_remote.rs` (`answers` on `spawn.host`), `crates/core/src/service/spaces.rs` (the `pub` post wrapper), `crates/coord/src/talk.rs` (`replyTo` and `waiting` on `PostRow`, `Post` and its fold), `crates/window/src/talk.rs` (`TalkPost`), `crates/core/src/service.rs` and `app/src/shell/mod.rs` (`set_answerer`), `crates/window/src/assistant.rs` and `crates/window/src/lib.rs` (its turn and `Runner`, `tools:false`).
Done when, with a fake turn, tests show:
- Open answers once as `agent:<cardId>` with the answer id and `replyTo`.
- Each check in §5's table stops it, Closed included.
- A post with `replyTo` starts no turn.
- Two Cores both home answer, and peers show one post.
- A question that arrived before `set_answerer` is answered by the scan.
- The packet for a space with a private card, a card on another space, and a private wait on a space card contains none of them.
- A 512 KB post is clipped.
- A scrubbed reply posts nothing.
- A mention elsewhere runs nothing.
- Ask me and `NEEDS_OWNER` post the waiting line.

**4. The labelled lines and notification.**
Files: `frontend/src/lib/assistant.ts` (`from`, `parseHistory`, `historyFor`, `capKeeping`), new `frontend/src/lib/assistant-talk-follow.ts`, `frontend/src/components/assistant/assistant-chat.tsx` (the `from` branch, **Let it answer**), a host call for the person's `assistantCard` per space, `crates/core/src/service.rs` (the `needs` toast, the `held` event), `frontend/src/App.tsx` (the `assistant` target).
Done when:
- Tests show the line surviving a reload past #1000's filter with `from` intact, no draft or go button, and not in `historyFor`; `asked` and `held` lighting no dot; an unanswered `needs` kept through the cap.
- There are screenshots at desktop and phone width.
- A two-instance scratch run shows an `answer` line for a question typed in the channel, and a `needs` line under Ask me.

**5. Ask from your chat.**
Files: `crates/window/src/assistant.rs` (prompt, marker), `assistant.ts`, `assistant-chat.tsx`, `crates/window/src/commands.rs` (host command).
Done when: nothing posts before the tap; the posted text is exactly the normalised text shown; a draft with tag characters posts without them, and an emoji survives; the scrub runs; the ASSISTANT_TOOLS test is untouched.

**6. The owner's OK.**
Files: `crates/window/src/assistant.rs` (the separate turn, prompt and renderer), `crates/window/src/commands.rs` (Send), `assistant.ts`, `assistant-chat.tsx`.
Done when: Send appears only on a turn the tap started; that turn parses no markers and never auto-goes; what's posted is exactly the normalised text shown; Send is refused once an answer exists; one OK sends one answer; a phone shows no action it can't run.

**7. (light) Tidy the docs.** The list in §14.

## 13. Not in v1

- **"I can spawn one."** A peer can't start agents on your machine (decisions 50 and 51). The assistant says it will ask you. A later **Start a reviewer** button on the line would be a person's tap and fits on top. #1022's reviewer picking is the natural first use, and should ask through this channel rather than invent a second way.
- **Rounds between assistants.** D7 could relax to a next round the person taps.
- **Standing permission** ("always tell Eve about my private cards").
- **Opening the chat by itself** (§8).
- **DMs between assistants.** They're refused today, and the visible channel is the point.
- **Closing the HLC backdating gap** (§3). It applies to every card and needs a verifier change.

## 14. Found along the way

Worth correcting, on build card 7:

- `talk-channels.md` §3 says every member agent gets next-turn reading. Other people's agents don't (`tell_readers`); only mentions cross people.
- `merge-desk.md` still places desks under Talk › Channels in places. Decision 54 took them out.
- Decision 23 says "Ask me (default)". Decision 26 and the code say Trusted.
- `v0.5-shared-boards.md` still says "stop at hop 6". It's 50 (decision 37).
- Decision 26 says requests reach every device and back up in Needs you. Phones don't show ask toasts, and `needsOf` has no asks.

Two behaviours that look like bugs, neither of them this card's:

- `talk_post`'s spec says mentioning "person" notifies their phone. In a space channel, `board_key` refuses `user` and `map_talk_keys` silently drops the mention.
- The assistant's `events` tool ignores the caller and can list Personal channel posts. It's the person's own data, but the `talk_*` tools were left out, so it may be a surprise.
