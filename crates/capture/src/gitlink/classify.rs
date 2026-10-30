//! I:internal/capture/gitlink/classify.go: committed files to captured
//! edits, and which command made a commit.

use gocompat::text::{equal_fold, to_lower_str};

use super::{
    COMMITTER_WINDOW, Commit, CommitCmd, EditRef, FileChange, FileLink, LINK_EXACT, LINK_HEURISTIC,
    LINK_PARTIAL, LINK_UNATTRIBUTED, zero_sha,
};
use crate::gotime::{add, after, before, sub, sub_nanos};

/// Links each of `c`'s files (deletions included: a deletion links edits
/// only by root, never by blob) to candidate edits. Per file:
/// - exact: some non-heuristic edit (any root) whose AfterBlob or
///   AfterBlobLF is the file's blob: the newest such edit plus the earlier
///   edits with the same Root and Path, heuristic ones included;
/// - partial: else the edits to the path whose Root is `root`;
/// - heuristic: as partial when all of those are heuristic;
/// - unattributed: none.
pub fn classify(c: &Commit, root: &str, cands: &[EditRef]) -> Vec<FileLink> {
    c.files
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|f| classify_file(f, root, cands))
        .collect()
}

fn classify_file(f: &FileChange, root: &str, cands: &[EditRef]) -> FileLink {
    let mut link = FileLink {
        path: f.path.clone(),
        link: LINK_UNATTRIBUTED.into(),
        edits: Vec::new(),
    };
    let deleted = f.status == "D" || f.blob.is_empty() || zero_sha(&f.blob);
    if !deleted {
        let mut best: Option<usize> = None;
        for (i, e) in cands.iter().enumerate() {
            if e.path != f.path || e.heuristic || !blob_match(e, &f.blob) {
                continue;
            }
            if best.is_none_or(|b| !before(&e.at, &cands[b].at)) {
                best = Some(i);
            }
        }
        if let Some(bi) = best {
            let b = &cands[bi];
            let run: Vec<usize> = cands
                .iter()
                .enumerate()
                .filter(|(i, e)| {
                    *i == bi
                        || (e.path == b.path && same_root(&e.root, &b.root) && !after(&e.at, &b.at))
                })
                .map(|(i, _)| i)
                .collect();
            link.link = LINK_EXACT.into();
            link.edits = edit_ids(cands, run);
            return link;
        }
    }
    // An empty root matches no edit's root, not even an empty one.
    let mut own = Vec::new();
    let mut all_heuristic = true;
    for (i, e) in cands.iter().enumerate() {
        if !root.is_empty() && e.path == f.path && same_root(&e.root, root) {
            own.push(i);
            all_heuristic = all_heuristic && e.heuristic;
        }
    }
    if own.is_empty() {
        return link;
    }
    link.link = if all_heuristic {
        LINK_HEURISTIC
    } else {
        LINK_PARTIAL
    }
    .into();
    link.edits = edit_ids(cands, own);
    link
}

fn blob_match(e: &EditRef, blob: &str) -> bool {
    (!e.after_blob.is_empty() && equal_fold(&e.after_blob, blob))
        || (!e.after_blob_lf.is_empty() && equal_fold(&e.after_blob_lf, blob))
}

/// The ids of `cands[idx]`, oldest first; equal times keep candidate order.
fn edit_ids(cands: &[EditRef], mut idx: Vec<usize>) -> Vec<String> {
    idx.sort_by(|&a, &b| crate::gotime::cmp(&cands[a].at, &cands[b].at));
    idx.into_iter().map(|i| cands[i].id.clone()).collect()
}

/// The command that made `c`: Cwd under `root`, its [Start, At] span within
/// [`COMMITTER_WINDOW`] of the commit time, the closest first (inside the
/// span is distance 0), then the latest. `None` when none.
pub fn committer(c: &Commit, root: &str, cmds: &[CommitCmd]) -> Option<CommitCmd> {
    if root.is_empty() {
        return None;
    }
    let mut best: Option<(usize, i128)> = None;
    for (i, cmd) in cmds.iter().enumerate() {
        if !under_root(&cmd.cwd, root) {
            continue;
        }
        let start = if cmd.start.is_zero() || after(&cmd.start, &cmd.at) {
            cmd.at
        } else {
            cmd.start
        };
        if before(&cmd.at, &sub(&c.at, COMMITTER_WINDOW))
            || after(&start, &add(&c.at, COMMITTER_WINDOW))
        {
            continue;
        }
        let d = if before(&c.at, &start) {
            sub_nanos(&start, &c.at)
        } else if after(&c.at, &cmd.at) {
            sub_nanos(&c.at, &cmd.at)
        } else {
            0
        };
        let take = match best {
            None => true,
            Some((b, bd)) => d < bd || (d == bd && after(&cmd.at, &cmds[b].at)),
        };
        if take {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| cmds[i].clone())
}

/// Whether `dir` is `root` or inside it.
pub fn under_root(dir: &str, root: &str) -> bool {
    let (d, r) = (norm_path(dir, root), norm_path(root, dir));
    if d.is_empty() || r.is_empty() {
        return false;
    }
    if d == r {
        return true;
    }
    if r.ends_with('/') {
        // A drive root "c:/" or "/".
        return d.starts_with(&r);
    }
    d.starts_with(&format!("{r}/"))
}

/// Two worktree roots, compared the way [`under_root`] does.
pub(crate) fn same_root(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (na, nb) = (norm_path(a, b), norm_path(b, a));
    !na.is_empty() && na == nb
}

/// A comparable path: forward slashes, no trailing slash (but a bare drive
/// root "c:/" or "/"), and when `p` or `reference` is Windows-style,
/// lowercase with a Git Bash "/c/..." turned into "c:/...".
fn norm_path(p: &str, reference: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    let win = windows_style(p) || windows_style(reference);
    let mut p = p.replace('\\', "/");
    if win {
        p = to_lower_str(&p);
        let b = p.as_bytes();
        if b.len() >= 2
            && b[0] == b'/'
            && b[1].is_ascii_alphabetic()
            && (b.len() == 2 || b[2] == b'/')
        {
            p = format!("{}:{}", &p[1..2], &p[2..]);
        }
    }
    while p.len() > 1 && p.ends_with('/') && !(p.len() == 3 && p.as_bytes()[1] == b':') {
        p.pop();
    }
    if p.len() == 2 && p.as_bytes()[1] == b':' {
        p.push('/');
    }
    p
}

/// A drive-letter path ("C:\x", "c:/x") or one with backslashes.
fn windows_style(p: &str) -> bool {
    if p.contains('\\') {
        return true;
    }
    let b = p.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}
