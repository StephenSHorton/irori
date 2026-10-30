//! I:internal/capture/cmddir.go: the directory a commit command really ran
//! git in, and the committer view of a commit-command row.

use crate::gitlink::{COMMITTER_WINDOW, CommitCmd};
use crate::gopath;
use crate::gotime::{add, after, before, sub};
use crate::rows::CommitCmdRow;

const COMMIT_SUBCOMMANDS: &[&str] = &[
    "commit",
    "merge",
    "cherry-pick",
    "revert",
    "rebase",
    "am",
    "pull",
];

/// Walks the command's segments (split on `&& || ; |` and newlines outside
/// quotes), following cd / Set-Location / pushd, and for the first git
/// segment whose subcommand can commit applies that git's own `-C`s. When
/// nothing commits it returns where the segments ended (cwd without a cd).
/// `home` expands `~`, `$HOME`, `%USERPROFILE%` and friends ("" leaves
/// them). The result is not cleaned (capture.md §7 trap 17).
pub fn command_dir(command: &str, cwd: &str, home: &str) -> String {
    let mut cur = cwd.to_string();
    for seg in split_segments(command) {
        let w = shell_words(&seg);
        if w.is_empty() {
            continue;
        }
        let prog = gocompat::text::to_lower_str(&prog_base(&w[0]));
        match prog.as_str() {
            "cd" | "set-location" | "sl" | "pushd" | "push-location" | "chdir" => {
                let mut args = &w[1..];
                if !args.is_empty()
                    && (gocompat::text::equal_fold(&args[0], "-path")
                        || gocompat::text::equal_fold(&args[0], "-literalpath"))
                {
                    args = &args[1..];
                }
                if !args.is_empty() && args[0] != "-" {
                    cur = resolve_dir(&args[0], &cur, home);
                }
            }
            "git" => {
                let mut dir = cur.clone();
                let mut i = 1;
                while i < w.len() && w[i].starts_with('-') {
                    if w[i] == "-C" && i + 1 < w.len() {
                        dir = resolve_dir(&w[i + 1], &dir, home);
                        i += 2;
                    } else if (w[i] == "-c"
                        || w[i] == "--git-dir"
                        || w[i] == "--work-tree"
                        || w[i] == "--namespace")
                        && i + 1 < w.len()
                    {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < w.len() && COMMIT_SUBCOMMANDS.contains(&w[i].as_str()) {
                    return dir;
                }
            }
            _ => {}
        }
    }
    cur
}

/// A program word without its directory and `.exe`.
fn prog_base(w: &str) -> String {
    let b = gopath::base(&gopath::from_slash(&w.replace('\\', "/")));
    let b = b.strip_suffix(".exe").unwrap_or(&b);
    b.strip_suffix(".EXE").unwrap_or(b).to_string()
}

/// Expands a home prefix and joins a relative result to `cur`.
fn resolve_dir(p: &str, cur: &str, home: &str) -> String {
    let mut p = p.to_string();
    if !home.is_empty() {
        for pre in [
            "~",
            "$HOME",
            "${HOME}",
            "$env:USERPROFILE",
            "$env:HOME",
            "%USERPROFILE%",
        ] {
            if p == pre || p.starts_with(&format!("{pre}/")) || p.starts_with(&format!("{pre}\\")) {
                p = format!("{home}{}", &p[pre.len()..]);
                break;
            }
        }
    }
    let p = git_bash_path(&p);
    if is_abs_any_os(&p) || cur.is_empty() {
        return p;
    }
    gopath::join(&[cur, &gopath::from_slash(&p)])
}

/// Git Bash's `/c/Users/...` to `C:/Users/...`.
fn git_bash_path(p: &str) -> String {
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[2] == b'/' && b[1].is_ascii_alphabetic() {
        return format!("{}:{}", p[1..2].to_ascii_uppercase(), &p[2..]);
    }
    if b.len() == 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() {
        return format!("{}:/", p[1..2].to_ascii_uppercase());
    }
    p.to_string()
}

/// Splits on `&&`, `||`, `;`, `|` and newlines outside quotes (a single
/// `&` is not a separator).
fn split_segments(s: &str) -> Vec<String> {
    let rs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut b = String::new();
    let mut q: Option<char> = None;
    let mut i = 0;
    while i < rs.len() {
        let r = rs[i];
        if let Some(qc) = q {
            if r == qc {
                q = None;
            }
            b.push(r);
        } else if r == '\'' || r == '"' {
            q = Some(r);
            b.push(r);
        } else if r == ';'
            || r == '\n'
            || r == '|'
            || (r == '&' && i + 1 < rs.len() && rs[i + 1] == '&')
        {
            if (r == '&' || r == '|') && i + 1 < rs.len() && rs[i + 1] == r {
                i += 1;
            }
            out.push(std::mem::take(&mut b));
        } else {
            b.push(r);
        }
        i += 1;
    }
    out.push(b);
    out
}

/// One segment's words: quotes group (and are removed), a backslash before
/// a space escapes it (Git Bash), other backslashes are kept.
fn shell_words(s: &str) -> Vec<String> {
    let rs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut b = String::new();
    let (mut in_word, mut q) = (false, None::<char>);
    let mut i = 0;
    while i < rs.len() {
        let r = rs[i];
        if let Some(qc) = q {
            if r == qc {
                q = None;
            } else {
                b.push(r);
            }
        } else if r == '\'' || r == '"' {
            q = Some(r);
            in_word = true;
        } else if r == '\\' && i + 1 < rs.len() && rs[i + 1] == ' ' {
            b.push(' ');
            in_word = true;
            i += 1;
        } else if r == ' ' || r == '\t' || r == '\r' {
            if in_word {
                out.push(std::mem::take(&mut b));
                in_word = false;
            }
        } else {
            b.push(r);
            in_word = true;
        }
        i += 1;
    }
    if in_word {
        out.push(b);
    }
    out
}

/// Windows drive paths (either slash) and POSIX paths.
fn is_abs_any_os(p: &str) -> bool {
    if gopath::is_abs(p) || p.starts_with('/') || p.starts_with("\\\\") {
        return true;
    }
    let b = p.as_bytes();
    b.len() >= 3 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

impl CommitCmdRow {
    /// Whether a commit at `t` could be this command's: inside its
    /// [StartedAt, At] span, give or take twice the committer window.
    pub fn spans(&self, t: &gocompat::time::GoTime) -> bool {
        let start = if self.started_at.is_zero() || after(&self.started_at, &self.at) {
            self.at
        } else {
            self.started_at
        };
        let w = 2 * COMMITTER_WINDOW;
        !before(t, &sub(&start, w)) && !after(t, &add(&self.at, w))
    }

    /// The row as the committer match sees it; rows logged before `dir`
    /// existed parse their first line.
    pub fn commit_ref(&self, home: &str) -> CommitCmd {
        let dir = if self.dir.is_empty() {
            command_dir(&self.command, &self.cwd, home)
        } else {
            self.dir.clone()
        };
        CommitCmd {
            key: self.key.clone(),
            turn_id: self.turn_id.clone(),
            cwd: dir,
            at: self.at,
            start: self.started_at,
        }
    }
}
