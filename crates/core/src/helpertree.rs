//! Helper worktrees for a card with several agents (docs/design/
//! spawned-agents.md §3, part 7). Irori makes each helper's worktree
//! itself, branched from the card's branch, so a helper starts from the
//! lead's plan and scaffold commits; `claude --bg -w` can only branch from
//! the default branch or HEAD, so it stays for single-agent cards. The
//! helper is spawned with its folder set to the worktree made here.
//!
//! The worktree sits where `-w` puts its own (`.claude/worktrees/<name>`
//! in the repo), so its trust follows the repo's entry, and the helper's
//! card reports it in Where like any agent's, so a settled card's build
//! folder goes the usual way (card #427).
//!
//! Making it again is safe: an existing worktree for the part is reused,
//! and a part whose worktree is gone but whose branch is left (a helper
//! that ended before the lead merged) gets a worktree on that branch, so
//! the fresh helper starts from the old one's commits.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where Claude Code's `-w` keeps worktrees, under the repo root.
pub const WORKTREES: &str = ".claude/worktrees";

/// A helper's worktree and its branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperTree {
    pub path: PathBuf,
    pub branch: String,
    /// False when an existing worktree was reused.
    pub made: bool,
}

/// `s` with anything outside `[A-Za-z0-9._-]` as `-`, runs collapsed and
/// the ends trimmed, so a step id (`s3`) or a short ref (`#12`, `wind#4`)
/// makes a safe branch and folder name.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            c
        } else {
            '-'
        };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out.trim_matches(|c| c == '-' || c == '.').to_string()
}

/// The helper branch for `part` of the card on `card_branch`:
/// `<card branch>-<part>`, as §3 names it.
pub fn helper_branch(card_branch: &str, part: &str) -> String {
    format!("{card_branch}-{}", slug(part))
}

/// The helper's worktree folder in `repo`: the branch's last path
/// segment, slugged, under [`WORKTREES`].
pub fn helper_path(repo: &Path, branch: &str) -> PathBuf {
    let leaf = branch.rsplit('/').next().unwrap_or(branch);
    repo.join(WORKTREES).join(slug(leaf))
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    // A path from another device (a Mac's /Users/... on the PC) is no
    // checkout here: refuse it before spawning anything.
    if !repo.is_dir() {
        return Err(format!(
            "git {}: no folder {} on this device",
            args.join(" "),
            repo.display()
        ));
    }
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: Irori is a GUI app, so without it every call
        // flashes a console window up on the person's screen.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The main checkout of the repo `dir` is in (a linked worktree's too).
pub fn main_root(dir: &Path) -> Option<PathBuf> {
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let common = PathBuf::from(common);
    if common.file_name().is_some_and(|n| n == ".git") {
        return common.parent().map(Path::to_path_buf);
    }
    None
}

fn has_ref(repo: &Path, r: &str) -> bool {
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{r}^{{commit}}"),
        ],
    )
    .is_ok()
}

/// The branch checked out in each of `repo`'s worktrees, by path.
fn worktrees(repo: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let list = git(repo, &["worktree", "list", "--porcelain"])?;
    let mut out = Vec::new();
    let mut path: Option<PathBuf> = None;
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/")
            && let Some(p) = path.take()
        {
            out.push((p, b.to_string()));
        }
    }
    Ok(out)
}

/// Makes (or finds) the worktree for `part` of the card whose work is on
/// `card_branch`, in the git repo at `repo`. The card branch is taken
/// locally, else from `origin/` (the lead pushes at each step).
pub fn make(repo: &Path, card_branch: &str, part: &str) -> Result<HelperTree, String> {
    if slug(part).is_empty() {
        return Err(format!(
            "no part to name the helper's branch after: {part:?}"
        ));
    }
    if card_branch.trim().is_empty() {
        return Err("the card has no branch to start helpers from".into());
    }
    let branch = helper_branch(card_branch, part);
    let path = helper_path(repo, &branch);
    if let Some((p, _)) = worktrees(repo)?.into_iter().find(|(_, b)| *b == branch) {
        return Ok(HelperTree {
            path: p,
            branch,
            made: false,
        });
    }
    if path.exists() {
        return Err(format!(
            "{} is in the way: it isn't the worktree of {branch}",
            path.display()
        ));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let p = path.to_string_lossy().to_string();
    if has_ref(repo, &format!("refs/heads/{branch}")) {
        // The part's last helper ended before its merge: carry on from it.
        git(repo, &["worktree", "add", &p, &branch])?;
    } else {
        let base = if has_ref(repo, &format!("refs/heads/{card_branch}")) {
            card_branch.to_string()
        } else if has_ref(repo, &format!("refs/remotes/origin/{card_branch}")) {
            format!("origin/{card_branch}")
        } else {
            return Err(format!(
                "the card's branch {card_branch} isn't here or on origin"
            ));
        };
        git(
            repo,
            &["worktree", "add", "--no-track", "-b", &branch, &p, &base],
        )?;
    }
    Ok(HelperTree {
        path,
        branch,
        made: true,
    })
}
