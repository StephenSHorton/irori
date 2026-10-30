//! I:internal/capture/gitlink/describe.go.

use std::time::Duration;

use sys::cancel::Cancel;

use super::{Commit, FileChange, Git, GitError, MAX_FILES, is_sha};
use crate::gotime;

/// Commit.subject's cap, in runes.
const MAX_SUBJECT: usize = 200;

/// Reads a commit: `git show -s --format=%H%x00%P%x00%ct%x00%an%x00%s` and
/// `git diff-tree -r --no-commit-id --no-renames --raw --no-abbrev
/// --diff-merges=first-parent -z [--root]`.
pub fn describe(
    g: &dyn Git,
    cancel: &Cancel,
    timeout: Duration,
    root: &str,
    sha: &str,
) -> Result<Commit, GitError> {
    if !is_sha(sha) {
        return Err(GitError::Other(format!(
            "gitlink: {} is not a full object id",
            gocompat::quote::quote(sha)
        )));
    }
    let out = g.run(
        cancel,
        timeout,
        root,
        &[
            "-c",
            "log.showSignature=false",
            "show",
            "-s",
            "--no-color",
            "--format=%H%x00%P%x00%ct%x00%an%x00%s",
            sha,
            "--",
        ],
    )?;
    let trimmed = out.trim_end_matches(['\r', '\n']);
    let parts: Vec<&str> = trimmed.splitn(5, '\0').collect();
    if parts.len() != 5 || !is_sha(parts[0]) {
        return Err(GitError::Other(format!(
            "gitlink: unexpected git show output for {sha}"
        )));
    }
    let ct: i64 = parts[2].parse().map_err(|_| {
        GitError::Other(format!(
            "gitlink: bad committer time {} for {sha}",
            gocompat::quote::quote(parts[2])
        ))
    })?;
    let mut c = Commit {
        sha: parts[0].to_ascii_lowercase(),
        parents: gocompat::text::to_lower_str(parts[1])
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        at: gotime::unix_local(ct),
        author: parts[3].to_string(),
        subject: trunc_runes(parts[4], MAX_SUBJECT),
        files: None,
        files_omitted: 0,
    };
    let mut args = vec![
        "diff-tree",
        "-r",
        "--no-commit-id",
        "--no-renames",
        "--raw",
        "--no-abbrev",
        "--diff-merges=first-parent",
        "-z",
    ];
    if c.parents.is_empty() {
        args.push("--root");
    }
    let sha_l = c.sha.clone();
    args.push(&sha_l);
    args.push("--");
    let out = g.run(cancel, timeout, root, &args)?;
    let mut files =
        parse_raw_z(&out).map_err(|e| GitError::Other(format!("gitlink: diff-tree {sha}: {e}")))?;
    if files.len() > MAX_FILES {
        c.files_omitted = (files.len() - MAX_FILES) as i64;
        files.truncate(MAX_FILES);
    }
    c.files = Some(files);
    Ok(c)
}

/// Reads `diff-tree --raw -z` output: for each path a
/// `:<mode> <mode> <old> <new> <status>` field, then the path.
pub fn parse_raw_z(out: &str) -> Result<Vec<FileChange>, String> {
    let mut files = Vec::new();
    let fields: Vec<&str> = out.split('\0').collect();
    let mut i = 0;
    while i < fields.len() {
        let meta = fields[i].trim_start_matches(['\r', '\n']);
        if meta.is_empty() {
            i += 1;
            continue;
        }
        if !meta.starts_with(':') {
            return Err(format!("unexpected field {}", gocompat::quote::quote(meta)));
        }
        let m: Vec<&str> = meta[1..].split_whitespace().collect();
        if m.len() != 5 || m[4].is_empty() {
            return Err(format!("unexpected field {}", gocompat::quote::quote(meta)));
        }
        if i + 1 >= fields.len() {
            return Err(format!(
                "entry {} has no path",
                gocompat::quote::quote(meta)
            ));
        }
        i += 1;
        files.push(FileChange {
            path: fields[i].to_string(),
            blob: gocompat::text::to_lower_str(m[3]),
            // Go m[4][:1]: the first byte.
            status: String::from_utf8_lossy(&m[4].as_bytes()[..1]).into_owned(),
        });
        i += 1;
    }
    Ok(files)
}

/// At most `n` runes, no ellipsis.
fn trunc_runes(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
