//! The Storage page (card #728, docs/design/storage.md): what Irori keeps
//! on this device and how much space each kind takes, each kind's items to
//! browse, and the clean-outs the person asks for. Nothing here runs by
//! itself (board.md "No retention rules").
//!
//! Irori's log (`irori.db`) and the shared spaces' logs can't be trimmed:
//! their rows are signed and synced, and a row deleted here breaks sync
//! with every other device (store `insert_one`'s gap rule). So the page
//! shows what they hold and offers what frees nothing but tidies the lists
//! (Clear done cards and threads, #138), and the files around them are what
//! it deletes.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use coord::Task;
use gocompat::time::GoTime;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::ts;

/// Irori's own log: cards, threads, Talk, conversations, messages.
pub const LOG: &str = "log";
/// The shared spaces' logs (`spaces/`), one file each.
pub const SPACES: &str = "spaces";
/// Attachments (`files/`).
pub const FILES: &str = "files";
/// Agents' worktrees' build output (`<tree>/target`).
pub const BUILDS: &str = "builds";
/// Downloaded installers (`updates/`).
pub const UPDATES: &str = "updates";
/// Irori's logs and crash reports.
pub const LOGS: &str = "logs";
/// Agents' briefs (`briefs/`).
pub const BRIEFS: &str = "briefs";
/// What older versions moved aside: the v0.3 log backup, reset folders.
pub const BACKUPS: &str = "backups";
/// Copies of attachments saved out for an agent or the person.
pub const RECEIVED: &str = "received";
/// The speech model (`voice/`).
pub const VOICE: &str = "voice";
/// Patches and summaries capture keeps (`blobs/`): never deleted.
pub const CAPTURE: &str = "capture";
/// Claude Code's own conversations (`~/.claude/projects`): Claude's
/// folder, shown, never deleted by Irori.
pub const CLAUDE: &str = "claude";

/// Why an item stays: Irori writes it now.
pub const IN_USE: &str = "Irori is writing to it now";
/// Why the sweep keeps a worktree whose card isn't done (card #1118).
pub const WAITING_FOR_DONE: &str = "waiting for Done";
/// Why a brief stays.
pub const CARD_OPEN: &str = "its card is still open";
/// Why a settled card's brief stays (card #1215): its agent runs or rests,
/// and a wake reads its settings file again.
pub const AGENT_RESTING: &str = "its agent may wake and reads it again";
/// Why an installer stays.
pub const STAGED: &str = "it's the update waiting to install";

/// One kind of thing Irori keeps, as the page's overview lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageKind {
    /// [`LOG`], [`FILES`], ...
    pub id: String,
    /// Bytes on disk (0 until measured, see `measured`).
    pub bytes: i64,
    /// How many items it holds (files, rows, trees).
    pub items: i64,
    /// Whether `bytes` is known: build output and Claude's folder are
    /// measured when the page asks for their items.
    pub measured: bool,
    /// Where it is on disk.
    pub path: String,
}

/// What Irori keeps on this device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageReport {
    /// Irori's data folder.
    pub dir: String,
    pub kinds: Vec<StorageKind>,
}

/// One item of a kind: a file, a folder, a space, a tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageItem {
    /// What a clean names it by: an attachment's hash, a file name, a tree's
    /// folder, a card id.
    pub id: String,
    /// What the person reads.
    pub name: String,
    pub bytes: i64,
    /// Last changed (a file), or when it was attached.
    #[specta(type = ts::Time)]
    pub at: GoTime,
    /// A second line: the cards it belongs to, who sent it, ...
    pub detail: String,
    /// The cards on this device it belongs to (ids), to open.
    pub cards: Vec<String>,
    /// Whether a clean takes it.
    pub cleanable: bool,
    /// Why it stays, when it does.
    pub keep: String,
    /// An attachment this device attached itself: other devices may have
    /// no copy, so deleting it here may lose it.
    pub only_here: bool,
    /// Why the sweep keeps a worktree (card #1118), when it does: "waiting
    /// for Done", or "canceled, 3 unpushed commits". A process holding it
    /// is `holders`.
    pub pinned: String,
    /// Processes with their folder in this worktree: End it ends one.
    pub holders: Vec<StorageHolder>,
    /// A canceled card's worktree, which the person may delete whole,
    /// unpushed work and all.
    pub delete_anyway: bool,
}

/// A process that holds a worktree (card #1118).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageHolder {
    pub pid: u32,
    pub name: String,
}

/// One item a clean left.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageKept {
    pub id: String,
    pub why: String,
}

/// What a clean did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleaned {
    pub removed: i64,
    pub freed: i64,
    pub kept: Vec<StorageKept>,
}

/// A share of Irori's log by what its rows are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogPart {
    /// conversations, messages, cards, talk, notes, code, agents or other.
    pub part: String,
    pub rows: i64,
    pub bytes: i64,
}

/// A card or thread the person cleared from the lists (#138).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClearedCard {
    pub id: String,
    pub short: String,
    pub title: String,
    /// Its space's name, "" for Personal.
    pub board: String,
    pub thread: bool,
    #[specta(type = ts::Time)]
    pub updated_at: GoTime,
}

/// The share of the log a row kind belongs to.
pub fn log_part(kind: &str) -> &'static str {
    let head = kind.split('.').next().unwrap_or("");
    match head {
        "capture" => "conversations",
        "message" | "delivery" | "wake" | "turn" => "messages",
        "task" | "blocker" | "lease" | "wait" | "pr" | "attach" | "ask" | "space" | "board" => {
            "cards"
        }
        "talk" | "call" => "talk",
        "note" => "notes",
        "commit" | "desk" => "code",
        "agent" | "spawn" => "agents",
        "assistant" => "assistant",
        _ => "other",
    }
}

/// The bytes and files under `p`, links not followed.
pub fn dir_size(p: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            // The entry's own metadata doesn't follow links, and costs no
            // second lookup.
            let Ok(m) = e.metadata() else {
                continue;
            };
            if link(&m) {
                continue;
            }
            if m.is_dir() {
                stack.push(e.path());
            } else {
                bytes += m.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

fn link(m: &fs::Metadata) -> bool {
    if m.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: a junction.
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn file_len(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn mtime(p: &Path) -> GoTime {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| GoTime::unix(d.as_secs() as i64, d.subsec_nanos() as i64))
        .unwrap_or_default()
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A log's files: the db and its WAL and shared memory.
fn db_files(db: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|s| PathBuf::from(format!("{}{s}", db.display())))
        .collect()
}

/// The entries of `dir` that are files, by name.
fn files_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// A log or crash file in the data folder, and whether Irori writes it now.
fn log_file(name: &str) -> Option<bool> {
    let current = name == irori_core::diag::LOG_NAME || name == irori_core::diag::CRASH_NAME;
    let rotated = |base: &str| {
        name.strip_prefix(base)
            .and_then(|r| r.strip_prefix('.'))
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    };
    if current {
        return Some(true);
    }
    if rotated(irori_core::diag::LOG_NAME)
        || rotated(irori_core::diag::CRASH_NAME)
        || (name.starts_with("crash-") && name.ends_with(".log"))
        || name == "gui-stdout.log"
        || name == "gui-stderr.log"
    {
        return Some(false);
    }
    None
}

impl Window {
    fn data(&self) -> PathBuf {
        self.core.data_dir().to_path_buf()
    }

    fn files_dir(&self) -> PathBuf {
        self.data().join("files")
    }

    fn claude_projects(&self) -> PathBuf {
        Path::new(self.core.presence().claude_dir()).join("projects")
    }

    /// Every card here, by id: the private log's and each space's.
    fn storage_cards(&self) -> HashMap<String, (Task, String)> {
        let mut out: HashMap<String, (Task, String)> = self
            .core
            .coord()
            .list_tasks(&coord::TaskFilter::default())
            .into_iter()
            .map(|t| (t.id.clone(), (t, String::new())))
            .collect();
        for b in self.core.board_list() {
            if let Ok(bc) = self.core.board_coord(&b.board_id) {
                for t in bc.list_tasks(&coord::TaskFilter::default()) {
                    out.entry(t.id.clone())
                        .or_insert_with(|| (t, b.name.clone()));
                }
            }
        }
        out
    }

    /// StorageService.Report: each kind's size here. Build output and
    /// Claude's folder take a walk of many files, so they come unmeasured
    /// and the page asks for their items.
    pub fn storage_report(&self) -> Result<StorageReport, String> {
        let data = self.data();
        let kind = |id: &str, bytes: u64, items: u64, path: &Path| StorageKind {
            id: id.into(),
            bytes: bytes as i64,
            items: items as i64,
            measured: true,
            path: text(path),
        };
        let mut kinds = Vec::new();

        let db = data.join("irori.db");
        let log_bytes: u64 = db_files(&db).iter().map(|p| file_len(p)).sum();
        kinds.push(kind(LOG, log_bytes, 0, &db));

        let spaces = self.storage_items(SPACES)?;
        kinds.push(kind(
            SPACES,
            spaces.iter().map(|i| i.bytes as u64).sum(),
            spaces.len() as u64,
            &data.join(irori_core::SPACES_DIR),
        ));

        let files = self.core.files().list();
        kinds.push(kind(
            FILES,
            files.iter().map(|(_, n)| n).sum(),
            files.len() as u64,
            &self.files_dir(),
        ));

        kinds.push(StorageKind {
            id: BUILDS.into(),
            items: self.core.build_trees().len() as i64,
            measured: false,
            ..StorageKind::default()
        });

        for id in [UPDATES, LOGS, BRIEFS, BACKUPS, RECEIVED, VOICE] {
            let items = self.storage_items(id)?;
            let path = match id {
                UPDATES => data.join("updates"),
                BRIEFS => data.join("briefs"),
                RECEIVED => data.join(blob::RECEIVED_DIR),
                VOICE => data.join("voice"),
                _ => data.clone(),
            };
            kinds.push(kind(
                id,
                items.iter().map(|i| i.bytes as u64).sum(),
                items.len() as u64,
                &path,
            ));
        }

        let blobs = data.join("blobs");
        let (b, n) = dir_size(&blobs);
        kinds.push(kind(CAPTURE, b, n, &blobs));

        kinds.push(StorageKind {
            id: CLAUDE.into(),
            measured: false,
            path: text(&self.claude_projects()),
            ..StorageKind::default()
        });

        Ok(StorageReport {
            dir: text(&data),
            kinds,
        })
    }

    /// StorageService.Log: Irori's log by what its rows are, largest first.
    pub fn storage_log(&self) -> Result<Vec<LogPart>, String> {
        let mut parts: HashMap<&'static str, LogPart> = HashMap::new();
        for k in self.core.store().kind_sizes().map_err(|e| e.to_string())? {
            let p = log_part(&k.kind);
            let e = parts.entry(p).or_insert_with(|| LogPart {
                part: p.into(),
                ..LogPart::default()
            });
            e.rows += k.rows;
            e.bytes += k.bytes;
        }
        let mut out: Vec<LogPart> = parts.into_values().collect();
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.part.cmp(&b.part)));
        Ok(out)
    }

    /// StorageService.Items: one kind's items, largest first.
    pub fn storage_items(&self, kind: &str) -> Result<Vec<StorageItem>, String> {
        let mut out = match kind {
            SPACES => self.space_items(),
            FILES => self.file_items(),
            BUILDS => self.build_items(),
            UPDATES => self.update_items(),
            LOGS => self.log_items(),
            BRIEFS => self.brief_items(),
            BACKUPS => self.backup_items(),
            RECEIVED => self.received_items(),
            VOICE => self.voice_items(),
            CLAUDE => self.claude_items(),
            LOG | CAPTURE => Vec::new(),
            _ => return Err(format!("no storage kind {kind}")),
        };
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    fn space_items(&self) -> Vec<StorageItem> {
        let dir = self.data().join(irori_core::SPACES_DIR);
        let names: HashMap<String, String> = self
            .core
            .board_list()
            .into_iter()
            .map(|b| (b.board_id, b.name))
            .collect();
        let mut by_id: HashMap<String, (u64, GoTime)> = HashMap::new();
        for (name, p) in files_in(&dir) {
            let id = name
                .split_once('.')
                .map(|(id, _)| id.to_string())
                .unwrap_or(name.clone());
            let e = by_id.entry(id).or_default();
            e.0 += file_len(&p);
            if name.ends_with(".db") {
                e.1 = mtime(&p);
            }
        }
        by_id
            .into_iter()
            .map(|(id, (bytes, at))| {
                let shown = names.get(&id).cloned();
                StorageItem {
                    name: shown.clone().unwrap_or_else(|| "Direct messages".into()),
                    detail: if shown.is_some() {
                        String::new()
                    } else {
                        "A space for direct messages with another person, or one this device has left".into()
                    },
                    id,
                    bytes: bytes as i64,
                    at,
                    keep: if shown.is_some() {
                        "Shared with the space's members: forget it from its menu on Boards to remove it here".into()
                    } else {
                        "Shared with its members: a direct-message space, or one you left that goes once another member has your leave".into()
                    },
                    ..StorageItem::default()
                }
            })
            .collect()
    }

    /// Each attachment here, named from the rows that offered it.
    fn file_items(&self) -> Vec<StorageItem> {
        let files = self.core.files().list();
        if files.is_empty() {
            return Vec::new();
        }
        let want: HashSet<&str> = files.iter().map(|(id, _)| id.as_str()).collect();
        let me = self.core.device_id().to_string();
        let mut offers: HashMap<String, Offer> = HashMap::new();
        let mut stores = vec![self.core.store().clone()];
        for id in self.core.boards() {
            stores.extend(self.core.board_store(&id));
        }
        for st in stores {
            for ev in offered_rows(&st) {
                let Ok(o) = serde_json::from_slice::<OfferRow>(&ev.payload) else {
                    continue;
                };
                if !want.contains(o.sha256.as_str()) {
                    continue;
                }
                let e = offers.entry(o.sha256.clone()).or_default();
                if e.name.is_empty() {
                    e.name = o.name;
                    e.at = GoTime::unix_milli(ev.hlc.wall_ms());
                }
                if ev.device_id == me {
                    e.mine = true;
                } else {
                    e.others = true;
                    if e.from.is_empty() {
                        e.from = ev.device_id.clone();
                    }
                }
            }
        }
        let chat = crate::assistant::chat_files(&self.core);
        files
            .into_iter()
            .map(|(id, bytes)| {
                let o = offers.remove(&id).unwrap_or_default();
                // A file only the assistant chat has (card #999) was never
                // offered, so nothing else holds it.
                let chat_only = o
                    .name
                    .is_empty()
                    .then(|| chat.get(&id).map(|r| r.name.clone()))
                    .flatten();
                let path = self.core.files().path(&id);
                let from = if o.from.is_empty() {
                    String::new()
                } else {
                    match self.core.device_label(&o.from) {
                        l if l.is_empty() => String::new(),
                        l => format!("From {l}"),
                    }
                };
                StorageItem {
                    name: blob::safe_name(chat_only.as_deref().unwrap_or(&o.name), &id),
                    at: if o.at.is_zero() { mtime(&path) } else { o.at },
                    detail: if chat_only.is_some() {
                        "From the assistant chat".into()
                    } else if o.mine && !o.others {
                        "Attached on this device".into()
                    } else {
                        from
                    },
                    only_here: chat_only.is_some() || (o.mine && !o.others),
                    cleanable: true,
                    id,
                    bytes: bytes as i64,
                    ..StorageItem::default()
                }
            })
            .collect()
    }

    fn build_items(&self) -> Vec<StorageItem> {
        self.build_items_of(None)
    }

    /// [`Self::build_items`], only the trees in `want` when given: a clean
    /// of one tree measures that one.
    fn build_items_of(&self, want: Option<&HashSet<String>>) -> Vec<StorageItem> {
        let cards = self.storage_cards();
        self.core
            .build_trees()
            .into_iter()
            .filter(|t| want.is_none_or(|w| w.contains(&t.tree)))
            .map(|t| {
                let target = Path::new(&t.tree).join("target");
                let (bytes, _) = dir_size(&target);
                let refs: Vec<String> = t
                    .cards
                    .iter()
                    .filter(|id| cards.contains_key(*id))
                    .cloned()
                    .collect();
                let titles: Vec<String> = t
                    .cards
                    .iter()
                    .filter_map(|id| {
                        cards
                            .get(id)
                            .map(|(c, _)| format!("{} {}", c.short_ref(), c.title))
                    })
                    .collect();
                let holders: Vec<StorageHolder> = t
                    .holders
                    .iter()
                    .map(|h| StorageHolder {
                        pid: h.pid,
                        name: h.name.clone(),
                    })
                    .collect();
                let held = !t.live && !t.holders.is_empty();
                let tree_path = Path::new(&t.tree);
                // Why the sweep keeps it: a process (named), a card that
                // isn't done, or a canceled card's work.
                let pinned = if t.live || held {
                    String::new()
                } else if t.canceled {
                    match irori_core::treeclean::unpushed_head(tree_path) {
                        Ok(0) => "canceled".to_string(),
                        Ok(n) => format!(
                            "canceled, {n} unpushed commit{}",
                            if n == 1 { "" } else { "s" }
                        ),
                        Err(e) => format!("canceled, Irori couldn't read its commits ({e})"),
                    }
                } else if t.waiting_for_done {
                    WAITING_FOR_DONE.to_string()
                } else if !t.settled {
                    CARD_OPEN.to_string()
                } else {
                    String::new()
                };
                StorageItem {
                    name: tree_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| t.tree.clone()),
                    bytes: bytes as i64,
                    at: mtime(&target),
                    detail: titles.join(" · "),
                    cards: refs,
                    cleanable: bytes > 0 && !t.live && !held,
                    keep: if t.live {
                        irori_core::LIVE_TREE.into()
                    } else if held {
                        irori_core::treeclean::held_by(&t.holders)
                    } else if bytes == 0 {
                        "no build output".into()
                    } else {
                        String::new()
                    },
                    pinned,
                    holders: if t.live { Vec::new() } else { holders },
                    delete_anyway: t.canceled && !t.live && !held,
                    id: t.tree,
                    ..StorageItem::default()
                }
            })
            .collect()
    }

    fn update_items(&self) -> Vec<StorageItem> {
        let st = self.seams.updater.status();
        let staged = matches!(st.phase.as_str(), "downloading" | "ready" | "installing")
            && !st.version.is_empty();
        files_in(&self.data().join("updates"))
            .into_iter()
            .map(|(name, p)| {
                let keep = staged && name.contains(&st.version);
                StorageItem {
                    bytes: file_len(&p) as i64,
                    at: mtime(&p),
                    cleanable: !keep,
                    keep: if keep { STAGED.into() } else { String::new() },
                    id: name.clone(),
                    name,
                    ..StorageItem::default()
                }
            })
            .collect()
    }

    fn log_items(&self) -> Vec<StorageItem> {
        files_in(&self.data())
            .into_iter()
            .filter_map(|(name, p)| {
                let current = log_file(&name)?;
                Some(StorageItem {
                    bytes: file_len(&p) as i64,
                    at: mtime(&p),
                    cleanable: !current,
                    keep: if current {
                        IN_USE.into()
                    } else {
                        String::new()
                    },
                    id: name.clone(),
                    name,
                    ..StorageItem::default()
                })
            })
            .collect()
    }

    /// Agents' briefs, one item per card: the brief and settings of each
    /// time Irori started an agent for it.
    fn brief_items(&self) -> Vec<StorageItem> {
        let cards = self.storage_cards();
        let kept = self.core.kept_brief_stems();
        let mut by_card: HashMap<String, (u64, GoTime, usize)> = HashMap::new();
        let mut agent_kept: HashSet<String> = HashSet::new();
        for (name, p) in files_in(&self.data().join("briefs")) {
            let Some(id) = brief_card(&name) else {
                continue;
            };
            if brief_stem(&name).is_some_and(|s| kept.contains(s)) {
                agent_kept.insert(id.to_string());
            }
            let e = by_card.entry(id.to_string()).or_default();
            e.0 += file_len(&p);
            let at = mtime(&p);
            if at.unix_nano() > e.1.unix_nano() {
                e.1 = at;
            }
            if name.ends_with(".md") {
                e.2 += 1;
            }
        }
        by_card
            .into_iter()
            .map(|(id, (bytes, at, starts))| {
                let card = cards.get(&id).map(|(t, _)| t);
                let open = card.is_some_and(|t| !irori_core::buildclean::settled(t));
                let resting = !open && agent_kept.contains(&id);
                StorageItem {
                    name: card
                        .map(|t| format!("{} {}", t.short_ref(), t.title))
                        .unwrap_or_else(|| format!("task:{id}")),
                    detail: format!(
                        "{starts} agent start{}{}",
                        if starts == 1 { "" } else { "s" },
                        if card.is_none() {
                            " · its card isn't on this device"
                        } else {
                            ""
                        }
                    ),
                    cards: card.map(|t| vec![t.id.clone()]).unwrap_or_default(),
                    bytes: bytes as i64,
                    at,
                    cleanable: !open && !resting,
                    keep: if open {
                        CARD_OPEN.into()
                    } else if resting {
                        AGENT_RESTING.into()
                    } else {
                        String::new()
                    },
                    id,
                    ..StorageItem::default()
                }
            })
            .collect()
    }

    fn backup_items(&self) -> Vec<StorageItem> {
        let data = self.data();
        let mut out = Vec::new();
        let old = data.join(format!("irori.db{}", store::BACKUP_SUFFIX));
        if old.is_file() {
            out.push(StorageItem {
                id: text(old.file_name().unwrap_or_default().as_ref()),
                name: "The log before Irori 0.4".into(),
                detail: "A copy kept when Irori moved to a new log format".into(),
                bytes: file_len(&old) as i64,
                at: mtime(&old),
                cleanable: true,
                ..StorageItem::default()
            });
        }
        if let Ok(rd) = fs::read_dir(&data) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("reset-") && e.file_type().is_ok_and(|t| t.is_dir()) {
                    let (b, _) = dir_size(&e.path());
                    out.push(StorageItem {
                        id: name.clone(),
                        name: "Before a reset".into(),
                        detail: format!("The log and identity a reset moved aside ({name})"),
                        bytes: b as i64,
                        at: mtime(&e.path()),
                        cleanable: true,
                        ..StorageItem::default()
                    });
                }
            }
        }
        out
    }

    fn received_items(&self) -> Vec<StorageItem> {
        let dir = self.data().join(blob::RECEIVED_DIR);
        let Ok(rd) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        rd.flatten()
            .map(|e| {
                let p = e.path();
                let (b, _) = if p.is_dir() {
                    dir_size(&p)
                } else {
                    (file_len(&p), 1)
                };
                let names: Vec<String> = fs::read_dir(&p)
                    .map(|rd| {
                        rd.flatten()
                            .map(|f| f.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                let id = e.file_name().to_string_lossy().into_owned();
                StorageItem {
                    name: if names.is_empty() {
                        id.clone()
                    } else {
                        names.join(", ")
                    },
                    detail: "A copy saved out for an agent; the attachment itself stays".into(),
                    id,
                    bytes: b as i64,
                    at: mtime(&p),
                    cleanable: true,
                    ..StorageItem::default()
                }
            })
            .collect()
    }

    fn voice_items(&self) -> Vec<StorageItem> {
        files_in(&self.data().join("voice"))
            .into_iter()
            .map(|(name, p)| StorageItem {
                detail: "Downloads again the next time you use voice".into(),
                bytes: file_len(&p) as i64,
                at: mtime(&p),
                cleanable: true,
                id: name.clone(),
                name,
                ..StorageItem::default()
            })
            .collect()
    }

    fn claude_items(&self) -> Vec<StorageItem> {
        let dir = self.claude_projects();
        let Ok(rd) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        rd.flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| {
                let (b, n) = dir_size(&e.path());
                let id = e.file_name().to_string_lossy().into_owned();
                StorageItem {
                    name: id.clone(),
                    detail: format!("{n} file{}", if n == 1 { "" } else { "s" }),
                    id,
                    bytes: b as i64,
                    at: mtime(&e.path()),
                    keep: "Claude Code's folder: Irori reads it and never deletes from it".into(),
                    ..StorageItem::default()
                }
            })
            .collect()
    }

    /// StorageService.EndHolder (card #1118): ends process `pid`, which has
    /// its folder in worktree `tree`. Says who was ended.
    pub fn storage_end_holder(&self, tree: &str, pid: u32) -> Result<String, String> {
        self.core
            .end_tree_holder(crate::trim(tree), pid)
            .map(|h| h.label())
    }

    /// StorageService.DeleteTree (card #1118): deletes a canceled card's
    /// worktree, unpushed work and all. Its branch stays.
    pub fn storage_delete_tree(&self, tree: &str) -> Result<(), String> {
        self.core.delete_canceled_tree(crate::trim(tree))
    }

    /// StorageService.Clean: deletes the items given of one kind, or with
    /// none given every one a clean may take. What can't be deleted stays
    /// and says why.
    pub fn storage_clean(&self, kind: &str, ids: &[String]) -> Result<StorageCleaned, String> {
        let want: HashSet<&str> = ids.iter().map(|s| crate::trim(s)).collect();
        let items = if kind == BUILDS && !want.is_empty() {
            let keys: HashSet<String> =
                want.iter().map(|t| irori_core::treeclean::key(t)).collect();
            self.build_items_of(Some(&keys))
        } else {
            self.storage_items(kind)?
        };
        let want: HashSet<String> = if kind == BUILDS {
            want.iter().map(|t| irori_core::treeclean::key(t)).collect()
        } else {
            want.iter().map(|t| t.to_string()).collect()
        };
        let mut out = StorageCleaned::default();
        for it in items {
            if !want.is_empty() && !want.contains(it.id.as_str()) {
                continue;
            }
            if !it.cleanable {
                if !want.is_empty() {
                    out.kept.push(StorageKept {
                        id: it.id,
                        why: if it.keep.is_empty() {
                            "this can't be deleted here".into()
                        } else {
                            it.keep
                        },
                    });
                }
                continue;
            }
            match self.clean_one(kind, &it) {
                Ok(Some(why)) => out.kept.push(StorageKept { id: it.id, why }),
                Ok(None) => {
                    out.removed += 1;
                    out.freed += it.bytes;
                }
                Err(why) => out.kept.push(StorageKept { id: it.id, why }),
            }
        }
        Ok(out)
    }

    /// Deletes one item; `Some(why)` when it stays after all.
    fn clean_one(&self, kind: &str, it: &StorageItem) -> Result<Option<String>, String> {
        let data = self.data();
        let rm_file = |p: PathBuf| fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()));
        let rm_dir =
            |p: PathBuf| fs::remove_dir_all(&p).map_err(|e| format!("{}: {e}", p.display()));
        // Names come back from the page: only a plain name in its folder.
        let plain = |n: &str| !n.is_empty() && !n.contains(['/', '\\']) && n != "." && n != "..";
        match kind {
            FILES => self
                .core
                .files()
                .remove(&it.id)
                .map(|_| None)
                .map_err(|e| e.to_string()),
            BUILDS => match self.core.clear_build(&it.id)? {
                irori_core::buildclean::Cleared::Gone => Ok(None),
                irori_core::buildclean::Cleared::Kept(why) => Ok(Some(why.to_string())),
            },
            UPDATES if plain(&it.id) => rm_file(data.join("updates").join(&it.id)).map(|_| None),
            LOGS if plain(&it.id) && log_file(&it.id) == Some(false) => {
                rm_file(data.join(&it.id)).map(|_| None)
            }
            BRIEFS if plain(&it.id) => {
                // An agent that runs or rests keeps its files (card #1215),
                // even one that started resting after the list was read.
                let kept = self.core.kept_brief_stems();
                let mut stayed = false;
                for (name, p) in files_in(&data.join("briefs")) {
                    if brief_card(&name) != Some(it.id.as_str()) {
                        continue;
                    }
                    if brief_stem(&name).is_some_and(|s| kept.contains(s)) {
                        stayed = true;
                        continue;
                    }
                    rm_file(p)?;
                }
                Ok(stayed.then(|| AGENT_RESTING.to_string()))
            }
            BACKUPS if plain(&it.id) => {
                let p = data.join(&it.id);
                if p.is_dir() { rm_dir(p) } else { rm_file(p) }.map(|_| None)
            }
            RECEIVED if plain(&it.id) => {
                let p = data.join(blob::RECEIVED_DIR).join(&it.id);
                if p.is_dir() { rm_dir(p) } else { rm_file(p) }.map(|_| None)
            }
            VOICE if plain(&it.id) => rm_file(data.join("voice").join(&it.id)).map(|_| None),
            _ => Ok(Some("this can't be deleted here".into())),
        }
    }

    /// StorageService.Cleared: the cards and threads the person cleared
    /// from the lists (#138), newest first.
    pub fn storage_cleared(&self) -> Vec<ClearedCard> {
        let cleared = self.core.coord().archived_ids();
        if cleared.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<ClearedCard> = self
            .storage_cards()
            .into_values()
            // Reopened ones are back in the lists, as the lists say.
            .filter(|(t, _)| {
                cleared.contains(&t.id) && coord::is_terminal(coord::effective_state(t))
            })
            .map(|(t, board)| ClearedCard {
                short: t.short_ref(),
                thread: crate::threads::is_thread(&t),
                title: t.title.clone(),
                updated_at: t.updated_at,
                id: t.id,
                board,
            })
            .collect();
        out.sort_by(|a, b| {
            b.updated_at
                .unix_nano()
                .cmp(&a.updated_at.unix_nano())
                .then(a.id.cmp(&b.id))
        });
        out
    }

    /// StorageService.Unclear: puts cleared cards back in the lists.
    pub fn storage_unclear(&self, ids: &[String]) -> Result<i64, String> {
        let ids: Vec<String> = ids
            .iter()
            .map(|s| crate::trim(s).to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            return Ok(0);
        }
        let n = self
            .core
            .coord()
            .archive(&self.user_key(), &ids, false)
            .map_err(|e| e.to_string())?;
        Ok(n as i64)
    }

    /// The resolved threads a clear would take: closed, not cleared yet,
    /// last changed more than `older_than_secs` ago (0: all).
    fn threads_to_clear(&self, older_than_secs: i64) -> Vec<String> {
        let cleared = self.core.coord().archived_ids();
        let cut = self.now_ms() - older_than_secs.max(0) * 1000;
        let mut out: Vec<String> = self
            .storage_cards()
            .into_values()
            .map(|(t, _)| t)
            .filter(|t| {
                crate::threads::is_thread(t)
                    && !t.withdrawn
                    && coord::is_terminal(coord::effective_state(t))
                    && !cleared.contains(&t.id)
                    && (older_than_secs <= 0 || t.updated_at.unix_milli_value() < cut)
            })
            .map(|t| t.id)
            .collect();
        out.sort();
        out
    }

    /// StorageService.ClearThreadsCount: how many resolved threads a clear
    /// by age would take.
    pub fn clear_threads_count(&self, older_than_secs: i64) -> i64 {
        self.threads_to_clear(older_than_secs).len() as i64
    }

    /// StorageService.ClearThreads: takes resolved threads out of the
    /// lists, as Clear done cards does cards (#138). They stay in the log.
    pub fn clear_threads(&self, older_than_secs: i64) -> Result<i64, String> {
        let ids = self.threads_to_clear(older_than_secs);
        if ids.is_empty() {
            return Ok(0);
        }
        let n = self
            .core
            .coord()
            .archive(&self.user_key(), &ids, true)
            .map_err(|e| e.to_string())?;
        Ok(n as i64)
    }
}

/// The card id a brief file is for: `<id>-<generation>.md` or
/// `<id>-<generation>.settings.json`.
/// The `<card>-<generation>` stem of a brief or settings file name.
pub fn brief_stem(name: &str) -> Option<&str> {
    brief_card(name)?;
    name.strip_suffix(".settings.json")
        .or_else(|| name.strip_suffix(".md"))
}

pub fn brief_card(name: &str) -> Option<&str> {
    let stem = name
        .strip_suffix(".settings.json")
        .or_else(|| name.strip_suffix(".md"))?;
    let (id, generation) = stem.rsplit_once('-')?;
    (!id.is_empty() && !generation.is_empty() && generation.chars().all(|c| c.is_ascii_digit()))
        .then_some(id)
}

#[derive(Default)]
struct Offer {
    name: String,
    at: GoTime,
    from: String,
    mine: bool,
    others: bool,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct OfferRow {
    sha256: String,
    name: String,
}

/// Every attach.offered row in `st`.
fn offered_rows(st: &store::Store) -> Vec<store::Event> {
    let mut out = Vec::new();
    let mut f = store::Filter {
        kinds: vec![store::kinds::ATTACH_OFFERED.into()],
        limit: store::MAX_LIST_LIMIT,
        ..store::Filter::default()
    };
    while let Ok(page) = st.list(&f) {
        let full = page.len() as i64 >= store::MAX_LIST_LIMIT;
        if let Some(last) = page.last() {
            f.after = last.cursor();
        }
        out.extend(page);
        if !full {
            break;
        }
    }
    out
}
