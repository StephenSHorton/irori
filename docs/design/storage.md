# Storage: see what Irori keeps on this device, and clean it out

*2026-11-07, card #728, from Stephen on #690: "maybe there's a place where everything is archived and the user can clean that out or browse it etc etc. That was what the maintenance section of settings was meant for. I didn't actually want it to go hide in the settings modal, I wanted it to be an accessible place to look at all the space irori is taking up and manage that space. conversations, threads, everything." It replaces Settings > Maintenance (#138).*

## 1. Why

Irori's footprint had no single place to look. On the PC on 2026-11-07, the data folder held about 2.3 GB, and the agents' worktrees held about 101 GB of build output in four trees. The space went to:

- attachments: 1.5 GB
- the log (`irori.db`): 400 MB
- old installers: 214 MB
- shared spaces: 100 MB

The only clean-up Settings offered was Clear done cards, which frees no space.

## 2. Retention stays manual

board.md's "No retention rules" holds, as does decision 25 ("no retention timers"). Storage never cleans anything by itself. The person picks a kind, sees its items, and chooses what goes.

Two things already clean themselves, and they stay as they are:

- the build-output sweep (#427, #565), which works on settled cards' trees after an idle hour
- log rotation

Both handle build output and logs. Neither touches the person's data.

## 3. What it lists

Each kind shows its size, how many items it has, where it is on disk, what it holds, and what deleting does, in plain words.

| Kind | Where | Can the page delete it? |
|---|---|---|
| Irori's log | `irori.db` (+ `-wal`, `-shm`) | No (§4). Instead the page offers a breakdown by what the rows are, Clear done cards, Clear resolved threads, and Put back. |
| Shared spaces | `spaces/<id>.db` and the files beside it | No. To remove a space from this device, leave or forget it from its menu on Boards. A direct-message space is never offered. |
| Attachments | `files/<2hex>/<sha256>` | Yes, each file or all of them. Files attached on this device are marked "may be the only copy". |
| Build output | `<tree>/target` in worktrees Irori made | Yes, except a tree an agent works in now. |
| Downloaded updates | `updates/` | Yes, except the installer waiting to install. |
| Logs | `irori.log.N`, `crash.log.N`, `crash-*.log`, `gui-*.log` | Yes, except `irori.log` and `crash.log`, which Irori is writing. |
| Agents' briefs | `briefs/<cardId>-<n>.md` and `.settings.json` | Yes, for a settled card or a card not on this device. |
| Left from older versions | `irori.db.v0.3`, `reset-*/` | Yes. |
| Saved copies | `received/` | Yes. The attachment itself stays. |
| Speech model | `voice/` | Yes. It downloads again on next use. |
| Conversation changes | `blobs/` (capture's patches and summaries) | No. No other copy exists, because the mesh never serves `blobs/`. |
| Claude Code's conversations | `<claude dir>/projects` | No. These are Claude Code's files, and deleting them would break `--resume`. |

**Conversations and threads.** Agents' conversations are `capture.*` rows in the log. Talk and threads are rows there too. All of them are listed in the log's breakdown.

**Closed boards (#690).** Closed boards aren't on main yet. When they land, their cards are cards in the log like any other, and Clear done cards and the cleared list cover them.

## 4. Why the log can't be trimmed

Every row in the log is signed, and each device numbers its rows with no gaps. Sync works from each device's highest row number (store `vector`, `missing`, `insert_one`). Deleting rows from the log breaks sync:

- Deleting a device's newest rows lowers the number other devices see, so the rows come back.
- Deleting rows from the middle makes this device serve a gap. A peer that is new or behind then stalls on it for good.
- Blanking a payload fails the signature check.

The one safe pattern is `irori-dev compact-spawn`. It only runs offline, and only on superseded rows below every peer's floor. A Storage button can't do that, so the page says plainly that the log can't be trimmed. What it offers instead is tidying: Clear done cards and Clear resolved threads write `task.archived` rows, which take cards out of the lists. They free no space, and Put back reverses them.

## 5. Deleting an attachment

Deleting an attachment removes the file from `files/` and adds its id to `files/dropped`. Without that mark, the small-file pick-up (#580, `prefetch.rs`) would fetch the file again from another device at the next start or resync, because it is small and recent. Two things clear the mark:

- a fetch the person asks for (opening the file)
- attaching the same file again

Both go through `Files::place_file`.

## 6. Where it lives in the app

- **Nav:** "Storage" in the sidebar, after Notifications, with the HardDrive icon. The palette has "Go to Storage", with keywords including maintenance, disk and clean.
- **Address:** `#storage` is the overview and `#storage/<kind>` is one kind's page. In the preview, use `?storage` or `?storage=<kind>`.
- **Moved here:** Settings > Maintenance is gone, and Clear done cards now lives on the log's page. The Closed view keeps its compact Clear done.

## 7. Build contract (card #728)

**Store** (`crates/store`):
- `Store::kind_sizes() -> Vec<KindSize { kind, rows, bytes }>`. `bytes` is payload plus signature, and the count covers both `events` and `local_events`.

**Blob** (`crates/blob/src/files.rs`):
- `Files::list() -> Vec<(id, size)>`
- `Files::remove(id) -> size`, which deletes the file and marks it dropped
- `Files::dropped(id)`
- The mark is cleared in `place_file`.

**Core:**
- `prefetch::check` skips dropped files.
- `Core::build_trees() -> Vec<BuildTree { tree, cards, settled, live }>`, built from the worktree book plus cards' Where on this device.
- `Core::clear_build(tree)`. It refuses a tree that isn't listed and keeps a live one (`LIVE_TREE`). Otherwise it calls `buildclean::clear(tree, 0, now)`.
- `Core::settled_cards()`.

**Window commands** (`crates/window/src/storage.rs`):

| Command | Returns |
|---|---|
| `storage_report` | `StorageReport { dir, kinds: StorageKind[] }`. Builds and Claude's folder come back with `measured: false`. |
| `storage_log` | `LogPart[]` (conversations, messages, cards, talk, notes, code, agents, other) |
| `storage_items {kind}` | `StorageItem[] { id, name, bytes, at, detail, cards, cleanable, keep, onlyHere }` |
| `storage_clean {kind, ids}` | `StorageCleaned { removed, freed, kept[] }`. Empty `ids` means every item that may go. Ids are plain names inside their folder, never paths. |
| `storage_cleared` | `ClearedCard[]` |
| `storage_unclear {ids}` | The number of cards put back |
| `threads_clear_count {olderThanSecs}` / `threads_clear {olderThanSecs}` | Clears resolved threads, which must be in a terminal state. `thread_rows` leaves them out while they stay closed. |

**Frontend:**
- gateway: `frontend/src/lib/storage.ts`, with a sample and `kindInfo` for the copy
- page: `frontend/src/components/storage/storage-page.tsx`
- nav: `lib/nav.ts`, sidebar, `App.tsx`

**Tests:**
- `crates/store/tests/it/store.rs` `kind_sizes_counts_rows_and_bytes`
- `crates/blob/tests/it/files.rs` `files_list_remove_and_bring_back`
- `crates/core/tests/it/prefetch.rs` `a_file_deleted_from_storage_is_not_picked_up_again`
- `crates/window/tests/it/storage.rs`
- `frontend/src/lib/storage.test.ts`, `nav.test.ts`

## 8. Not in scope

- Freeing space inside `irori.db` or a space's log. That needs an online version of the compaction floor rule, which is its own design.
- Automatic clean-up of any kind.
- The WebView2 profile (`%LOCALAPPDATA%\com.stephenshorton.irori`). Its path belongs to the shell, not the window, and it is in use while the app runs.
- Two things the survey turned up that need their own cards:
  - One space log on the PC holds 225,060 `task.left` rows, which looks like a write loop.
  - `updates/` kept 9 installers even though `download_verified` deletes earlier ones.
