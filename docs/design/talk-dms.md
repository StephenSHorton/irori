# Person-to-person DMs: the build (card #538, Talk 6)

*2026-11-05. Design: [talk-channels.md §5](talk-channels.md) ("Where DM rows live"). Builds on [spaces.md](spaces.md). This page says how the build does it.*

A DM between two people rides on a hidden two-person space, made on the first message. It has no boards and no invite, because the two already share a space (call it S). It isn't in the switcher. A DM with an agent stays on your own devices' log, as today.

## 1. What a DM space is

An ordinary space log (`spaces/<id>.db`) with a random id, one secret, and exactly two members:

- `board.created {name: "dm", tag: "~dm"}`, founded by the first writer. The tag fails `valid_board_tag`, so nobody can type it or give a board it. The tag is cosmetic: **trust rests only on the private marker below**.
- `board.member.added` for the other person, role **member**. A member can't add anyone and can't mint a board secret, so `rotate_secret` needs no change: the founder's device mints epoch 1, and the other side takes it from the offer.
- `talk.member` (join only) and `talk.post` rows. No new kind and no new field in the log.

**Private marker** (new kind, `Replicated, Refuses`; an older own device stores it and ignores it, so it never opens the space):

```
dm.space.joined {boardId, creator: MemberPin, with: <personId>, at}
```

It plays `board.joined`'s part (other devices of the person open the space from it) and says the space is hidden. It is written **before** the space opens or takes rows, so no row folds before the marker is there.

`DmSpaces` on `Services` holds the markers by space id. It fills before `talk.load` and `open_boards`, again before `talk.load` on a resync, and in every `reconcile_boards` pass before a space opens; `write_dm_marker` fills it as it writes. A marker without a sound creator, a valid id and `with` counts for nothing. A marker stays in the cache after its space is forgotten, so files that outlive a forget can't come back as a plain space. Whether a space is live is `board(id)`'s to say, and the limit of 4 live spaces counts that way.

**Conversation id:** `pair_dm_id(a, b)` is `dm:` and 16 hex digits of SHA-256 over a domain prefix and the two sorted `person:<id>` keys. It is never `canon`ed: `dm_id` turns your own key into `user`, so the two sides would disagree. `dm_id` stays for agent DMs.

**Both write first:** two logs, one conversation. Talk folds both under the same `convId`. A writer uses the lowest space id whose other person has a counted cert, otherwise its own lowest pending log.

## 2. Hand-off over S

S's log carries nothing new, and it couldn't: every member of S reads every row. The secret goes on the point-to-point `irori/board/1` connection to one of the other person's devices, in a new frame. An older build skips a frame type it doesn't know (`boardmesh.rs`, "A frame type this version does not know is skipped"). The board hello gains `dms` (`DMS_PROTOCOL = 1`); a device that doesn't say it is never asked.

```
Wire::Dm {boardId, step, body}      body: opaque JSON (invite depends on mesh, so Bundle can't be named there)
KeyProof {firstPub, rotations: [MemberKeyRotated]}
```

| Step | From | Body |
|---|---|---|
| `ask` | founder side | `{}` |
| `pin` | other side | `{pin, proof: KeyProof, cert: MemberCert}` |
| `offer` | founder side | `{bundle, proof: KeyProof}` |
| `done` | other side | `{rows}` (the joiner's cert and profile) |

**On the wire.** A frame's `boardId` is the hidden space; the connection it rides on is S's. The mesh hands the Core three things through `BoardMeshOpts::dms` (`DmHooks`): a peer's hello, each vector tick on its connection, and a `dm` frame. Each comes with `via` (S) and a `LivePeer`: the device id, the key its handshake proved, the person S's verifier certifies it to (never the hello's word), and its `dms`. A hello reaches the hook whatever it said about `dms`, so the window can tell an old device from a silent one. A frame reaches the hook only from a device S's verifier admits now, so one of the person's own devices that S doesn't count has no say. The hooks run on the connection's task and must return at once, so the Core queues the work. `BoardMesh::send_to(device, frame)` writes to that device alone, and refuses one that isn't connected, doesn't count on S, or never said `dms`. `live_peers()` lists who is connected now.

**Why a proof.** A board pin doesn't bind `personId` to a key: `valid_pin` checks shape, and an owner of some other space can list a device of theirs as "Bob". So S's word is never trusted for who someone is. `personId` is a hash of the person's first key (`first_key_of`), so `verify_key_proof(personId, proof)` checks `firstPub` against the id and walks each rotation's `prevSig`. A rotation from before v0.5 has no `prevSig` and can't be proven; that person gets no DM space. A device builds its own proof with `Chain::key_proof(pin.person_epoch, &pin.person_pub)`: the first key comes from the certs, because `forget` and pairing drop it from `person.json`, and two rotations to one epoch both stay in the log, so the walk keeps the path that ends at the pin's key. Pins and chains are public, so a replay of Bob's real pin must also fail: the `cert` must verify under the proven key and name the TLS peer's own `devicePub`.

**The founder checks `pin`:** the proof's result equals `(pin.person_epoch, pin.person_pub)`, `pin.root_device_id` is this connection's device, the cert verifies and names this peer. Then it writes `board.member.added` and sends the offer.

**The joiner checks `offer` before anything touches disk,** folding the bundle in a throwaway verifier: `board.created` verifies, `verify_key_proof` passes for its creator, the sender's S cert belongs to that creator, the members are exactly the two, and the pin is this device under the current key. A held id must already be this pair's. A person can have at most 4 live DM spaces with one other person, so S's members can't fill a disk.

**Then, in this order:** marker, `open_board`, secret, rows, cert and profile (role from this device's own chain, not `ROLE_DESKTOP`), `keep_current`, `done`. A failure after the marker forgets the space. `bundle::check_offer` does the fold, the two-member check, the pin and the secret's shape; the caller adds the proof, the S cert and the held id. The fold walks rows as `insert_board_rows` does (signatures, a run stops at a refused row or a gap), not bare `observe`, so a row the store won't keep can't change the answer.

**A second `join` writes nothing.** If `done` is lost, the founder offers again. `join` skips a cert and profile it already holds and returns this device's rows from seq 1, because fresh rows alone would sit past a `Gap` in the founder's store and the space would stay pending. `join` also folds the bundle before it opens anything, so a refused invite leaves no secret behind now that the secret goes in first.

**The whole log goes in the offer.** A store refuses a row past a seq gap (`Gap`), and the founder's posts come before its `board.member.added`, so the offer can't be trimmed to identity rows. New posts on a pending log are refused past half of `MAX_MESSAGE`, which is what an offer can carry.

**Pending** is derived, not queued: the other person has no counted cert. The hand-off runs when the space is made (walking live peers on S), on a hello from one of their devices on S, and on each S vector tick while pending. Hello alone isn't enough: it goes once per connection. Any founder device that holds the secret may run it. After the add, the retry goes only to the pin's root, and builds the offer without writing.

**Devices.** Once one device of a person holds the space, the rest follow: the marker reaches them, a private `dm.secret` frame carries the secret (the fields, sealing, vouching and pending path of `board.secret`, but older builds ignore it, so an old device never opens the log; `OwnBoards::is_dm` picks the name, and a DM space's secret never goes as `board.secret`), and `keep_current` certifies them.

## 3. The gate

S takes no new row, so S needs no gate. A hidden log has `talk.*` rows behind `gate_refusal` like any space, but with a different count: only devices **heard saying `spaces` below 1** hold it back (`BoardMesh::said_too_old`). `not_ready_for_spaces`'s rule (every device heard since this start) can't be met here. A log made a moment ago has heard nobody, and the other person can't connect until the hand-off that needs the first row, so a person with a sleeping phone could never start a DM. This is safe because a hidden log only reaches builds that know the marker and the `ask` frame; the version check moved to the hand-off, where an older device is simply never asked.

## 4. The fold rule

`space_may_touch` kept every space out of DMs (card #461): a space's members mustn't write into the rest of your Talk. It still does, with one door. `space_talk_row` stamps a fold-only `dmConv` on a row when all of these hold, and removes any `dmConv` a row arrived with:

1. A marker exists for this space, and its creator matches `board.created`'s.
2. Members in any state are within {me, `with`}.
3. `convId == pair_dm_id(me, with)`.
4. The actor is one of the pair (`person:` key).
5. The kind is `talk.post`, or `talk.member` with op join and a key of the pair.

Any other talk row of a hidden space folds to nothing. `space_may_touch` lets a space touch a DM only when `dmConv == conv`.

Refusals run **before** the append in `append_space_talk`, because `space_talk_row` runs after it and a refused row would stay in the log: a non-person `by`, any other kind, a `leave`, an `agent:` target. Mentions are cleared on a person DM, so `board_key` never mints a card and nobody is woken through one.

Agents don't DM other people, and people don't DM other people's agents: the tools refuse both. An agent's DM with its own person is unchanged.

## 5. Hiding

`shown_boards()` and `boards.list()` stay whole: the mesh, followers and Talk's fold need them. What people and agents see filters on the marker cache, which fills before `talk.load` and `open_boards`: `board_list`, `boards_by_tag`, `tag_holder`, `board_tag_holder`, notes, and the card and agent board candidates. A hidden space's relays and addresses come from the person's other spaces.

## 6. The window

A person DM has an empty `spaceId`, and `convInSpace` shows it under every pick. The new-DM picker has a people section (everyone you share a space with). A conversation carries `waiting` while pending, shown as "Waiting for Bob's Irori". If Bob's device says no `dms` in its hello, it reads "Bob's Irori needs updating". Opening a DM writes nothing; the first post does.

## 7. Changes from the card and from §5

- **The gate (§3).** The card says gate `talk.*` writes with `not_ready_for_spaces`. Its every-device rule can't be met by a log's first write, so the hidden log counts only devices known to be too old.
- **Leaving.** `talk_dm_leave` leaves the space, and a later DM from that person offers it again. A per-person block list is follow-up work.
- **No mailbox.** Delivery needs one device of each person online at the same time, as boards do today.

## 8. Checks

`five_cores.rs`:

- `a_dm_survives_a_restart_with_nothing_in_either_switcher`: the done test.
- `a_dm_starts_with_bob_already_connected`: the creation trigger.
- `a_dm_to_an_offline_friend_lands_later`.
- `a_two_device_person_can_start_a_dm_with_a_phone_asleep`.
- `a_second_device_gets_the_dm`.
- `both_first_messages_make_one_conversation`.
- `a_hostile_owner_cant_read_or_forge_a_dm`: M2 pinned as Bob, with its own key and replaying Bob's real pin and chain.
- `a_refused_offer_leaves_nothing_on_disk`.
- `a_third_member_cant_touch_a_dm`.
- `an_old_peer_gets_no_ask`.

Units: `verify_key_proof` (good chain, bad link, a pre-v0.5 rotation), each §4 rule, a `dms` hello round trip, unknown `Wire` types skipped.
