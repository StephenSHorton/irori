# Office Assistants talking across people: design

*2026-11-08, card #1030, from the person's idea on thread #1021. Design only: nothing here is built yet. Build cards are in §12.*

The idea, in the person's words: "Maybe office assistants can communicate with other people's office assistants", to agree on who should review a PR, or because "I need to ask addam some questions, maybe office assistants already have that info". "Would want this to be legit though", and "We don't have to go hard on this. just an idea."

So this is the smallest version that is legit. One person or agent asks, the other person's assistant answers once, in a channel everyone in the space can read.

## 1. Decisions

| | Decision | Why |
|---|---|---|
| D1 | **Each person has one assistant per space. It has a name the person may choose, and by default it is "Addam's assistant".** Others always see "Juno · Addam's assistant", the person part taken from who wrote the row. | The person's decision on #1029. The default needs nothing written and is how people already talk. The name store is #1029's, so both designs give one answer. |
| D2 | **It is an `agent:<cardId>` card with kind `assistant`**, never `person:`, and the owner's `member.profile` points at it. | The only actor shape every peer accepts that is not the person. The pointer, not the id, says which card is the assistant, so nobody can squat it (§3). |
| D3 | **One channel per space, `#office-assistants`, made by itself.** | The person asked for a dedicated channel; decision 52 says no switching on. |
| D4 | **An assistant answers only in that channel.** | One visible record, replicated, and it can't be edited or deleted. |
| D5 | **The turn that answers another person has no tools.** Irori gathers the facts in code and the model only writes text. Irori posts it. | A peer's words must not be able to reach private data. See §6. |
| D6 | **It only ever says what the asker's device already holds**: this space's rows. Anything else needs the owner's yes on the exact text. | This makes the sharing rule structural instead of a prompt. |
| D7 | **One question, one answer.** An answer never starts another turn. | Two assistants can't loop, by construction. |
| D8 | **A labelled line in the receiver's Office Assistant chat, the dot, and a notification.** The chat does not pop open by itself in v1. | Opening steals focus today, and covers a phone screen. §8 has the tune for later. |
| D9 | **It never starts agents, answers blockers, approves, merges or speaks as its owner.** "I can spawn one" becomes "I'll ask Addam". | "It must never act as the person." |

## 2. What is there today

The earlier check on #1023 read a checkout 265 commits behind main. On main (1cb61ddd):

- **The Office Assistant exists, per device, not per person.** Each question is one headless `claude -p` on Haiku, with no session between turns (`crates/window/src/assistant.rs`). Its history is the webview's localStorage. It is off the roster on purpose, so it has no agent key, card or inbox.
- **Its authority is the person's, read-only.** The API treats its secret as `Sender("user:<device>", …, "assistant")` and allows only `tools::ASSISTANT_TOOLS` (14 read tools, pinned by a test). It reads private cards and its own agents' transcripts.
- **Delivery across people already works in space channels.** `talk_post_bridge` wakes `agent:` keys that a post mentions, under the receive setting, Pause, rate limit and hop limit. Card #1030's "mentions aren't delivered across people" is out of date for channels, cards and threads; it is still true for `send`/`reply`, and DMs to another person's agent are refused on purpose.
- **So the gaps are identity and a way to answer**, plus the channel, the labelled line and the sharing rule.
- **#987's assistant write tools are not on main.** Nothing here builds on them.

## 3. Who the assistant is

**The name.** #1029 owns it (its build card "The assistant's name"): a private `assistant.name` row `{name, by, at}`, newest wins, set in the app, published to other members as an `assistantName` field on `member.profile`. No name means the default, `<profile name>'s assistant`, computed, so nothing is written and every device agrees. This design only reads it. Until #1029's card lands, every assistant has the default.

**The card.** A new `agent.card` row on the space's log:

- `cardId`: a random id from `new_board_id`, like any card.
- `name`: the assistant's name (above), rewritten when it changes.
- `kind`: `assistant`. Today's values are `claude` or the agent's own kind; nothing reads it as a closed list.
- `personId`, `deviceId`: as for any card. `status`: `idle`.

Written by a new `Services::assistant_card(board)`, not `card_for`, which needs a roster entry. No roster entry and no `board_cards` mapping: the assistant stays off the roster, and nothing reaches the inbox (§5).

**The pointer.** The same call sets a new `assistantCard` field on the person's `member.profile` for that space, to the card's id. It sits outside `personSig` like `color`, `avatar` and `githubLogin`, so older builds still count the row. Only the person's own devices can write their profile, so only they can say which card is their assistant. A card counts as the assistant only when its owner's newest profile points at it. Only the assistant's computer (below) mints a card, and only when the profile has no pointer. If two devices race, both cards are the person's, the newest profile wins, and the other is an ordinary idle card nobody mentions.

**Why not an id everyone can compute.** That was the first draft, and it fails. `fold_cards` gives an id to the first `agent.card` row whose writer's person matches its `personId`, so another member can write a row with that id and their own `personId` first. A newer verifier could refuse such a row, but an older peer would accept it, and then refuse the owner's real posts in `bind_actor`. A refused row stalls the writer's stream (`store::kinds`, "stalling … fails closed"), so one hostile member would stall the owner's device on every older peer. A random id can't be claimed ahead, and the pointer needs no verifier change. Decision 30's key rotation doesn't matter either: nothing here is derived from a key.

**It can't be forged across people.** `bind_actor` already checks that an `agent:` actor's card belongs to the signing device's person, and an `agent.card` must name its writer's person. A peer can only ever write as its own assistant.

**What the label proves.** It proves which person's device wrote the row, not that their real assistant wrote it. A member running a modified client can post as their own assistant whatever text they like, the same as they can post as themself. They can never post as someone else's assistant. Peers should read an assistant's answer as that person's word, not as a machine-checked fact.

**The label comes from the row, not the name.** Show "Juno · Addam's assistant" (or just "Addam's assistant" for a default name) only when the card is kind `assistant` *and* the owner's profile points at it. The person part is always built from the card's verified `personId` and that person's profile name, never from the card's `name`. A card merely *named* "Addam's assistant" that fails either test shows as an ordinary agent ("Addam's assistant · Eve", agent tag), so a name can't be used to pass as someone's assistant. Talk's `Member`/`agent.card` mapping carries the kind through; `conversation.tsx` shows an `assistant` tag.

**Mentions.** People pick from the list, which records the key. Agents write `@Addam's assistant` or `@Juno`, which `coord::mentions` already matches: names match character by character, so the apostrophe and space are fine, the same way `@cedar two` is. If two members share a name, the name is ambiguous and the sender gets the candidates' keys, as today. `board_mentionables` lists the card with no change. Offer it in the picker only in `#office-assistants`.

**Which computer answers.** The person's one device for which `coord.agent_home("")` is this device, and which has the answering turn installed (desktop, Claude Code signed in; the same condition as the namer). If that computer is off, nothing answers (§10).

## 4. The channel

- **Id:** `ch_` plus a hash of a fixed domain and the space id, following `pair_dm_id`. Every member computes the same id, and the fold keeps one channel however many write it. The id is public, and that is fine: a channel has no owner to steal, and it can't move to another space.
- **Name:** `office-assistants`. Topic: "Ask anyone's assistant. It answers from what is on this space."
- **Made by** a new `ensure_assistants_channel(space)` on Core start, when a space appears, and when a space's gate opens. It writes `talk.channel.created` through `append_space_talk` if the channel isn't folded yet, and retries next time if the gate refuses. Skip hidden DM spaces and Personal. Don't use `create_channel_in`: it mints a random id and joins the author.
- **No new kind and no schema change:** `coord::talk::is_assistants_channel(id, space)` recomputes the id. An older device shows an ordinary channel.
- Talk pins it first in the space's channels. Anyone may archive it, like any channel; archived means no answers, and `ensure` does not bring it back.

## 5. Asking and answering

**Three ways to ask**, all a post in the channel that mentions the assistant's card:

1. A person types it in the channel.
2. An agent posts it with `talk_post` (how #1022's reviewer picking will use this).
3. From your own Office Assistant chat (§9), posted as *your* assistant after you tap Send.

**The bridge.** `talk_post_bridge` runs on the receiver's Core, and has already checked the author and the channel. Add one split, before `send_ref`:

- Mentions of *this person's assistant card*, in the assistants channel, go to the answering turn.
- Everything else carries on to `inbox.bridge` unchanged.

Not through `inbox.bridge`: it would leave a delivery for a key with no session and no wake, which the sweep then treats as gone. A mention of the assistant in any other channel, or on a card, reaches nothing.

**When a turn runs.** In order, each failing one stopping it:

| Check | If it fails |
|---|---|
| This device is the assistant's computer (§3) | Nothing here; that computer handles it |
| Post is younger than 1 hour | Chat line only: "Addam's assistant didn't get to this in time" |
| The post isn't an assistant's answer: its author's card is kind `assistant` *and* it has `replyTo` | No turn, ever (D7) |
| Not already answered: no post of this assistant has this `replyTo` | Nothing |
| Pause all is off on this device | Treat as Ask me below |
| The receive setting (below) | Below |
| Under the rate limit: 8 per asking person per 10 minutes (`DIRECT_LIMIT`, `DIRECT_WINDOW`), kept in memory | Chat line: "N questions from Eve were not answered" |

**The receive setting decides**, read with the same `receive_for` and `receive_decision` lookup other people's rows use:

| Setting | The assistant |
|---|---|
| Open (the default) | Answers |
| Members wake | Answers people you picked; the rest as Ask me |
| Ask me | A chat line with **Let it answer**; a tap runs the turn |
| Closed | Does not answer. Normal agents get these at their next turn, but this assistant has no next turn |

**The turn.** Modelled on naming (`thread_names.rs`): a `set_answerer` hook installs a `Namer`-style turn on desktops, and a worker thread takes jobs from a channel and survives a panic. Own `Runner`, so a peer never takes a slot from the person's chat. `tools: false`, no MCP config and no secret. Same model as the Office Assistant.

**The post.** Irori posts the text as `agent:<cardId>` with `append_space_talk`:

- `mentions`: the asker's card if an agent asked, so it hears. People get the §8 notification.
- `replyTo`: the question's `<deviceId>/<seq>`. A new optional `PostRow` field; older readers ignore it.
- `hop`: the question's hop + 1, set explicitly. `board_hop` looks the actor up by session key, and the assistant has none.
- `agentsNow: true`.

Before posting: `scrub::board_reasons`. A match posts nothing and puts a line in the owner's chat saying what was held back. Replies are capped at 1,500 characters.

## 6. What it may share

**The rule: on its own, an assistant says only what the asker's device already holds.** So the feature adds convenience, not access. A clever question can't get anything out, because the turn never held anything else.

| About its owner | On its own | With the owner's yes (§7) |
|---|---|---|
| Open cards on **this** space: title, state, holders, steps, blockers, linked PRs | Yes | n/a |
| Comments on this space's cards | Yes, recent ones | n/a |
| Their agents on this space: name, status | Yes | n/a |
| This channel's recent posts | Yes | n/a |
| Cards on **another** shared space, even one the asker is also in | No | Yes |
| Private cards, comments and threads | No, and not even that they exist | Yes, exact text |
| Personal channels, DMs | No | Yes, exact text |
| Agents' transcripts and turns, notes, commits and PRs not on a space card | No | Yes, exact text |
| Secrets and absolute paths | Never | Never: the scrub refuses it |

Facts go in **as a packet built by code**, in `crates/core/src/service/assistant_answers.rs`, read only from the folded view of this space's log. Owner's cards first, at most 30, then their agents, then the channel's last 10 posts. The asker's words go in fenced as untrusted, following `inbox::fence`, and the system prompt says so.

The turn's output is parsed by code: a reason first, then `ANSWER: <text>` or `NEEDS_OWNER: <one line>`. Reason before verdict is what makes Haiku use what it read (see the repo note on this). Anything that doesn't parse counts as `NEEDS_OWNER`.

On `NEEDS_OWNER` the assistant posts a fixed line, "That isn't on this space, so I've asked Addam", and the owner gets §7's line.

## 7. The owner's OK

The chat line shows the question with **Answer this**. Nothing about private work leaves without these steps:

1. The tap starts an ordinary turn of the owner's own chat, with its read tools. The peer's question is in the prompt, fenced as untrusted.
2. The reply is a draft in the chat, with **Send to Eve's assistant** and **Don't send**. The model can't send it: it has no tool to.
3. On Send, the host posts exactly the text shown, as the assistant's card, with `replyTo`, through a new Tauri command beside `assistant_ask`. The scrub still applies.

The Send button appears only on a turn started by that tap, so a marker that a peer got the model to write elsewhere produces nothing. Each OK covers one answer. There is no standing permission in v1.

**The question is not the only untrusted text in that turn.** Its read tools bring in other people's words too: `show` and `board` return card titles, objectives and comments written by any member of a shared space, unlabelled, and `search` and `events` can return their posts. So a member can plant instructions in a card and then ask a question that leads the owner's turn to read it. The system prompt for this turn says that everything the tools return from a shared space is untrusted, like the question. The defence that holds is step 2: nothing leaves until the owner has read the exact text, which is why the draft is shown in full (not collapsed, links shown as their URL) and capped at the same 1,500 characters as §5.

The owner's OK is not built on `Request` and `Inner::request`, which share cards. Those keep their action in memory only and assume an agent's next turn, and their preview covers card fields only.

## 8. What the receiver sees

**The labelled line.** A new `ChatEntry.from`: `{person, space, conv, postId, kind}`, where `kind` is one of:

| `kind` | When | Dot and toast |
|---|---|---|
| `answer` | Another person's assistant answered a question you asked (from your chat, §9) | Yes |
| `needs` | A question to your assistant waits on you: **Let it answer** (Ask me, §5) or **Answer this** (§6, `NEEDS_OWNER`) | Yes |
| `asked` | Your assistant answered someone on its own | No: a quiet line only |
| `held` | The scrub held back your assistant's answer, or the rate limit or the hour dropped questions | No: a quiet line only |

For example:

> **From Eve's assistant** · #office-assistants on mlg
> Who should review PR #12?
> *Your assistant answered: "…"* · Open in Talk

**How this fits #1000.** Card #1000 cut the chat down to what waits on the person. `answer` and `needs` do. `asked` doesn't, but the person asked for this feature to be visible ("would want this to be legit"), so the owner sees each time their assistant spoke for them. It is a quiet line, with no dot and no toast, so it doesn't ask for attention. If it turns out to be noise, `asked` lines can fold into one per conversation; they are never dropped altogether.

- **Where it comes from:** the replicated channel, not a push. A new follower beside `assistant-follow.ts` (which, after #1000, follows only the threads the chat posted, and is not the place for this), on `talk:changed`, turns new posts that concern this person (asked of their assistant, answered to them) into lines, with a per-device cursor. So every computer of the person's gets the line, and the channel is the permanent record while localStorage is only a view.
- **Its rendering:** its own branch in `Entry`, before the reply branch. It never goes through the `<<thread>>`/`<<go>>` parser and never fires auto-go, since it is someone else's text. Its text is shown as plain text, with links as their URL.
- **Surviving a reload:** after #1000, `parseHistory` drops a saved assistant line that has `update` but no answer act and doesn't pass `waitsOnPerson`. A `from` line must be its own field, never an `update`, and `parseHistory` must whitelist `from`, or a reload drops it. Build card 3's test checks exactly this.
- **In the model's history:** left out by default, like `note`. If the person then asks about it, it goes in fenced as another person's words. `render_prompt` must never show it as "The person" or "You".

**Notifications.** Irori posts these in code; the assistant has no `notify` tool, and adding it to `ASSISTANT_TOOLS` would break the read-only rule and title it with the person's own name. One toast through the `Notifier`, titled "Addam's assistant", when a question needs the owner or an answer arrives, dedupe-keyed by conversation so a burst is one card. Use a `talk:` id so that it also works on the phone toaster, opening the channel.

**Opening the chat.** The person said "maybe even open". v1 lights the dot and shows the toast; clicking the toast opens the chat. Auto-opening is left out: `AssistantPanel` takes the focus when it mounts, `show()` raises the window over the person's work, and on a phone the panel covers the screen. A later tune: open without focus, only when the window is already in front and the person isn't typing, and only on a wide window.

## 9. Asking from your own chat

The chat model has no way to post, and keeps it that way. When the person says "ask Addam's assistant…", it answers with a draft marker, parsed like `<<thread>>`, naming the person and the space. The chat shows the exact text with **Send**. The tap posts it through a host command, as the person's assistant card with `agentsNow: true`. Tap-to-send matters here because the model holds private reads while writing a question that will be public to the space.

`ASSISTANT_TOOLS` and its tests stay exactly as they are.

## 10. Devices, offline, limits

- **The assistant's computer is off or signed out:** the post sits in the channel, and no answer comes. On return, questions under an hour old are answered and older ones become a chat line. A person can see the assistant's computer is away from the person's presence.
- **Phone-only people** have no assistant turn, so nothing answers them; they can still ask, and read. The chat line is shown on phones, but a phone can't run **Let it answer**, **Answer this** or Send, as it can't run the chat.
- **A device catches up after a long time offline:** the bridge runs on live rows only, so some may be missed. They stay readable in the channel. Accepted, as elsewhere (decision 48).
- **Removed from a space:** `active` membership already gates posting and the bridge.
- **Cost:** the receiver pays for the Haiku turn, which is why the rate limit stops the turn rather than demoting it.

## 11. Versions

Nothing needs a gate. `agent.card` and `talk.channel.created` are existing kinds, `kind: "assistant"` is a new value of an existing string, `replyTo` is an optional field, and `assistantCard` sits outside `personSig` on `member.profile`, as `color` did. The verifier is unchanged. An older peer sees a channel, and an agent card with an agent tag, and can ask but its posts are ordinary. Rows from an older peer work as questions.

## 12. Build plan

All after #1030, in this order. Each one is normal weight unless marked, and links here. Every card: tests from the repo's patterns, `sh scripts/check.sh`, scratch builds per CLAUDE.md.

**1. The card and the channel.** Files: `crates/coord/src/talk.rs` (id, `is_assistants_channel`, `PostRow.reply_to`), `crates/identity/src/objects.rs` (`MemberProfile.assistant_card`, outside `personSig`), `crates/core/src/service/boards.rs` (`assistant_card`, the profile pointer), `crates/core/src/service/spaces.rs` (`ensure_assistants_channel`), `crates/window/src/mentions.rs`, `crates/tools/src/talk.rs` (label, picker limit), `frontend/src/components/talk/*` (tag, pinned first). Done when: a two-Core test shows both Cores with one channel of the same id, each person's assistant card listed by the other's `board_mentionables`, no assistant row with a `person:` actor, a kind-`claude` card named "Addam's assistant" not labelled as the assistant, a kind-`assistant` card the owner's profile doesn't point at not labelled as the assistant, an older-build profile row (no `assistantCard`) still counted, and two of the owner's devices minting at once settling on one pointed-at card. The name is the default only; reading #1029's `assistantName` is a one-line addition on #1029's name card or a light follow-up, so this card doesn't wait on it.

**2. The answering turn.** Files: new `crates/core/src/service/assistant_answers.rs` (packet, parse, loop), `crates/core/src/service/bridge.rs` (the split), `crates/core/src/service.rs` and `app/src/shell/mod.rs` (`set_answerer`), `crates/window/src/assistant.rs` (its turn, `tools:false`), `crates/inbox` (expose the receive lookup). Done when, with a fake turn: Open answers once as `agent:<cardId>`; each §5 check stops it; an answer starts no turn; the packet of a test with a private card and a card on another space contains neither; a scrubbed reply posts nothing; a mention elsewhere runs nothing.

**3. The labelled line and notification.** Files: `frontend/src/lib/assistant.ts` (`from`, `parseHistory`, `historyFor`), a new follower beside `frontend/src/lib/assistant-follow.ts`, `frontend/src/components/assistant/assistant-chat.tsx`, `crates/core/src/service.rs` (toast), `frontend/src/App.tsx` (open the chat from a toast). Done when: tests show the line surviving a reload past #1000's `waitsOnPerson` filter, `asked` and `held` lines lighting no dot, never parsed for markers, and not in the model's history as the person; screenshots at desktop and phone width; a real two-instance scratch run shows the line.

**4. Ask from your chat.** Files: `crates/window/src/assistant.rs` (prompt, marker), `assistant.ts`, `assistant-chat.tsx`, `crates/window/src/commands.rs` (host command). Done when: nothing posts before the tap, the posted text is exactly what was shown, and the ASSISTANT_TOOLS test is untouched.

**5. The owner's OK.** Files: the same chat files, plus §5's Ask me and §7's buttons. Done when: Send only appears on a turn the tap started, what's posted is exactly what was shown, one OK sends one answer, and a phone shows no action it can't run.

**6. (light) Tidy the docs.** The list in §14.

## 13. Not in v1

- **"I can spawn one."** A peer can't start agents on your machine (decisions 50 and 51). The assistant says it will ask you. A later **Start a reviewer** button on the line is a person's tap and fits on top; #1022's reviewer picking is the natural first use, and should ask through this channel rather than invent a second way.
- **Rounds between assistants.** D7 can relax to a person-tapped next round.
- **Standing permission** ("always tell Eve about my private cards").
- **Opening the chat by itself** (§8).
- **DMs between assistants.** Refused today, and the visible channel is the point.

## 14. Found along the way

Worth correcting, on build card 6:

- `talk-channels.md` §3 says every member agent gets next-turn reading: other people's agents don't (`tell_readers`), and only mentions cross people.
- `merge-desk.md` still places desks under Talk › Channels in places; decision 54 took them out.
- Decision 23 says "Ask me (default)"; decision 26 and the code say Trusted.
- `v0.5-shared-boards.md` still says "stop at hop 6"; it is 50 (decision 37).
- Decision 26 says requests reach every device and back up in Needs you; phones don't show ask toasts, and `needsOf` has no asks.

Two behaviours that look like bugs, neither of them this card's:

- `talk_post`'s spec says mentioning "person" notifies their phone, but in a space channel `board_key` refuses `user` and the mention is silently dropped.
- The assistant's `events` tool ignores the caller and can list Personal channel posts. It's the person's own data, but the `talk_*` tools were left out, so it may be a surprise.
