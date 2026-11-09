//! Card #565: finished means gone. Once a card is settled, this device
//! removes the worktree its agent worked in, build output and all, unless
//! that could lose work. [`crate::buildclean`] only ever cleared `target/`.
//!
//! **Scope.** Only trees Irori made: `<repo>/.claude/worktrees/<name>`, by
//! [`make`] for a card's agent, [`crate::helpertree::make`] for a helper,
//! or [`reopen`]. Never a linked worktree the person made
//! (`~/projects/irori-release`), never a main checkout. Main checkouts'
//! `target/` stay out of this too. No card's agent is started with
//! `claude -w`: a tree Claude Code makes is one `claude rm` deletes when
//! the agent ends, without any of these gates, so the window's end never
//! runs `rm` for a job whose folder is a tree still on disk
//! ([`tree_on_disk`]).
//!
//! **Gates**, all local, no fetch, cheapest first ([`remove`]). Any doubt
//! keeps the tree and says why:
//! 0. a removal git left half done is finished ([`begun_path`]); a tree
//!    a past removal left at `<name>.irori-removing` is moved back with
//!    `git worktree move` ([`restore`], and every pass for every tree
//!    whatever its card's state, [`restore_all`]);
//! 1. the tree is one of ours ([`ours`]);
//! 2. its card is settled and no unsettled card names it
//!    ([`crate::buildclean::settled_worktrees`]), asked again just before
//!    the folder moves ([`Check::recheck`]);
//! 3. no live agent is in it: `claude agents`' cwds and the attached
//!    sessions' cwds, passed in as [`Check::live`], and no process's
//!    current folder ([`cwd_in`]);
//! 4. `git status` with `status.showUntrackedFiles=all` and `--ignored`
//!    shows nothing but build output: an untracked file is work, and so is
//!    an ignored one other than a `target/` cargo made (`CACHEDIR.TAG` in
//!    it), a `node_modules/` with a `package.json` beside it,
//!    `__pycache__/`, `frontend/dist/` or buildclean's
//!    `target.irori-clearing/` ([`regenerable`]); and no file is
//!    skip-worktree or assume-unchanged, which `git status` doesn't look
//!    at;
//! 5. every commit HEAD has been on (HEAD and its reflog, which goes with
//!    the tree) and every tip of its own refs (`refs/worktree/`,
//!    `refs/bisect/`, `refs/rewritten/`) is on a remote branch, so a
//!    detached HEAD on a pushed commit goes and an unpushed or reset-away
//!    commit stays; with no HEAD reflog to read, it stays;
//! 6. no other worktree or repo is inside it, it isn't locked (`git
//!    worktree lock`), and no link (symlink, junction) is anywhere in it,
//!    build output included;
//! 7. no cargo holds a lock in `target/`, and it has been idle for
//!    [`Check::idle`]; then `target/` is cleared;
//! 8. gate 2's recheck;
//! 9. `git worktree move` to `<name>.irori-removing` ([`aside_path`]):
//!    Windows refuses while a native process is in the folder or has a
//!    file open in it, so the move is the in-use probe; and from here on
//!    nothing new can start at the tree's path, while git still knows the
//!    tree where it is;
//! 10. gates 3 to 6 again, on the moved tree (a process's folder under
//!     either name), since clearing a big `target/` takes minutes; any of
//!     them failing moves it back;
//! 11. `git worktree remove` of the moved tree, without `--force`, and no
//!     `git worktree prune`: that would drop every entry whose folder is
//!     missing, a worktree the person made on an unmounted drive included.
//!     The branch is kept: only the folder goes.
//!
//! **What this can't see.** Windows has no way to list every process's
//! current directory without admin rights. Gate 9 covers native ones.
//! MSYS processes (Git Bash, the Claude Code Bash tool's shell) open their
//! folder so it can be renamed, so for them gate 3 asks Git for Windows'
//! own `sh` for its `/proc` cwds; an MSYS2 or Cygwin install of its own,
//! or a sandbox's, isn't seen, and a tree goes from under such a shell
//! (with no work in it: gates 4 to 6 still hold). A file another process
//! opened with delete sharing doesn't stop the move or the delete either.
//! Where names a tree by its path as text, so a Where spelling the same
//! folder another way (a symlink, an 8.3 name) doesn't hold it. And a
//! half-done removal is finished by file times: a file written into the
//! remnant with an older time goes with it. Any refusal from `git
//! worktree remove` (submodules, a file in use) is a kept tree and the
//! next sweep looks again.
//!
//! **Who holds it (card #1118).** Gate 3 names the processes it finds
//! ([`holders`]: "held by cat.exe (pid 4242)"), on Windows through Git's `sh`,
//! so the kept line, the card and Storage's End it ([`end_holder`]) say what
//! to end. A native Windows process is still only "in use". A canceled
//! card's tree that gates 4 to 6 keep can be deleted whole at the person's
//! word ([`remove_anyway`]).
//!
//! **Attribution.** A card whose agent never wrote Where (a thread, a shared
//! board's card, `mentions`, `454-talk`) still has to reach its tree, so each
//! tree is written in the [`Book`] when it is spawned, made or first seen
//! under a session holding a card. A tree in neither Where nor the book is
//! left alone: nothing says whose it is.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::buildclean::{self, CLEARING, Cleared};
use crate::helpertree::WORKTREES;

/// How long a tree kept for its work waits before it is looked at again:
/// a push is what frees it, and nothing hurries that.
pub const KEPT_EVERY: Duration = Duration::from_secs(60 * 60);
/// What a tree's name gets for its last checks and its removal
/// ([`aside_path`]): `git worktree move` takes it there, so it stays a
/// registered worktree and nothing new can start at its old path.
const ASIDE: &str = ".irori-removing";
/// The mark beside a tree whose removal git has begun ([`begun_path`]).
const BEGUN: &str = ".irori-begun";
/// The book's file in the data folder.
const BOOK: &str = "worktrees.json";

/// `p` as a key for comparing paths: forward slashes, no `\\?\` prefix or
/// trailing slash, and on Windows one case. git prints `C:/x`, `claude
/// agents` prints `C:\x`, and a drive letter's case varies.
pub fn key(p: &str) -> String {
    let p = p.strip_prefix(r"\\?\").unwrap_or(p).replace('\\', "/");
    let p = p.trim_end_matches('/');
    if cfg!(windows) {
        p.to_lowercase()
    } else {
        p.to_string()
    }
}

/// Whether `p` is `dir` or inside it.
pub fn under(p: &str, dir: &str) -> bool {
    let (p, dir) = (key(p), key(dir));
    !dir.is_empty() && (p == dir || p.starts_with(&format!("{dir}/")))
}

/// The tree `dir` is in, by its path alone: `<repo>/.claude/worktrees/<name>`
/// as a [`key`]. Reads nothing on disk.
pub fn tree_of(dir: &str) -> Option<String> {
    let k = key(dir);
    let mark = format!("/{WORKTREES}/");
    let at = k.find(&mark)? + mark.len();
    let leaf = k[at..].split('/').next().filter(|l| !l.is_empty())?;
    Some(k[..at + leaf.len()].to_string())
}

/// Whether `dir` is in a tree of ours shape (`.claude/worktrees/<name>`)
/// that is on disk, at its name or aside for its removal: a folder no
/// `claude rm` may be run for (card #565), as `rm` deletes a job's
/// worktree without Irori's checks.
pub fn tree_on_disk(dir: &str) -> bool {
    tree_of(dir).is_some_and(|t| {
        let t = Path::new(&t);
        t.exists() || aside_path(t).exists() || begun_path(t).exists()
    })
}

/// Whether `root` sits in a Claude Code job's temp folder
/// (`~/.claude/jobs/<id>/tmp/...`): a clone that goes when its job does,
/// so nothing new starts there (card #565).
pub fn temp_root(root: &str) -> bool {
    key(root).contains("/.claude/jobs/")
}

/// The best of `roots` to start in: the first that is a folder here, a main
/// checkout before a linked worktree. Never a temp job clone (card #565):
/// with only one of those, there is no checkout here ([`only_temp`] names
/// it for the log).
pub fn pick_root(roots: &[String]) -> Option<String> {
    let rank = |r: &String| {
        if Path::new(r).join(".git").is_dir() {
            0
        } else if tree_of(r).is_some() {
            2
        } else {
            1
        }
    };
    roots
        .iter()
        .filter(|r| !temp_root(r) && Path::new(r).is_dir())
        .min_by_key(|r| rank(r))
        .cloned()
}

/// A temp job clone among `roots` that is a folder here: what
/// [`pick_root`] passed over.
pub fn only_temp(roots: &[String]) -> Option<String> {
    roots
        .iter()
        .find(|r| temp_root(r) && Path::new(r).is_dir())
        .cloned()
}

fn git_cmd(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    // Long paths: a deep target/ is past MAX_PATH on Windows.
    cmd.args(["-c", "core.longpaths=true", "-C"])
        .arg(dir)
        .args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW, as helpertree's git.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    git_raw(dir, args).map(|s| s.trim().to_string())
}

/// [`git`]'s output as it is, untrimmed (a `-z` list ends in a NUL).
fn git_raw(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_cmd(dir, args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// [`git`] with `input` on its stdin (a rev list too long for a command
/// line on Windows).
fn git_stdin(dir: &Path, args: &[&str], input: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = git_cmd(dir, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    let mut stdin = child.stdin.take().ok_or("git: no stdin")?;
    let text = input.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(text.as_bytes()));
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    let wrote = writer.join().map_err(|_| "git: stdin writer panicked")?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    wrote.map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The shape of a tree of ours: a folder at `.claude/worktrees/<name>`.
fn shaped(tree: &Path) -> bool {
    let p = tree.to_string_lossy();
    tree_of(&p).is_some_and(|t| t == key(&p))
}

/// The main checkout of `tree`, when it is a linked worktree Irori made:
/// `.git` a file, and git agreeing it is a worktree of the repo that holds
/// it.
pub fn ours(tree: &Path) -> Option<PathBuf> {
    if !shaped(tree) || !tree.join(".git").is_file() {
        return None;
    }
    let main = crate::helpertree::main_root(tree)?;
    let top = git(tree, &["rev-parse", "--show-toplevel"]).ok()?;
    // Real paths: git resolves symlinks and 8.3 names, a card's path may not.
    let real = |p: &Path| {
        fs::canonicalize(p)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    };
    let (top, here) = (real(Path::new(&top))?, real(tree)?);
    let home = real(&main.join(WORKTREES))?;
    (key(&top) == key(&here) && under(&top, &home)).then_some(main)
}

/// See [`Check::recheck`].
pub type RecheckFn<'a> = dyn Fn(&str) -> Result<(), String> + Sync + 'a;

/// What [`remove`] needs to know from outside.
#[derive(Clone)]
pub struct Check<'a> {
    /// The folders live agents and attached sessions are in.
    pub live: &'a [String],
    /// How long `target/` must be untouched.
    pub idle: Duration,
    pub now: SystemTime,
    /// Asked last, just before the folder moves (card #565 finding 7): a
    /// fresh look at the cards and agents, since clearing a big `target/`
    /// takes a while and a reopened card's agent may have started in the
    /// tree meanwhile. `Err` says why the tree stays.
    pub recheck: Option<&'a RecheckFn<'a>>,
}

impl std::fmt::Debug for Check<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Check")
            .field("live", &self.live)
            .field("idle", &self.idle)
            .field("recheck", &self.recheck.is_some())
            .finish()
    }
}

/// What became of one tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The worktree is gone; its branch stays.
    Removed,
    /// Not a tree Irori made, or already gone: nothing was touched.
    NotOurs,
    /// In use, or git declined: the next sweep looks again.
    Busy(String),
    /// It holds work, or something Irori can't judge: said on the card
    /// once, after "it has".
    Work(String),
    /// At a tree's path but not a git worktree any more: nothing here can
    /// judge it, so it stays, is said once and isn't looked at again.
    Broken(String),
}

/// Ignored folders that are only ever build output or fetched packages,
/// so they may go with a tree, by path from its top. Any other ignored
/// path (`.env`, `notes.local`, a scratch folder) is work (card #565
/// finding 1). `target` and `node_modules` count anywhere, but only as
/// what made them says ([`regenerable`]).
/// Tauri's build script writes `app/gen/schemas/` on every build.
const REGEN_PATHS: &[&str] = &["frontend/dist", "app/gen/schemas", CLEARING];

/// Whether an ignored entry `p` of `tree`, as `git status` prints it (a
/// folder ends in `/`), is regenerable output: inside `frontend/dist/`,
/// Tauri's `app/gen/schemas/` or buildclean's own `target.irori-clearing/`,
/// a `__pycache__/`, a `target/` cargo made (it holds `CACHEDIR.TAG`, as
/// [`crate::buildclean`] asks), or a `node_modules/` with a
/// `package.json` beside it. A hand-made `docs/target/notes.md` is work.
pub fn regenerable(tree: &Path, p: &str) -> bool {
    let p = p.replace('\\', "/");
    let Some(dir) = p.strip_suffix('/') else {
        // A file: only inside one of those folders.
        return p
            .rsplit_once('/')
            .is_some_and(|(d, _)| regenerable(tree, &format!("{d}/")));
    };
    if REGEN_PATHS
        .iter()
        .any(|r| dir == *r || dir.starts_with(&format!("{r}/")))
    {
        return true;
    }
    let segs: Vec<&str> = dir.split('/').collect();
    if segs.iter().any(|s| s.is_empty() || *s == "." || *s == "..") {
        return false;
    }
    (0..segs.len()).any(|i| {
        let at = |n: usize| segs[..n].iter().fold(tree.to_path_buf(), |p, s| p.join(s));
        match segs[i] {
            "__pycache__" => true,
            "target" => at(i + 1).join("CACHEDIR.TAG").is_file(),
            "node_modules" => at(i).join("package.json").is_file(),
            _ => false,
        }
    })
}

/// Work `git status` can see, ignored files included, whatever the repo's
/// `status.showUntrackedFiles` says (findings 1 and 3). `None` when there
/// is none.
fn status_work(tree: &Path) -> Result<Option<String>, String> {
    let out = git_raw(
        tree,
        &[
            "-c",
            "status.showUntrackedFiles=all",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?;
    let (mut changed, mut ignored) = (0, Vec::new());
    let mut parts = out.split('\0').filter(|e| !e.is_empty());
    while let Some(e) = parts.next() {
        let (code, path) = e.split_at(e.len().min(3));
        if code.starts_with('R') || code.starts_with('C') {
            parts.next(); // the old name
        }
        if code == "!! " {
            // An ignored link (a `target` symlinked to a shared build
            // folder) is named as a link: it keeps the tree either way.
            let full = tree.join(path.trim_end_matches('/'));
            if fs::symlink_metadata(&full).is_ok_and(|m| is_link(&m)) {
                return Ok(Some(format!(
                    "a link inside ({})",
                    path.trim_end_matches('/')
                )));
            }
            if !regenerable(tree, path) {
                ignored.push(path.to_string());
            }
        } else {
            changed += 1;
        }
    }
    if changed > 0 {
        return Ok(Some(format!("{changed} uncommitted or untracked file(s)")));
    }
    if let Some(first) = ignored.first() {
        return Ok(Some(format!(
            "{} ignored file(s) that aren't build output, such as {first}",
            ignored.len()
        )));
    }
    Ok(None)
}

/// Files git is told not to look at, skip-worktree (sparse checkout too)
/// or assume-unchanged: a change to one shows in no `git status`, and `git
/// worktree remove` would take it.
fn hidden_files(tree: &Path) -> Result<Option<String>, String> {
    let out = git_raw(tree, &["ls-files", "-v", "-z"])?;
    let hidden: Vec<&str> = out
        .split('\0')
        .filter(|e| {
            e.chars()
                .next()
                .is_some_and(|t| t == 'S' || t.is_ascii_lowercase())
        })
        .map(|e| e.get(2..).unwrap_or(e))
        .collect();
    Ok(hidden.first().map(|first| {
        format!(
            "{} file(s) git is told not to look at (skip-worktree or assume-unchanged), such as {first}",
            hidden.len()
        )
    }))
}

/// Refs that live in a worktree's own admin folder and go with it.
const OWN_REFS: &[&str] = &["refs/worktree", "refs/bisect", "refs/rewritten"];

/// Commits this tree's HEAD has been on that no remote branch holds
/// (finding 4): its HEAD reflog and its own refs ([`OWN_REFS`]) go with
/// the tree, so a commit only they reach would be lost. HEAD itself is
/// one of them.
fn unpushed_history(tree: &Path) -> Result<Option<String>, String> {
    let head = git(tree, &["rev-parse", "HEAD"])?;
    let mut revs = vec![head.clone()];
    let log = PathBuf::from(git(tree, &["rev-parse", "--git-path", "logs/HEAD"])?);
    let log = if log.is_absolute() {
        log
    } else {
        tree.join(log)
    };
    match fs::read_to_string(&log) {
        Ok(text) => {
            for line in text.lines() {
                let mut f = line.split_whitespace();
                for sha in [f.next(), f.next()].into_iter().flatten() {
                    if sha.len() >= 40
                        && sha.bytes().all(|b| b.is_ascii_hexdigit())
                        && sha.bytes().any(|b| b != b'0')
                    {
                        revs.push(sha.to_string());
                    }
                }
            }
        }
        // No reflog (core.logAllRefUpdates off, or it was deleted): which
        // commits only it holds can't be known, so the tree stays.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "it has no HEAD reflog ({}), so Irori can't tell which commits only it holds",
                log.display()
            ));
        }
        Err(e) => return Err(format!("{}: {e}", log.display())),
    }
    // Its own refs (refs/worktree, refs/bisect) go with it too.
    let mut args = vec!["for-each-ref", "--format=%(objectname)"];
    args.extend(OWN_REFS);
    let own: Vec<String> = git(tree, &args)?
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    revs.extend(own.iter().cloned());
    revs.sort();
    revs.dedup();
    let mut input = revs.join("\n");
    input.push_str("\n--not\n--remotes\n");
    let out = git_stdin(tree, &["rev-list", "--stdin"], &input)?;
    let lost: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let n = lost.len();
    Ok(match n {
        0 => None,
        _ if lost.contains(&head.as_str()) => {
            Some(format!("{n} commit(s) that aren't on any remote branch"))
        }
        _ if own.iter().any(|r| lost.contains(&r.as_str())) => Some(format!(
            "{n} commit(s) that aren't on any remote branch, held by its own refs (refs/worktree, refs/bisect), though its HEAD is pushed"
        )),
        // HEAD is pushed; an amend, rebase or reset left these behind.
        _ => Some(format!(
            "{n} earlier commit(s) in its history (amended, rebased or reset away) that aren't on any remote branch, though its HEAD is pushed"
        )),
    })
}

/// The most entries [`scan`] looks at before it gives up and keeps the tree.
const SCAN_MAX: usize = 300_000;

/// Whether `m` is a link of any kind: a symlink, or on Windows any reparse
/// point (a junction, a mount point).
pub(crate) fn is_link(m: &fs::Metadata) -> bool {
    if m.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Whether link `path` is a plain symlink to somewhere inside `tree`
/// (`node_modules/.bin/x -> ../pkg/bin/x`): git removes such a link as a
/// file, and nothing outside goes with it. A junction or other reparse
/// point on Windows never is: git can't remove one cleanly (finding 9).
/// A target that can't be read isn't.
fn link_within(path: &Path, m: &fs::Metadata, tree: &Path) -> bool {
    if cfg!(windows) || !m.file_type().is_symlink() {
        return false;
    }
    let Ok(to) = fs::read_link(path) else {
        return false;
    };
    let to = match path.parent() {
        Some(dir) if to.is_relative() => dir.join(to),
        _ => to,
    };
    // Lexically, `..` and all: the target may not exist.
    let mut out = PathBuf::new();
    for c in to.components() {
        match c {
            std::path::Component::ParentDir => {
                if !out.pop() {
                    return false;
                }
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    let inside = under(&out.to_string_lossy(), &tree.to_string_lossy());
    // And for real, where it exists: a link inside to a link outside.
    let real_ok = match (fs::canonicalize(&out), fs::canonicalize(tree)) {
        (Ok(r), Ok(t)) => under(&r.to_string_lossy(), &t.to_string_lossy()),
        (Err(_), _) => true,
        _ => false,
    };
    inside && real_ok
}

/// Looks through the whole tree, build output included, for what `git
/// worktree remove` would take or half take with it: another repo or
/// worktree inside (finding 2), or a link (finding 9: a junction to a
/// shared folder, which git can't remove and must never delete through).
fn scan(tree: &Path) -> Option<String> {
    scan_dir(tree, tree, SCAN_MAX, false)
}

/// Build output can't hold anything that isn't cargo's: before a
/// `target/` (or a leftover `target.irori-clearing/`) under `tree` is
/// deleted, it must hold no repo or worktree of its own, `.git` at its top
/// included, and no link out of the tree. A clone an agent made under
/// `target/tmp` would otherwise go with the build output. The one `.git`
/// allowed is in a build script's `OUT_DIR` ([`in_build_out`]).
pub(crate) fn scan_build(tree: &Path, dir: &Path) -> Option<String> {
    if dir.join(".git").exists() {
        let rel = dir
            .strip_prefix(tree)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        return Some(format!("another repo or worktree inside ({rel}/.git)"));
    }
    scan_dir(dir, tree, SCAN_BUILD_MAX, true)
}

/// Whether `rel`, a path under a `target/`, is inside a build script's
/// `OUT_DIR` (`<profile>/build/<crate>-<hash>/out/`, a target triple
/// before the profile or not). Cargo makes that folder for the build
/// script and nothing else writes there, so a `.git` in it is a vendored
/// source the script copied (whisper-rs-sys copies whisper.cpp with its
/// `.git`), not an agent's clone.
/// `target/tmp/` (`CARGO_TARGET_TMPDIR`) is the tests' own and never
/// counts.
pub fn in_build_out(rel: &Path) -> bool {
    let segs: Vec<_> = rel.components().map(|c| c.as_os_str()).collect();
    segs.first().is_some_and(|s| *s != "tmp")
        && (1..segs.len())
            .any(|i| segs[i] == "build" && segs.get(i + 2).is_some_and(|s| *s == "out"))
}

/// How many entries [`scan_build`] walks before it gives up and keeps the
/// build output: a big `target/` holds far more files than a source tree.
const SCAN_BUILD_MAX: usize = 5_000_000;

/// Walks `root`, a folder in `tree`, for a `.git` below `root` and links
/// that leave `tree`. With `build`, `root` is build output and a `.git` in
/// a build script's `OUT_DIR` ([`in_build_out`]) is cargo's.
fn scan_dir(root: &Path, tree: &Path, max: usize, build: bool) -> Option<String> {
    let unread = |p: &Path, e: std::io::Error| {
        Some(format!(
            "a folder Irori couldn't read ({}: {e})",
            p.display()
        ))
    };
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => return unread(&dir, e),
        };
        for e in rd {
            let e = match e {
                Ok(e) => e,
                Err(e) => return unread(&dir, e),
            };
            seen += 1;
            if seen > max {
                return Some(format!("more than {max} files, too many to check"));
            }
            let path = e.path();
            let m = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => return unread(&path, e),
            };
            let rel = path
                .strip_prefix(tree)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if is_link(&m) && !link_within(&path, &m, tree) {
                return Some(format!("a link inside ({rel})"));
            }
            if e.file_name() == ".git"
                && dir != root
                && !(build && path.strip_prefix(root).is_ok_and(in_build_out))
            {
                return Some(format!("another repo or worktree inside ({rel})"));
            }
            if m.is_dir() {
                stack.push(path);
            }
        }
    }
    None
}

/// What `git worktree list` says keeps `tree`: another worktree of the
/// same repo registered inside it (finding 2), or `tree` locked with `git
/// worktree lock`, which is the person saying keep it.
fn worktree_list(main: &Path, tree: &Path) -> Result<Option<String>, String> {
    let list = git(main, &["worktree", "list", "--porcelain"])?;
    let real = |p: &Path| {
        fs::canonicalize(p)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string_lossy().into_owned())
    };
    let here = real(tree);
    let mut this = false;
    for l in list.lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            let there = real(Path::new(p));
            this = key(&there) == key(&here);
            if !this && under(&there, &here) {
                return Ok(Some(format!("another worktree inside ({p})")));
            }
        } else if this && (l == "locked" || l.starts_with("locked ")) {
            return Ok(Some("a lock (git worktree lock)".into()));
        }
    }
    Ok(None)
}

/// `p` without Windows' `\\?\` prefix, which git can't make a worktree
/// under ("could not create leading directories"). Other paths are as given.
pub fn plain(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        p.to_string()
    }
}

/// Whether `folder` is a repo's main checkout: `.git` is a folder, or a
/// file (`--separate-git-dir`) whose git dir is the repo's own rather than a
/// linked worktree's. Any doubt answers yes, so a code card gets a tree of
/// its own rather than starting bare in someone's checkout.
pub fn is_main_checkout(folder: &Path) -> bool {
    let dot = folder.join(".git");
    if dot.is_dir() {
        return true;
    }
    if !dot.is_file() {
        return false;
    }
    let (Ok(dir), Ok(common)) = (
        git(folder, &["rev-parse", "--absolute-git-dir"]),
        git(
            folder,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ),
    ) else {
        return true;
    };
    key(dir.trim()) == key(common.trim())
}

/// Whether `path` is a tree moved aside for its last checks or a removal
/// mark: never a place to start an agent.
pub fn being_removed(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with(ASIDE) || s.ends_with(BEGUN)
}

/// Where `tree` sits for its last checks and its removal: `<name>.irori-removing`
/// beside it, moved there by `git worktree move`, so it is still a
/// registered worktree and nothing new can start at `tree` meanwhile.
pub fn aside_path(tree: &Path) -> PathBuf {
    let mut name = tree.as_os_str().to_owned();
    name.push(ASIDE);
    PathBuf::from(name)
}

/// `git worktree move` with retries for a moment, as something that just
/// looked at the folder may still hold it.
fn move_tree(main: &Path, from: &Path, to: &Path, tries: u32) -> Result<(), String> {
    let (f, t) = (from.to_string_lossy(), to.to_string_lossy());
    let mut last = String::new();
    for n in 0..tries.max(1) {
        if n > 0 {
            std::thread::sleep(Duration::from_millis(100));
        }
        match git(main, &["worktree", "move", &f, &t]) {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
        if !from.exists() {
            break;
        }
    }
    Err(last)
}

/// Puts back a tree a past [`remove`] left at its [`aside_path`] (its
/// move back failed, or Irori stopped part way): the name is the only
/// record, so every look starts here, and so does [`reopen`]. It moves
/// back with `git worktree move`, so git's own record follows it. `Err`
/// says why it is still there. A removal git has begun on ([`begun_path`])
/// is [`remove`]'s to finish, not put back.
pub fn restore(tree: &Path) -> Result<(), String> {
    let aside = aside_path(tree);
    if !aside.exists() {
        return Ok(());
    }
    if tree.exists() {
        return Err(format!(
            "both {} and {} exist, so Irori left both alone",
            tree.display(),
            aside.display()
        ));
    }
    if !registered(&aside) {
        return Err(format!(
            "it is at {} and isn't a git worktree there, so Irori left it alone",
            aside.display()
        ));
    }
    let Some(main) = ours(&aside) else {
        return Err(format!(
            "it is at {} and isn't a tree of ours there, so Irori left it alone",
            aside.display()
        ));
    };
    move_tree(&main, &aside, tree, 20).map_err(|e| {
        format!(
            "it is at {} and Irori couldn't move it back yet: {e}",
            aside.display()
        )
    })?;
    crate::logf(format_args!(
        "trees: moved {} back from {}",
        tree.display(),
        aside.display()
    ));
    Ok(())
}

/// Puts back every tree under `home` (a repo's `.claude/worktrees`) that a
/// past [`remove`] left at its [`aside_path`], whatever its card's state
/// now: a card reopened meanwhile drops its tree from the settled list,
/// and [`remove`] would never get to restore it. Also drops a removal mark
/// ([`begun_path`]) whose tree is gone from both names. Says what stays.
/// Run only while no removal is under way: the sweep runs it only while
/// no pass is running (`builds_busy` guards both).
pub fn restore_all(home: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(home) else {
        return Vec::new();
    };
    let mut left = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // A removal mark whose tree is gone: Irori stopped between git's
        // removal and dropping the mark. Nothing is left to finish.
        if let Some(leaf) = name.strip_suffix(BEGUN).filter(|l| !l.is_empty()) {
            let tree = home.join(leaf);
            if e.path().is_file() && !tree.exists() && !aside_path(&tree).exists() {
                let _ = fs::remove_file(e.path());
            }
            continue;
        }
        let Some(leaf) = name.strip_suffix(ASIDE).filter(|l| !l.is_empty()) else {
            continue;
        };
        if !e.path().is_dir() {
            continue;
        }
        let tree = home.join(leaf);
        if begun_path(&tree).is_file() && !registered(&e.path()) {
            left.push(format!(
                "git began removing {} and left part of it; the next look at its card finishes it",
                e.path().display()
            ));
            continue;
        }
        let _ = fs::remove_file(begun_path(&tree));
        if let Err(why) = restore(&tree) {
            left.push(why);
        }
    }
    left
}

/// Removes `tree` if every gate in the module doc holds. Gate 2 is the
/// caller's, asked again through [`Check::recheck`]. Touches nothing but
/// `tree` (and its [`aside_path`] and [`begun_path`]) and its own entry in
/// its repo's worktree list. On any doubt it keeps the tree.
pub fn remove(tree: &Path, c: &Check) -> Verdict {
    if let Some(v) = finish_begun(tree, c) {
        return v;
    }
    if let Err(why) = restore(tree) {
        return Verdict::Busy(why);
    }
    if !tree.is_dir() || !shaped(tree) {
        return Verdict::NotOurs;
    }
    // `.git` first: without that file, git would walk up and answer for the
    // main checkout around the folder.
    if !registered(tree) {
        return Verdict::Broken("it isn't a git worktree any more".into());
    }
    let Some(main) = ours(tree) else {
        return Verdict::NotOurs;
    };
    let path = tree.to_string_lossy().into_owned();
    if let Some(v) = in_use(&path, c) {
        return v;
    }
    let clearing = tree.join(CLEARING);
    if clearing.exists()
        && let Some(why) = scan_build(tree, &clearing)
    {
        return Verdict::Work(why);
    }
    if clearing.exists()
        && let Err(e) = fs::remove_dir_all(&clearing)
    {
        return Verdict::Busy(format!("{}: {e}", clearing.display()));
    }
    if let Some(v) = work_gates(&main, tree) {
        return v;
    }
    let target = tree.join("target");
    if fs::symlink_metadata(&target).is_ok_and(|m| is_link(&m)) {
        return Verdict::Work("a link inside (target)".into());
    }
    if target.is_dir() {
        if buildclean::recent(&target, c.idle, c.now) {
            return Verdict::Busy(BUILT_RECENTLY.into());
        }
        // Cargo's own output goes by rename first, so a build that starts
        // now sees it whole or not at all.
        match buildclean::clear(tree, c.idle, c.now) {
            Ok(Cleared::Gone | Cleared::Kept(buildclean::NO_TARGET)) => {}
            Ok(Cleared::Kept(buildclean::HOLDS_REPO)) => {
                return Verdict::Work(
                    scan_build(tree, &target).unwrap_or_else(|| buildclean::HOLDS_REPO.into()),
                );
            }
            Ok(Cleared::Kept(why)) => return Verdict::Busy(why.into()),
            Err(e) => return Verdict::Busy(e),
        }
    }
    if let Some(why) = scan(tree) {
        return Verdict::Work(why);
    }
    if let Some(f) = c.recheck
        && let Err(why) = f(&path)
    {
        return Verdict::Busy(why);
    }
    // Aside, by git, for the last checks and the removal (finding D): the
    // old path is gone, so no agent, shell or build can start in the tree
    // between those checks and git's removal, and a tree stranded here is
    // still a registered worktree that [`restore`] moves back. Windows
    // refuses the move while a native process is in the folder or has a
    // file open in it, so the move is also what finds those.
    let aside = aside_path(tree);
    if aside.exists() {
        return Verdict::Busy(format!("{} is in the way", aside.display()));
    }
    if let Err(e) = move_tree(&main, tree, &aside, 1) {
        crate::logf(format_args!(
            "trees: {} can't be moved aside yet: {e}",
            tree.display()
        ));
        if aside.exists() {
            // git moved it and then failed: the next look moves it back.
            return Verdict::Busy(format!("git worktree move: {e}"));
        }
        return Verdict::Busy("in use".into());
    }
    let moved = aside.to_string_lossy().into_owned();
    // Clearing a big `target/` takes minutes and the recheck waits on
    // `claude agents`: a shell, an editor or a hand-run build may have
    // come and gone meanwhile, leaving an ignored file or a detached
    // commit that `git worktree remove` would take without asking. So the
    // process and work gates run again, last, on the moved tree. A
    // process's folder may read as either name (Linux follows the folder,
    // an MSYS shell keeps the old name), so both are looked at.
    let last = in_use(&path, c)
        .or_else(|| in_use(&moved, c))
        .or_else(|| work_gates(&main, &aside))
        .or_else(|| scan(&aside).map(Verdict::Work));
    if let Some(v) = last {
        return put_back(&main, tree, v);
    }
    let admin = match git(&aside, &["rev-parse", "--absolute-git-dir"]) {
        Ok(a) => PathBuf::from(a),
        Err(e) => return put_back(&main, tree, Verdict::Busy(format!("git rev-parse: {e}"))),
    };
    // Written before git starts, so a removal git leaves half done (a file
    // held open part way through its delete) is finished by the next look
    // instead of being judged not a worktree.
    let begun = begun_path(tree);
    if let Err(e) = fs::write(&begun, "") {
        return put_back(
            &main,
            tree,
            Verdict::Busy(format!("{}: {e}", begun.display())),
        );
    }
    // No `git worktree prune` after it: `remove` drops this tree's own
    // entry, and a plain prune would drop every entry whose folder is
    // missing right now, a worktree the person made on a drive that isn't
    // mounted, or a tree of ours stranded aside, included.
    let removed = git(&main, &["worktree", "remove", &moved]);
    if admin.exists() {
        // git refused before it began (or the tree was moved back by a
        // reopen meanwhile): the tree is still whole.
        let _ = fs::remove_file(&begun);
        let why = match removed {
            Err(e) => format!("git worktree remove: {e}"),
            Ok(_) => "git left the folder behind".into(),
        };
        if !aside.exists() {
            return Verdict::Busy(format!("it was moved back while Irori looked ({why})"));
        }
        return put_back(&main, tree, Verdict::Busy(why));
    }
    if !aside.exists() {
        let _ = fs::remove_file(&begun);
        return Verdict::Removed;
    }
    // git has passed its point of no return: the entry is gone, so what is
    // left of the folder is what it was deleting when something held a file.
    finish(&aside, &begun)
}

/// How many commits `tree`'s HEAD has that no remote branch holds (card
/// #1118: Storage says it for a canceled card's tree).
pub fn unpushed_head(tree: &Path) -> Result<usize, String> {
    git(tree, &["rev-list", "--count", "HEAD", "--not", "--remotes"])?
        .parse()
        .map_err(|e| format!("git rev-list --count: {e}"))
}

/// Deletes a tree whose work the person has said to drop (card #1118:
/// Storage's "delete anyway" for a canceled card): `git worktree remove
/// --force`, so uncommitted and unpushed work goes with the folder; the
/// branch stays. What still holds: a tree of ours, nobody in it ([`in_use`]),
/// and no repo or link inside it ([`scan`]): git can't remove a junction
/// cleanly and must never delete through one. The caller checks the cards.
pub fn remove_anyway(tree: &Path, c: &Check) -> Result<(), String> {
    restore(tree)?;
    if !tree.is_dir() || !shaped(tree) || !registered(tree) {
        return Err("it isn't a worktree of ours".into());
    }
    let Some(main) = ours(tree) else {
        return Err("it isn't a worktree of ours".into());
    };
    let path = tree.to_string_lossy().into_owned();
    if let Some(Verdict::Busy(why)) = in_use(&path, c) {
        return Err(why);
    }
    if let Some(why) = worktree_list(&main, tree)? {
        return Err(why);
    }
    // The build output goes first, through the same checks as the sweep's.
    match buildclean::clear(tree, Duration::ZERO, c.now) {
        Ok(Cleared::Gone | Cleared::Kept(buildclean::NO_TARGET)) => {}
        Ok(Cleared::Kept(why)) => return Err(why.into()),
        Err(e) => return Err(e),
    }
    if let Some(why) = scan(tree) {
        return Err(why);
    }
    let removed = git(&main, &["worktree", "remove", "--force", &path]);
    if tree.exists() {
        if registered(tree) {
            // git refused before it began: the tree is whole.
            return Err(removed
                .err()
                .unwrap_or_else(|| "git left the folder behind".into()));
        }
        // git dropped its entry and a file held it part way (Windows): the
        // person said delete, so finish the folder, as the sweep's removal
        // does for a half-done one. Nothing else could judge it now.
        let mut last = String::new();
        for _ in 0..20 {
            match fs::remove_dir_all(tree) {
                Ok(()) => break,
                Err(_) if !tree.exists() => break,
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if tree.exists() {
            return Err(format!(
                "git removed its entry but not all of the folder ({last}); delete {} by hand",
                tree.display()
            ));
        }
    }
    crate::logf(format_args!(
        "trees: deleted {} at the person's word (canceled)",
        tree.display()
    ));
    Ok(())
}

/// Moves a tree [`remove`] moved aside back to `tree` and gives `v`, why
/// it stays. A move back that keeps failing leaves it at its
/// [`aside_path`], still a registered worktree, which [`restore`] puts
/// back on the next look.
fn put_back(main: &Path, tree: &Path, v: Verdict) -> Verdict {
    let aside = aside_path(tree);
    if !aside.exists() {
        // A reopen moved it back meanwhile.
        return match v {
            Verdict::Busy(why) => Verdict::Busy(why),
            _ => Verdict::Busy("it was moved back while Irori looked".into()),
        };
    }
    match move_tree(main, &aside, tree, 50) {
        Ok(()) => v,
        Err(e) => {
            crate::logf(format_args!(
                "trees: {} is stranded at {}: {e}; the next look moves it back",
                tree.display(),
                aside.display()
            ));
            Verdict::Busy(format!(
                "it is at {} and Irori couldn't move it back yet: {e}",
                aside.display()
            ))
        }
    }
}

/// Why a tree whose build was used lately stays: nothing to tell anyone,
/// it goes once the build is idle.
pub const BUILT_RECENTLY: &str = "built recently";

/// Whether `tree` is a registered worktree: its `.git` file is there and
/// git can follow it.
fn registered(tree: &Path) -> bool {
    tree.join(".git").is_file() && git(tree, &["rev-parse", "--git-dir"]).is_ok()
}

/// Gate 3: a live agent or session, or a process whose current folder is
/// in `path`.
fn in_use(path: &str, c: &Check) -> Option<Verdict> {
    if c.live.iter().any(|d| under(d, path)) {
        return Some(Verdict::Busy("an agent is in it".into()));
    }
    match holders(path) {
        Ok(h) if h.is_empty() => None,
        Ok(h) => Some(Verdict::Busy(held_by(&h))),
        Err(e) => Some(Verdict::Busy(e)),
    }
}

/// A process that has its current folder in a tree (card #1118): what keeps
/// the tree "in use", named so the person can end it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Holder {
    /// The operating system's process id (on Windows the Windows one, not
    /// the MSYS one).
    pub pid: u32,
    /// The program's name: `cat.exe`, `node`.
    pub name: String,
}

impl Holder {
    /// `cat.exe (pid 123)`.
    pub fn label(&self) -> String {
        format!("{} (pid {})", self.name, self.pid)
    }
}

/// How many holders [`held_by`] names before it counts the rest.
const HELD_NAMED: usize = 3;

/// The start of the reason [`held_by`] gives.
pub const HELD_BY: &str = "held by ";

/// Why a tree is kept when `holders` have their folder in it: names them,
/// or says "in use" when there are none to name.
pub fn held_by(holders: &[Holder]) -> String {
    if holders.is_empty() {
        return "in use".into();
    }
    let named: Vec<String> = holders.iter().take(HELD_NAMED).map(Holder::label).collect();
    let more = holders.len().saturating_sub(HELD_NAMED);
    let more = if more > 0 {
        format!(" and {more} more")
    } else {
        String::new()
    };
    format!("{HELD_BY}{}{more}", named.join(", "))
}

/// Gates 4 to 6's git part: anything git would lose with the tree. Run
/// before the clear and again just before the removal.
fn work_gates(main: &Path, tree: &Path) -> Option<Verdict> {
    let work = |r: Result<Option<String>, String>, what: &str| match r {
        Ok(None) => None,
        Ok(Some(why)) => Some(Verdict::Work(why)),
        Err(e) => Some(Verdict::Work(format!("{what}: {e}"))),
    };
    work(status_work(tree), "a status git couldn't give")
        .or_else(|| work(hidden_files(tree), "a file list git couldn't give"))
        .or_else(|| {
            work(
                unpushed_history(tree),
                "history Irori couldn't check is pushed",
            )
        })
        .or_else(|| {
            work(
                worktree_list(main, tree),
                "a worktree list git couldn't give",
            )
        })
}

/// Where the mark that a removal of `tree` has begun sits: beside it, never
/// in it (a file in it would be untracked work). The removal itself runs
/// on the tree at its [`aside_path`].
pub fn begun_path(tree: &Path) -> PathBuf {
    let mut name = tree.as_os_str().to_owned();
    name.push(BEGUN);
    PathBuf::from(name)
}

/// A removal a past look began and git left half done ([`begun_path`]),
/// on the tree at its [`aside_path`]. `None` when there is none, or git
/// never got going (the tree is still a worktree, so it is moved back and
/// judged as any other).
fn finish_begun(tree: &Path, c: &Check) -> Option<Verdict> {
    let begun = begun_path(tree);
    if !begun.is_file() || !shaped(tree) {
        return None;
    }
    let aside = aside_path(tree);
    if !aside.exists() {
        // Done, or moved back by a reopen before git began.
        let _ = fs::remove_file(&begun);
        return (!tree.exists()).then_some(Verdict::Removed);
    }
    if registered(&aside) {
        let _ = fs::remove_file(&begun);
        return None;
    }
    let moved = aside.to_string_lossy().into_owned();
    if let Some(v) = in_use(&moved, c).or_else(|| in_use(&tree.to_string_lossy(), c)) {
        return Some(v);
    }
    Some(finish(&aside, &begun))
}

/// Deletes what git left of `dir` (a tree at its [`aside_path`]) once its
/// entry is gone, unless anything in it is newer than the mark `begun`
/// (written there since, so work) or is a link. Every gate held just
/// before git began.
fn finish(dir: &Path, begun: &Path) -> Verdict {
    let since = match fs::metadata(begun).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(e) => {
            return Verdict::Broken(format!(
                "git began removing it and Irori can't read when ({e})"
            ));
        }
    };
    if let Some(why) = written_since(dir, since) {
        return Verdict::Broken(format!(
            "git began removing it, and {why}, so Irori left the rest at {}",
            dir.display()
        ));
    }
    let mut last = String::new();
    for _ in 0..20 {
        match fs::remove_dir_all(dir) {
            Ok(()) => break,
            Err(_) if !dir.exists() => break,
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if dir.exists() {
        return Verdict::Busy(format!(
            "git removed its entry but not all of it yet ({last}); the next look finishes it"
        ));
    }
    let _ = fs::remove_file(begun);
    crate::logf(format_args!(
        "trees: finished removing {} after git left part of it",
        dir.display()
    ));
    Verdict::Removed
}

/// What in `tree` makes finishing a half-done removal unsafe: a file
/// changed after `since`, a link, or a folder that can't be read.
fn written_since(tree: &Path, since: SystemTime) -> Option<String> {
    let mut stack = vec![tree.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => return Some(format!("{} can't be read ({e})", dir.display())),
        };
        for e in rd {
            let Ok(e) = e else {
                return Some(format!("{} can't be read", dir.display()));
            };
            let path = e.path();
            let Ok(m) = fs::symlink_metadata(&path) else {
                return Some(format!("{} can't be read", path.display()));
            };
            if is_link(&m) {
                return Some(format!("a link is inside ({})", path.display()));
            }
            if m.is_dir() {
                stack.push(path);
            } else if m.modified().map_or(true, |t| t > since) {
                return Some(format!("{} was written since", path.display()));
            }
        }
    }
    None
}

/// Whether any process here has its current folder in `tree` (finding 10).
/// On Windows that can't be listed without admin rights, so [`remove`]'s
/// `git worktree move` is what finds one. On Linux it is each `/proc/<pid>/cwd`; on
/// macOS, `lsof`'s cwd list. Not being able to look is an `Err`: the tree
/// stays.
pub fn cwd_in(tree: &str) -> Result<bool, String> {
    holders(tree).map(|h| !h.is_empty())
}

/// The processes that have their current folder in `tree`, by name and pid
/// (card #1118): the same look as [`cwd_in`], which only says there is one.
/// A native Windows process isn't listed (see the module doc); `git
/// worktree move` still refuses for it.
pub fn holders(tree: &str) -> Result<Vec<Holder>, String> {
    Ok(holders_of(&[tree.to_string()])?
        .remove(&key(tree))
        .unwrap_or_default())
}

/// [`holders`] for several trees with one look at the processes (Storage
/// lists every tree): each tree, as a [`key`], and its holders. A tree
/// nobody holds has no entry.
pub fn holders_of(trees: &[String]) -> Result<BTreeMap<String, Vec<Holder>>, String> {
    let paths: Vec<(String, String)> = trees
        .iter()
        .map(|t| {
            let real = fs::canonicalize(t)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| t.clone());
            (t.clone(), real)
        })
        .collect();
    let mut out: BTreeMap<String, Vec<Holder>> = BTreeMap::new();
    for (h, cwd) in process_cwds(trees)? {
        for (t, real) in &paths {
            if under(&cwd, t) || under(&cwd, real) {
                let list = out.entry(key(t)).or_default();
                if !list.contains(&h) {
                    list.push(h.clone());
                }
            }
        }
    }
    for list in out.values_mut() {
        list.sort_by_key(|h| h.pid);
    }
    Ok(out)
}

/// Ends holder `pid` of `tree` (card #1118, Storage's End it). It is looked
/// up again first, so a pid that has gone since, or been given to a process
/// outside the tree, is refused; Irori's own process is never ended.
/// Returns who was ended.
pub fn end_holder(tree: &str, pid: u32) -> Result<Holder, String> {
    if pid == std::process::id() {
        return Err("that is Irori itself".into());
    }
    // The process list can miss a process for a moment on a busy machine
    // (Git's `sh` listing `/proc`): look a few times before saying it went.
    let mut found = None;
    for n in 0..4 {
        if n > 0 {
            std::thread::sleep(Duration::from_millis(400));
        }
        found = holders(tree)?.into_iter().find(|h| h.pid == pid);
        if found.is_some() {
            break;
        }
    }
    let h = found.ok_or_else(|| format!("pid {pid} isn't in {tree} any more"))?;
    kill_pid(pid)?;
    Ok(h)
}

#[cfg(windows)]
fn kill_pid(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // /T: its children too (a dev server under a runner).
    let out = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("taskkill: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "taskkill: {} {}",
            String::from_utf8_lossy(&out.stdout).trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) -> Result<(), String> {
    let out = Command::new("kill")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("kill: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "kill: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Every process here that can be read and its current folder. `tree` lets
/// Windows ask for names of only the ones in it.
fn process_cwds(trees: &[String]) -> Result<Vec<(Holder, String)>, String> {
    #[cfg(windows)]
    {
        msys_holders(trees)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = trees;
        let rd = fs::read_dir("/proc").map_err(|e| format!("can't list processes: {e}"))?;
        let mut out = Vec::new();
        for e in rd.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            // Another user's process can't be read; it can't be in our
            // tree's folder without our say either.
            if let Ok(p) = fs::read_link(e.path().join("cwd")) {
                let name = fs::read_to_string(e.path().join("comm"))
                    .map(|n| n.trim().to_string())
                    .unwrap_or_default();
                out.push((Holder { pid, name }, p.to_string_lossy().into_owned()));
            }
        }
        Ok(out)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let _ = trees;
        let out = Command::new("lsof")
            .args(["-n", "-P", "-w", "-a", "-d", "cwd", "-F", "pcn"])
            .output()
            .map_err(|e| format!("can't list processes (lsof: {e})"))?;
        // lsof exits 1 when some process couldn't be read; the list is
        // still whole for this user's own.
        let text = String::from_utf8_lossy(&out.stdout);
        if text.trim().is_empty() {
            return Err("can't list processes (lsof said nothing)".into());
        }
        Ok(lsof_procs(&text))
    }
}

/// The processes and folders in `lsof -F pcn` output: a `p<pid>` line
/// starts a process, `c<command>` names it and `n<folder>` is its folder.
pub fn lsof_procs(text: &str) -> Vec<(Holder, String)> {
    let (mut pid, mut name) = (0u32, String::new());
    let mut out = Vec::new();
    for l in text.lines() {
        if let Some(p) = l.strip_prefix('p') {
            pid = p.parse().unwrap_or(0);
            name.clear();
        } else if let Some(c) = l.strip_prefix('c') {
            name = c.to_string();
        } else if let Some(n) = l.strip_prefix('n')
            && pid != 0
        {
            let holder = Holder {
                pid,
                name: name.clone(),
            };
            out.push((holder, n.to_string()));
        }
    }
    out
}

/// The current folders of the MSYS processes Git for Windows' own runtime
/// knows (Git Bash, its `sleep`, `less`, the Claude Code Bash tool's
/// shell). They open their folder so it can still be renamed, so
/// [`remove`]'s move can't see them; Git's `sh` can, through `/proc`.
/// None when there is no Git `sh` here. Another MSYS2 or Cygwin install,
/// or a process in another sandbox, is a world of its own and isn't seen.
#[cfg(windows)]
fn msys_holders(trees: &[String]) -> Result<Vec<(Holder, String)>, String> {
    let Some(sh) = git_sh() else {
        return Ok(Vec::new());
    };
    // One find and one cygpath, not a fork per process: a loop took 30 s on
    // a busy machine. Full paths: a sh started from outside has no
    // /usr/bin on its PATH. Each line is `/proc/<pid>`, a tab and the
    // folder; the folders are converted apart (cygpath would mangle the
    // pid column) and pasted back beside their pids.
    let text = sh_output(
        &sh,
        r#"L=$(/usr/bin/find /proc -mindepth 2 -maxdepth 2 -name cwd -printf '%h\t%l\n' 2>/dev/null); T=$(/usr/bin/mktemp); printf '%s\n' "$L" | /usr/bin/cut -f2 | /usr/bin/cygpath -m -f - > "$T"; printf '%s\n' "$L" | /usr/bin/cut -f1 | /usr/bin/paste - "$T"; /usr/bin/rm -f "$T"; echo end"#,
    )?;
    let mut lines: Vec<&str> = text.lines().map(str::trim).collect();
    if lines.pop() != Some("end") {
        return Err("can't list Git Bash processes".into());
    }
    let procs: Vec<(&str, &str)> = lines
        .into_iter()
        .filter_map(|l| {
            let (p, cwd) = l.split_once('\t')?;
            let pid = p.strip_prefix("/proc/")?;
            (!cwd.is_empty() && pid.bytes().all(|b| b.is_ascii_digit())).then_some((pid, cwd))
        })
        .collect();
    let reals: Vec<String> = trees
        .iter()
        .flat_map(|t| {
            let real = fs::canonicalize(t).map(|p| p.to_string_lossy().into_owned());
            [Some(t.clone()), real.ok()].into_iter().flatten()
        })
        .collect();
    let inside = |c: &str| reals.iter().any(|t| under(c, t));
    // The ones outside the tree need no name: nothing reads it.
    let mut out: Vec<(Holder, String)> = procs
        .iter()
        .filter(|(_, c)| !inside(c))
        .map(|(p, c)| {
            let holder = Holder {
                pid: p.parse().unwrap_or(0),
                name: String::new(),
            };
            (holder, c.to_string())
        })
        .collect();
    let here: Vec<&(&str, &str)> = procs.iter().filter(|(_, c)| inside(c)).collect();
    if here.is_empty() {
        return Ok(out);
    }
    let ids: Vec<&str> = here.iter().map(|(p, _)| *p).collect();
    let names = sh_output(
        &sh,
        &format!(
            r#"for p in {}; do printf '%s\t%s\t%s\n' "$p" "$(/usr/bin/cat /proc/$p/winpid 2>/dev/null)" "$(/usr/bin/basename "$(/usr/bin/readlink /proc/$p/exe 2>/dev/null)" 2>/dev/null)"; done"#,
            ids.join(" ")
        ),
    )?;
    for l in names.lines() {
        let f: Vec<&str> = l.trim_end().split('\t').collect();
        let [msys, win, name] = f[..] else { continue };
        // A process that has gone since has no Windows pid: not a holder.
        let Ok(pid) = win.trim().parse::<u32>() else {
            continue;
        };
        if let Some((_, cwd)) = here.iter().find(|(p, _)| *p == msys) {
            let name = if name.is_empty() {
                "a Git Bash process".to_string()
            } else {
                name.to_string()
            };
            out.push((Holder { pid, name }, cwd.to_string()));
        }
    }
    Ok(out)
}

/// Git for Windows' own `sh.exe`, when it is here.
#[cfg(windows)]
fn git_sh() -> Option<PathBuf> {
    let exec = git(Path::new("."), &["--exec-path"]).ok()?;
    // <git>/mingw64/libexec/git-core -> <git>/usr/bin/sh.exe
    let root = Path::new(&exec).ancestors().nth(3)?;
    let sh = root.join("usr").join("bin").join("sh.exe");
    sh.is_file().then_some(sh)
}

#[cfg(windows)]
fn sh_output(sh: &Path, script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = Command::new(sh)
        .args(["-c", script])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("can't list Git Bash processes ({}: {e})", sh.display()))?;
    if out.stdout.is_empty() {
        return Err(format!(
            "can't list Git Bash processes ({})",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The folders in `lsof -F n` output.
pub fn lsof_cwds(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix('n'))
        .map(str::to_string)
        .collect()
}

/// Whether any of `cwds` is `tree` or in it, real paths compared too.
pub fn any_under(cwds: &[String], tree: &str) -> bool {
    let real = fs::canonicalize(tree)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| tree.to_string());
    cwds.iter().any(|c| under(c, tree) || under(c, &real))
}

/// Makes tree `tree` of `main` again on `branch` (finding 6: a reopened
/// card whose tree went when it settled): one stranded at its
/// [`aside_path`] is moved back, else it is made on the local branch, or
/// one made from `origin/<branch>`. git refuses a branch checked out
/// elsewhere, and so does this.
pub fn reopen(main: &Path, tree: &Path, branch: &str) -> Result<(), String> {
    add(main, tree, branch, false)
}

/// Makes the tree a card's agent starts in (card #565 finding A): Irori
/// makes it, never `claude -w`, so `claude rm` never owns it. Like
/// [`reopen`], and a branch that is nowhere yet is made from `main`'s
/// HEAD. A tree of ours already there for it is used as it is.
pub fn make(main: &Path, tree: &Path, branch: &str) -> Result<(), String> {
    add(main, tree, branch, true)
}

fn add(main: &Path, tree: &Path, branch: &str, fresh: bool) -> Result<(), String> {
    if !shaped(tree) {
        return Err(format!("{} isn't a tree of ours", tree.display()));
    }
    if branch.trim().is_empty() || branch.starts_with('-') {
        return Err(format!("{branch:?} can't name its branch"));
    }
    if begun_path(tree).exists() && !aside_path(tree).exists() && !tree.exists() {
        let _ = fs::remove_file(begun_path(tree));
    }
    if begun_path(tree).exists() {
        return Err(format!("a removal of {} is under way", tree.display()));
    }
    restore(tree)?;
    if tree.exists() {
        let same = ours(tree).is_some_and(|m| {
            let real = |p: &Path| fs::canonicalize(p).map(|p| key(&p.to_string_lossy()));
            matches!((real(&m), real(main)), (Ok(a), Ok(b)) if a == b)
        });
        return if same && registered(tree) {
            Ok(())
        } else {
            Err(format!("{} is in the way", tree.display()))
        };
    }
    let has = |r: &str| git(main, &["rev-parse", "--verify", "--quiet", r]).is_ok();
    let p = tree.to_string_lossy().into_owned();
    if let Some(dir) = tree.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if has(&format!("refs/heads/{branch}")) {
        git(main, &["worktree", "add", &p, branch])?;
    } else if has(&format!("refs/remotes/origin/{branch}")) {
        let base = format!("origin/{branch}");
        git(
            main,
            &["worktree", "add", "--track", "-b", branch, &p, &base],
        )?;
    } else if fresh {
        git(
            main,
            &["worktree", "add", "--no-track", "-b", branch, &p, "HEAD"],
        )?;
    } else {
        return Err(format!("its branch {branch} isn't here or on origin"));
    }
    Ok(())
}

/// Whether `text` names tree `tree` (a path or its leaf), as a whole name:
/// `card-5-1` isn't named by a line about `card-5-12` (finding 8).
pub fn names_tree(text: &str, tree: &str) -> bool {
    let leaf = key(tree);
    let leaf = leaf.rsplit('/').next().unwrap_or(&leaf);
    if leaf.is_empty() {
        return false;
    }
    let t = key(text);
    let word = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_');
    t.match_indices(leaf).any(|(at, _)| {
        let starts = t[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !word(c) && c != '.');
        let mut rest = t[at + leaf.len()..].chars();
        // A full stop ends a sentence; `.x` goes on with the name.
        let ends = match rest.next() {
            None => true,
            Some('.') => !rest.next().is_some_and(word),
            Some(c) => !word(c),
        };
        starts && ends
    })
}

/// What this device remembers about its trees (card #565), in the data
/// folder: they only mean anything here, so they aren't in the log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    /// Each tree, as a [`key`], and the cards that worked in it.
    pub trees: BTreeMap<String, BTreeSet<String>>,
    /// Spawned agents whose job `claude rm` kept: it says the same each
    /// time it is asked, so it is asked once.
    pub kept_jobs: BTreeSet<String>,
}

impl Book {
    /// The book in `dir`; empty when there is none or it can't be read.
    pub fn load(dir: &Path) -> Self {
        fs::read(dir.join(BOOK))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Writes the book to `dir`, whole or not at all.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| format!("{BOOK}: {e}"))?;
        let tmp = dir.join(format!("{BOOK}.tmp"));
        fs::write(&tmp, text).map_err(|e| format!("{BOOK}: {e}"))?;
        fs::rename(&tmp, dir.join(BOOK)).map_err(|e| format!("{BOOK}: {e}"))
    }

    /// Writes down that card `id` worked in `tree`. True if that is new.
    pub fn note(&mut self, tree: &str, id: &str) -> bool {
        self.trees
            .entry(key(tree))
            .or_default()
            .insert(id.to_string())
    }
}
