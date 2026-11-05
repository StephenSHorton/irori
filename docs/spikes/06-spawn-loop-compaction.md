# Spike 06: compacting v0.11.0's spawn-loop rows (card #467)

On 2026-11-04, from 07:50Z to 09:36Z, v0.11.0 on the Mac looped on one card
with no checkout (#464, task `01M6JRKD…`). Each try wrote a `task.spawn`
`requested` row and a `failed` row ("no checkout of  on <device>"), the rows
kicked the sweep, and the sweep asked again. That is 3,423,938 rows for that
one card, with gen up to 1,711,969. The PC (`05f1…`, Windows, 0.11.0) did the
same on four of its own cards from 07:59Z to 08:03Z: 8,480 rows, all
"no checkout of remote:github.com/StephenSHorton/…".

Everything below was measured on copies of the Mac's `irori.db`, taken with
`sqlite3 -readonly … .backup`. The live data dir was never written to.

## The log

| | Mac `3dc3…` | PC `05f1…` | `45a5…` |
|---|---|---|---|
| rows (seqs gapless) | 3,558,396 | 68,848 | 3,037 |
| task.spawn | 3,424,635 (seq 131,591–3,558,368) | 8,480 (seq 60,279–68,848) | 0 |
| last row | 09:42Z | 08:03:42Z | 2026-11-03 21:57Z |
| what | MacBook Pro | DESKTOP-STEPHEN, root device | Android phone, 0.10.0 |

The DB was 2.26 GB. task.spawn payloads were 884 MB and their signatures 220 MB.

## Why the rows can't just be deleted today

- Rows are signed one at a time, with no previous-row hash, so deleting a
  row breaks no other row's signature.
- **Ingest requires gapless seqs.** `insert_one` refuses `seq > last+1` with
  `ErrorKind::Gap` (store db.rs). The mesh ignores Gap and acks the
  unchanged vector, and the sender resends the same batch at once, forever.
  A peer whose vector for a device is below a deleted seq never gets any
  later row of that device, and nothing reports it. 0.11.0 has the same
  store and mesh code (`git diff v0.11.0 HEAD` touches only kinds.rs there).
- **A device's newest row must stay.** `append` takes `MAX(seq)+1`, so
  deleting it would reuse a seq that peers hold: a fork, then `FORKED_SELF`.
- Peer vectors are not saved (`save_peers` keeps only epoch and key), so the
  Mac can't tell what each peer holds. From hlc, the PC holds the Mac's log to
  at most seq ~400,672, and `45a5…` to at most ~113,136. Both are inside or
  below the loop's range.

So a row may go only if every peer already holds that device's log past it.
Today that floor for the Mac is ≤ 113,136, below every loop row:
**nothing can be deleted yet**. The dry run
(`--floor 3dc3…=113136 --dry-run`) confirms 0 rows.

## What the fold needs

`fold_spawn` keeps one `Spawn` per card part (taskId, step, subtask): a higher gen
replaces it, and a same-gen non-`requested` row ends it. The result depends only on the
part's newest-gen rows. Nothing counts rows or reads an older gen (the backoff reads the newest
`op`/`at`, and the next gen is newest + 1). `task.spawned` rows are kept.

A row is **superseded** when the same device wrote a higher gen for the same
part with the same `by`. Same device and `by` mean the same writer-check verdict.

## The tool (`irori-dev`, copies only; it refuses a live dir)

- `compact-spawn --home <copy> --floor <device>=<seq|top> … [--dry-run] [--no-vacuum]`
  deletes superseded rows at or below each device's floor, never a device's
  newest row. It runs in one transaction, checks that the vector is
  unchanged, then VACUUMs. A device with no floor keeps every row.
- `spawn-fold --home <copy> --person <id> --root <dev> --out <dir> --now <t>`
  times open and fold, and dumps every card's whole folded state (`Debug`,
  spawns included) plus the Board views.
- `reconnect-sim --from <copy> --to <copy> …` replays the mesh session's
  loop between two stores: `missing(v, 500)`, then `insert_batch`, then the Coord
  folds via subscription (resyncs included), then the ack vector.

## Measurements (Mac, release build)

| | before | after (`--floor <every device>=top`) |
|---|---|---|
| DB size | 2,256.8 MB | 289.3 MB |
| rows | 3,630,281 | 197,185 (3,433,096 deleted; 19 task.spawn stay) |
| store open (warm / cold) | 640 ms / 6.7 s | 23 ms |
| Board fold at start | 19.5–20.9 s | 0.35 s |
| compaction itself | | 15.8 s delete + 1.1 s VACUUM |

`spawn-fold` views before and after: `diff -r` is identical, including every
card's spawn state (#464's `Spawn{generation: 1711969, op: "failed", …}`).

## The PC's reconnect

Simulated with the PC holding the Mac's log to seq 400,672, so 3,157,724
rows behind, pulled from the uncompacted Mac copy:

| build | rows/s | full catch-up | where the time went |
|---|---|---|---|
| v0.11.1 (`vector()` = `GROUP BY`) | 659 | **80 min** | sender `missing()` 34 min, fold 25 min, ack `vector()` 19 min |
| this branch (skip-scan `vector()`) | 8,421 | **6¼ min** | fold 4.8 min, insert 1.4 min |
| Mac on this branch, PC still on 0.11.0 | | **~45 min** (derived, not run) | the v0.11.1 run minus its sender `missing()` time |

Both measured runs used one build for sender and receiver. The mixed row is
an estimate: a 0.11.0 receiver keeps its own per-ack `GROUP BY` and fold
costs. Network time is not included. `vector()` was a full scan of the log, 0.44 s
here, run once per batch inside `missing()` and once per ack. The skip-scan
over the `(device_id, seq)` key returns the same map in under 0.1 ms.

Against a copy compacted above the PC's position, the PC stalls on the first
batch: "replicated row leaves a seq gap: 3dc3… holds seq 400672, got 400681",
with 0 rows applied, resent forever. Compacted only up to 400,672, the PC
syncs normally (1,041 rows/s on v0.11.1's `vector()`).

**A 0.11.0 PC also restarts its own loop on launch.** Its four cards
(`01M6FWX5…`, `01M6JC5J…`, `01M6JC5W…`, `01M6JC5Y…`) last failed with no
checkout, and v0.11.0 has no retry wait. It loops until it is upgraded, or
until it has pulled the Mac's rows that close or assign those cards. Those
rows sit after the 3.16M loop rows.

## The source

- 0.11.1 (#453) added `SPAWN_RETRY` (600 s), so a card with no checkout
  writes 2 rows every 10 minutes instead of ~540/s.
- This branch: `spawn_for` leaves a card whose newest request failed with the
  same no-checkout reason, until a checkout or a Where here appears. That
  makes 0 rows after the first pair (test
  `a_card_with_no_checkout_fails_once_and_is_not_asked_again`, which fails
  without the guard: 4 rows, not 2).
- `board_create` on a spawning board no longer says "Irori starts an agent
  for it" for a card with no repo and no Where.

## Running it on the real log

Safe once every peer holds every device's whole log (or up to a known seq),
on each device, with Irori quit there:

1. Upgrade every device (the PC first, before it starts) to a build with the
   guard and the `vector()` fix, and let them all sync fully. The phone
   counts too. If it is retired, revoking it takes it out of the floor.
2. Quit Irori. Copy the data dir without `irori.json`:
   `sqlite3 -readonly ~/.irori/irori.db ".backup <copy>/irori.db"` and
   `cp ~/.irori/device.json <copy>/`.
3. Read the floors off the peers, not `top`. On each other device, read
   `sqlite3 <its irori.db> "select device_id, max(seq) from events group by device_id"`.
   For each device, the floor is the lowest of those values across the peers. A
   superseded row written after the last sync would otherwise become a hole.
   Run `irori-dev compact-spawn --home <copy> --floor 3dc3…=<n> --floor 05f1…=<m>`
   with `--dry-run` first, then compare with `spawn-fold` before and after.
   `top` is only for measuring.
4. Keep the old `irori.db` aside, put the copy's `irori.db` in place (remove
   the old `-wal`/`-shm`), and start Irori.

**After compacting, a new device can't join.** A device that is paired or reset
starts at seq 0, so it stalls on the first deleted row (the Gap above), silently
and for good. Pair every device and let it sync *before* compacting, and don't
reset or reinstall one afterwards, until sync accepts holes.

**Reading floors without the peer's database.** The phone's app data can't be
read (release build), and the Mac keeps peer vectors only in memory. Two proofs
work from the Mac, because ingest is gapless (holding seq N means holding all
below it):

- A removal's "seen by N of N" (`/v1/devices?detail=1`, `pending`/`remaining`)
  means every remaining device's vector holds that revoke row.
- A row a peer visibly has, such as a note showing on its screen, proves that
  row's device up to that seq.

**The run on the Mac, 2026-11-05.** Floors: Mac 3,588,367 (the removal of the old
phone, seen by 3 of 3), PC 534,767 (a note the phone showed; the PC's last loop
row is 533,805). 3,887,749 of 4,129,079 rows went, 88 task.spawn stayed, and
2,568 MB became 327 MB. Core ready went from 17.8–27 s to 2.3–2.5 s. Afterwards
the phone showed a Mac row written after the run (seq 3,590,741), so sync past
the deleted rows works. The PC and phone still hold the rows: the phone's data
can't be reached with `irori-dev`, and the PC needs its own run.

When scripting this, quitting Irori may end the agent running it, so launch the
script detached. `irori attach` helpers outlive the app and share its binary
path, so wait on the app's pid, not on a process-name match.

**The run on the PC, 2026-11-05 (#536).** Same floors as the Mac. 3,887,753 of
4,131,874 rows went, 111 task.spawn stayed, and 2,557 MB became 331 MB. Core
ready went from 37.9 s to 12.5 s (coord 32.5 s to 2.6 s; the inbox step, 8.3 s,
now leads), on a loaded PC. Fold on same-moment copies went from 53.3 s to 3.7 s,
with `spawn-fold` views identical for all 554 cards. The whole outage was about
2 min 40 s.

Afterwards the Mac held the PC's marker rows (seq 536,187 and 536,188, written after
the swap). The phone's card page wouldn't load, and its data can't be read, so the
proof came from its own row: a `notification.read` for a blocker that was raised and
resolved in PC rows above the swap (536,258). A device only marks what it has folded,
and ingest is gapless, so that row proves the seqs below it too.

What differs on Windows:

- **There is no scripted quit.** Only the tray's Quit ends the app: closing hides
  the window, so a plain `taskkill` should only hide it (read in the code, not run). `taskkill /F` on the
  app's pid is safe for the log, because WAL keeps every committed row, and it
  skips `install_on_exit`. A Quit with an update ready starts the installer.
- **Copy after the app is down, `irori.db` and `irori.db-wal` both.** A copy from
  before the kill loses rows, and the device then reuses their seqs: a fork.
  After a kill, take a plain file copy and let SQLite recover the WAL in the copy.
- **Build `irori-dev` from the tag the app runs**, so the tool can't migrate the
  copy past what the app reads.
- **Detach with Task Scheduler** (interactive logon), and restart through
  `explorer.exe <irori.exe>`, so the app lands in the desktop session.
- **Rolling back after Irori has run needs the new rows.** Moving the old DB back
  drops them, and the seqs get reused. Copy the rows above the old vector from
  `events` and `local_events` onto the old DB first.

Letting peers skip the rows instead of pulling them needs a sync protocol that accepts
holes (a signed "compacted to N" row, gap-tolerant ingest), with every peer
upgraded before any device compacts. That is not built.
