//! `irori-dev space-to-personal`: card #640's move run on a copy of a real
//! data dir, with an offline Core (no network), so its counts can be
//! checked before it runs on the person's own devices.
//!
//! For each space it prints, from the space's fold and then from Personal's
//! (after the move, and again after a restart): cards, comments, steps and
//! ticked steps, blockers and open ones, subtasks and threads; the space
//! log's own row counts; and whether every `tag#N` finds the same card. It
//! fails on any difference.
//!
//! A Mac copy unseals the device key only in a debug build with
//! `IRORI_DEV_KEK_DIR` set to the original data dir; a PC copy only on the
//! PC (DPAPI).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use irori_core::{Core, MoveOpts, Options};
use store::{Filter, kinds};

pub struct Opts {
    pub home: PathBuf,
    pub claude: PathBuf,
    pub spaces: Vec<String>,
    pub allow_missing_files: bool,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let mut o = Opts {
            home: PathBuf::new(),
            claude: PathBuf::new(),
            spaces: Vec::new(),
            allow_missing_files: false,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
            match a.as_str() {
                "--home" => o.home = PathBuf::from(val()?),
                "--claude" => o.claude = PathBuf::from(val()?),
                "--space" => o.spaces.push(val()?),
                "--allow-missing-files" => o.allow_missing_files = true,
                _ => return Err(format!("unknown argument {a}")),
            }
        }
        if o.home.as_os_str().is_empty() || o.claude.as_os_str().is_empty() {
            return Err("space-to-personal needs --home <copy> and --claude <empty dir>".into());
        }
        if o.spaces.is_empty() {
            return Err("space-to-personal needs at least one --space <id>".into());
        }
        Ok(o)
    }
}

/// The counts the move must keep, summed over a space's cards.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Counts {
    cards: usize,
    comments: i64,
    steps: usize,
    ticked: usize,
    blockers: usize,
    open_blockers: usize,
    subtasks: usize,
    threads: usize,
    done: usize,
}

impl Counts {
    fn add(&mut self, t: &coord::Task) {
        self.cards += 1;
        self.comments += t.comments;
        self.steps += t.steps.len();
        self.ticked += t.steps.iter().filter(|s| s.done).count();
        self.blockers += t.blockers.len();
        self.open_blockers += t.blockers.iter().filter(|b| b.open()).count();
        self.subtasks += t.subtasks.len();
        self.threads += usize::from(t.ask.as_ref().is_some_and(|a| a.thread));
        self.done += usize::from(coord::is_terminal(&t.state));
    }

    fn line(&self) -> String {
        format!(
            "cards {}, comments {}, steps {} ({} ticked), blockers {} ({} open), subtasks {}, threads {}, closed {}",
            self.cards,
            self.comments,
            self.steps,
            self.ticked,
            self.blockers,
            self.open_blockers,
            self.subtasks,
            self.threads,
            self.done
        )
    }
}

/// One space before the move: its tag, each card's id and ref, the counts.
struct Before {
    id: String,
    tag: String,
    cards: Vec<(String, String)>,
    counts: Counts,
    per_card: BTreeMap<String, Counts>,
}

fn start(o: &Opts) -> Result<Core, String> {
    Core::start(Options {
        data_dir: o.home.clone(),
        claude_dir: o.claude.clone(),
        version: "space-to-personal".into(),
        headless: true,
        pair_net: Some(irori_core::offline::offline_net()),
        no_home: true,
        ..Options::default()
    })
    .map_err(|e| format!("start: {e}"))
}

fn per_card(tasks: &[coord::Task]) -> (Counts, BTreeMap<String, Counts>) {
    let mut all = Counts::default();
    let mut each = BTreeMap::new();
    for t in tasks {
        all.add(t);
        let mut c = Counts::default();
        c.add(t);
        each.insert(t.id.clone(), c);
    }
    (all, each)
}

fn raw_rows(core: &Core, id: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    let Some(st) = core.board_store(id) else {
        return out;
    };
    let mut f = Filter {
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    loop {
        let Ok(evs) = st.list(&f) else {
            return out;
        };
        for ev in &evs {
            *out.entry(ev.kind.clone()).or_default() += 1;
        }
        if (evs.len() as i64) < f.limit {
            return out;
        }
        f.after = evs[evs.len() - 1].cursor();
    }
}

/// Personal's fold of a space's cards, compared with `b`; returns the
/// lines that differ.
fn check_after(core: &Core, b: &Before, when: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let tasks: Vec<coord::Task> = b
        .cards
        .iter()
        .filter_map(|(id, _)| core.coord().get_task(id))
        .collect();
    let (counts, each) = per_card(&tasks);
    println!("  {when}: {}", counts.line());
    if counts != b.counts {
        bad.push(format!("{when}: totals differ"));
    }
    for (id, want) in &b.per_card {
        match each.get(id) {
            Some(got) if got == want => {}
            Some(got) => bad.push(format!("{when}: {id}: {got:?} (was {want:?})")),
            None => bad.push(format!("{when}: {id} is missing")),
        }
    }
    for (id, r) in &b.cards {
        match core.task_ref(r) {
            Ok(got) if &got == id => {}
            Ok(got) => bad.push(format!("{when}: {r} finds {got}, not {id}")),
            Err(e) => bad.push(format!("{when}: {r}: {e}")),
        }
    }
    if core.boards().contains(&b.id) {
        bad.push(format!("{when}: the space is still open"));
    }
    bad
}

pub fn run(o: &Opts) -> ExitCode {
    if let Err(e) = crate::verify::refuse_live(&o.home) {
        eprintln!("irori-dev: {e}");
        return ExitCode::from(2);
    }
    let core = match start(o) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("irori-dev: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut before = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for id in &o.spaces {
        let co = match core.board_coord(id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("irori-dev: space {id}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let tasks = co.list_tasks(&coord::TaskFilter::default());
        let (counts, each) = per_card(&tasks);
        let tag = core
            .space_boards(id)
            .ok()
            .and_then(|b| b.first().map(|x| x.tag.clone()))
            .unwrap_or_default();
        let cards = tasks.iter().map(|t| (t.id.clone(), t.short_ref())).collect();
        println!("space {id} ({tag})");
        let raw = raw_rows(&core, id);
        let kinds_line = |ks: &[&str]| {
            ks.iter()
                .map(|k| format!("{k} {}", raw.get(*k).copied().unwrap_or(0)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "  log: {}",
            kinds_line(&[
                kinds::TASK_CREATED,
                kinds::TASK_COMMENT,
                kinds::TASK_STEPS,
                kinds::TASK_STEP,
                kinds::TASK_STATUS
            ])
        );
        println!("  space fold: {}", counts.line());
        before.push(Before {
            id: id.clone(),
            tag,
            cards,
            counts,
            per_card: each,
        });
    }
    for b in &before {
        let opts = MoveOpts {
            keep_space: false,
            allow_missing_files: o.allow_missing_files,
        };
        match core.move_space_to_personal(&b.id, &opts) {
            Ok(m) => {
                println!(
                    "moved {} ({}): board {}, {} cards, {} rows copied, {} already, files {} ({} missing), open cards the person made: {}",
                    b.id,
                    b.tag,
                    m.board_id,
                    m.cards,
                    m.rows_copied,
                    m.rows_already,
                    m.files,
                    m.files_missing.len(),
                    if m.open_by_person.is_empty() {
                        "none".to_string()
                    } else {
                        m.open_by_person.join(" ")
                    }
                );
                for f in &m.files_missing {
                    println!("  missing file: {f}");
                }
                let left: Vec<String> = m
                    .rows_left
                    .iter()
                    .map(|(k, n)| format!("{k} {n}"))
                    .collect();
                println!("  left behind: {}", left.join(", "));
            }
            Err(e) => {
                println!("FAIL {} ({}): {e}", b.id, b.tag);
                bad.push(format!("{}: {e}", b.tag));
            }
        }
        bad.extend(check_after(&core, b, &format!("{} after the move", b.tag)));
    }
    let tags: Vec<String> = core
        .board_list()
        .into_iter()
        .map(|s| format!("{} ({})", s.name, s.tag))
        .collect();
    println!("spaces left: {}", tags.join(", "));
    if let Ok(ps) = core.personal_boards() {
        let names: Vec<String> = ps.iter().map(|p| format!("{}:{}", p.tag, p.name)).collect();
        println!("Personal's boards: {}", names.join(", "));
    }
    // Every count again from a fresh replay of the private log.
    let Ok(core) = Arc::try_unwrap(core) else {
        eprintln!("irori-dev: the Core is still shared");
        return ExitCode::FAILURE;
    };
    if let Err(e) = core.close() {
        eprintln!("irori-dev: close: {e}");
    }
    match start(o) {
        Ok(core) => {
            for b in &before {
                bad.extend(check_after(&core, b, &format!("{} after a restart", b.tag)));
            }
            let _ = core.close();
        }
        Err(e) => bad.push(format!("restart: {e}")),
    }
    if bad.is_empty() {
        println!("OK: every card, comment and step is in Personal, with the same refs");
        ExitCode::SUCCESS
    } else {
        for l in bad.iter().take(40) {
            println!("FAIL {l}");
        }
        ExitCode::FAILURE
    }
}
