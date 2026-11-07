//! Recording Claude Code's workspace trust for one repo (card #449,
//! docs/design/spawned-agents.md §7 "The pre-trust flow", §14 part 5).
//!
//! `claude --bg` refuses a folder whose trust dialog was never accepted,
//! and a git repo needs its own entry even under a trusted parent. Trust
//! lives in `.claude.json` as `projects[<dir>].hasTrustDialogAccepted`.
//! Running Claude Code sessions rewrite that file often, so a plain write
//! could lose their changes (or they ours). [`record_trust`] edits one key
//! with an atomic read-modify-write: read, change only that key, write a
//! temp file beside it, check the file didn't change underneath, rename,
//! then re-read to verify, retrying from a fresh read when either check
//! fails.
//!
//! Only the window's Trust and start calls this, after the person
//! confirms: no tool, row or agent writes trust (§7 point 4). The crates
//! agents reach (tools, core) can't call it: they don't depend on this
//! one.
//!
//! Which folder to pass: the main repo's root, not a worktree's path.
//! On Claude Code 2.1.318 a git repo needs its own entry (a trusted parent
//! doesn't count), a git worktree follows its main repo's entry, and a
//! folder outside git inherits from its parents (card #445's live checks).
//! Checked again on 2.1.319 (card #731): under a trusted home folder, a new
//! git repo is refused by `claude --bg` and interactive `claude` shows the
//! trust prompt there, while a plain folder starts.
//! Part 1's `trust_root(dir)` finds that root.
//!
//! The edit keeps every other byte: only the levels on the way to the key
//! are parsed (as ordered maps of raw JSON), and everything else is
//! written back exactly as it was read.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

/// The key Claude Code sets when its trust dialog is accepted.
pub const TRUST_KEY: &str = "hasTrustDialogAccepted";

/// How many fresh read-modify-write attempts before giving up.
const ATTEMPTS: usize = 8;

/// `.claude.json`: `$CLAUDE_CONFIG_DIR/.claude.json`, else in the home
/// folder (as the roster's trust reader).
pub fn config_path() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(d) => Some(PathBuf::from(d).join(".claude.json")),
        None => std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|v| !v.is_empty())
            .map(|h| PathBuf::from(h).join(".claude.json")),
    }
}

/// The `projects` key Claude Code uses for `dir`: forward slashes, no
/// trailing slash (`C:/work/api` on Windows, `/Users/me/api` elsewhere).
pub fn project_key(dir: &str) -> String {
    let s = dir.replace('\\', "/");
    let t = s.trim_end_matches('/');
    if t.is_empty() { s } else { t.to_string() }
}

/// Whether `dir` itself (not a parent) has its trust dialog accepted in
/// `file`. Unreadable or missing reads as not trusted.
pub fn is_trusted(file: &Path, dir: &str) -> bool {
    std::fs::read(file)
        .ok()
        .is_some_and(|b| trusted_in(&b, &project_key(dir)))
}

/// What [`record_trust`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// It was trusted already; the file wasn't touched.
    Already,
    /// Written and verified, after this many attempts (1 when nothing
    /// raced it).
    Written { attempts: usize },
}

/// Why trust couldn't be recorded. The file is never left half-written.
#[derive(Debug)]
pub enum TrustError {
    /// `.claude.json` isn't JSON Irori can read, so it isn't touched.
    Unreadable(String),
    /// Something else kept rewriting it through every attempt.
    Busy,
    Io(std::io::Error),
}

impl std::fmt::Display for TrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustError::Unreadable(e) => {
                write!(
                    f,
                    ".claude.json isn't readable JSON, so Irori left it alone: {e}"
                )
            }
            TrustError::Busy => write!(
                f,
                ".claude.json kept changing while Irori tried to record trust; try again"
            ),
            TrustError::Io(e) => write!(f, "couldn't write .claude.json: {e}"),
        }
    }
}

impl std::error::Error for TrustError {}

impl From<std::io::Error> for TrustError {
    fn from(e: std::io::Error) -> Self {
        TrustError::Io(e)
    }
}

/// Records that `dir`'s trust dialog was accepted, in `file`
/// (`.claude.json`). Call it only from the person's confirm in the window.
pub fn record_trust(file: &Path, dir: &str) -> Result<Recorded, TrustError> {
    record_trust_with(file, dir, &mut |_| {})
}

/// The points [`record_trust_with`] calls its hook at, so tests can race it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// The temp file is written; the original is about to be re-checked.
    BeforeRename,
    /// The temp file replaced the original; it's about to be verified.
    AfterRename,
}

pub(crate) fn record_trust_with(
    file: &Path,
    dir: &str,
    hook: &mut dyn FnMut(Stage),
) -> Result<Recorded, TrustError> {
    let key = project_key(dir);
    for attempt in 1..=ATTEMPTS {
        let before = read_opt(file)?;
        let text = before.as_deref().unwrap_or(b"{}");
        if trusted_in(text, &key) {
            return Ok(if attempt == 1 {
                Recorded::Already
            } else {
                Recorded::Written {
                    attempts: attempt - 1,
                }
            });
        }
        let after = with_trust(text, &key)?;
        let tmp = temp_beside(file, attempt);
        write_new(&tmp, &after, file)?;
        hook(Stage::BeforeRename);
        if read_opt(file)? != before {
            let _ = std::fs::remove_file(&tmp);
            continue;
        }
        if let Err(e) = std::fs::rename(&tmp, file) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        hook(Stage::AfterRename);
        if is_trusted(file, dir) {
            return Ok(Recorded::Written { attempts: attempt });
        }
    }
    Err(TrustError::Busy)
}

fn read_opt(file: &Path) -> Result<Option<Vec<u8>>, TrustError> {
    match std::fs::read(file) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn temp_beside(file: &Path, attempt: usize) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".claude.json".into());
    file.with_file_name(format!("{name}.irori-{}-{attempt}.tmp", std::process::id()))
}

/// Writes `bytes` to a new file at `tmp`, synced, with `like`'s
/// permissions when it exists (`.claude.json` is 0600 on a Mac).
fn write_new(tmp: &Path, bytes: &[u8], like: &Path) -> Result<(), TrustError> {
    let mut f = std::fs::File::create(tmp)?;
    let res = (|| {
        f.write_all(bytes)?;
        if let Ok(m) = std::fs::metadata(like) {
            std::fs::set_permissions(tmp, m.permissions())?;
        }
        f.sync_all()
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(tmp);
        return Err(e.into());
    }
    Ok(())
}

type Obj = IndexMap<String, Box<RawValue>>;

/// A JSON object rebuilt on the way to the key: the levels changed are
/// maps, the rest raw text.
#[derive(Serialize)]
#[serde(untagged)]
enum Node {
    Raw(Box<RawValue>),
    Map(IndexMap<String, Node>),
}

fn parse_obj(text: &[u8], what: &str) -> Result<Obj, TrustError> {
    serde_json::from_slice::<Obj>(text).map_err(|e| TrustError::Unreadable(format!("{what}: {e}")))
}

fn raw(s: &str) -> Box<RawValue> {
    RawValue::from_string(s.to_string()).expect("valid JSON literal")
}

fn trusted_in(text: &[u8], key: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Proj {
        #[serde(rename = "hasTrustDialogAccepted", default)]
        accepted: bool,
    }
    #[derive(serde::Deserialize)]
    struct Cfg {
        #[serde(default)]
        projects: std::collections::HashMap<String, Proj>,
    }
    serde_json::from_slice::<Cfg>(text)
        .ok()
        .and_then(|c| c.projects.get(key).map(|p| p.accepted))
        .unwrap_or(false)
}

/// `text` with `projects[key].hasTrustDialogAccepted` set to true and
/// every other value as it was.
fn with_trust(text: &[u8], key: &str) -> Result<Vec<u8>, TrustError> {
    let top = parse_obj(text, ".claude.json")?;
    let mut projects = match top.get("projects") {
        Some(p) => parse_obj(p.get().as_bytes(), "projects")?,
        None => Obj::new(),
    };
    // Keep the project where it was when it existed, else add it last.
    let pos = projects.get_index_of(key);
    let mut proj: IndexMap<String, Node> = match projects.shift_remove(key) {
        Some(p) => parse_obj(p.get().as_bytes(), "the project's entry")?
            .into_iter()
            .map(|(k, v)| (k, Node::Raw(v)))
            .collect(),
        None => IndexMap::new(),
    };
    proj.insert(TRUST_KEY.to_string(), Node::Raw(raw("true")));
    let mut out_projects: IndexMap<String, Node> = projects
        .into_iter()
        .map(|(k, v)| (k, Node::Raw(v)))
        .collect();
    match pos {
        Some(i) => {
            out_projects.shift_insert(i, key.to_string(), Node::Map(proj));
        }
        None => {
            out_projects.insert(key.to_string(), Node::Map(proj));
        }
    }

    let mut out: IndexMap<String, Node> = IndexMap::new();
    let mut placed = false;
    for (k, v) in top {
        if k == "projects" {
            out.insert(k, Node::Map(std::mem::take(&mut out_projects)));
            placed = true;
        } else {
            out.insert(k, Node::Raw(v));
        }
    }
    if !placed {
        out.insert("projects".into(), Node::Map(out_projects));
    }
    let mut bytes =
        serde_json::to_vec_pretty(&out).map_err(|e| TrustError::Unreadable(e.to_string()))?;
    if text.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "numStartups": 178,
  "installMethod": "native",
  "tipsHistory": {
    "b": 2,
    "a": 1
  },
  "projects": {
    "/Users/me": {
      "allowedTools": [],
      "hasTrustDialogAccepted": true
    },
    "/Users/me/projects/irori": {
      "allowedTools": ["Bash(git status)"],
      "mcpServers": {},
      "hasTrustDialogAccepted": false,
      "lastCost": 1.5
    },
    "/Users/me/other": {
      "zeta": 1,
      "alpha": 2
    }
  },
  "userID": "abc"
}
"#;

    fn file_with(text: &str) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".claude.json");
        std::fs::write(&f, text).unwrap();
        (d, f)
    }

    fn value(f: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap()
    }

    fn startups(f: &Path) -> i64 {
        value(f)["numStartups"].as_i64().unwrap()
    }

    /// Claude Code rewriting the file the way it does: read, change its own
    /// key, write a temp file, rename.
    fn claude_bumps(f: &Path, to: i64) {
        let mut v = value(f);
        v["numStartups"] = to.into();
        let tmp = f.with_file_name("claude-own.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        std::fs::rename(&tmp, f).unwrap();
    }

    #[test]
    fn only_the_key_changes_and_order_is_kept() {
        let (_d, f) = file_with(SAMPLE);
        assert!(!is_trusted(&f, "/Users/me/projects/irori"));
        // A trusted parent doesn't make the repo trusted (§7 Trust).
        assert!(is_trusted(&f, "/Users/me"));
        let r = record_trust(&f, "/Users/me/projects/irori/").unwrap();
        assert_eq!(r, Recorded::Written { attempts: 1 });
        assert!(is_trusted(&f, "/Users/me/projects/irori"));

        let mut want: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        want["projects"]["/Users/me/projects/irori"][TRUST_KEY] = true.into();
        assert_eq!(value(&f), want, "nothing else changed");
        let text = std::fs::read_to_string(&f).unwrap();
        let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s} in {text}"));
        // Top-level, project and inner key order all as they were.
        assert!(at("numStartups") < at("installMethod"));
        assert!(at("\"projects\"") < at("userID"));
        assert!(at("\"b\": 2") < at("\"a\": 1"));
        assert!(at("\"/Users/me\"") < at("/Users/me/projects/irori\""));
        assert!(at("/Users/me/projects/irori\"") < at("/Users/me/other"));
        assert!(at("\"zeta\"") < at("\"alpha\""));
        assert!(at("mcpServers") < at("lastCost"));
        assert!(text.ends_with("}\n"));
        assert_eq!(text.matches(TRUST_KEY).count(), 2, "one key per project");
    }

    #[test]
    fn a_new_repo_or_a_new_file_gets_its_entry() {
        let (_d, f) = file_with(SAMPLE);
        record_trust(&f, "/Users/me/projects/new").unwrap();
        let v = value(&f);
        assert_eq!(v["projects"]["/Users/me/projects/new"][TRUST_KEY], true);
        assert_eq!(v["userID"], "abc");

        let (_d2, f2) = file_with(r#"{"numStartups": 1}"#);
        record_trust(&f2, r"C:\work\api").unwrap();
        assert_eq!(value(&f2)["projects"]["C:/work/api"][TRUST_KEY], true);

        let d3 = tempfile::tempdir().unwrap();
        let f3 = d3.path().join(".claude.json");
        record_trust(&f3, "/x").unwrap();
        assert!(is_trusted(&f3, "/x"));
    }

    #[test]
    fn already_trusted_leaves_the_file_alone() {
        let (_d, f) = file_with(SAMPLE);
        let r = record_trust(&f, "/Users/me").unwrap();
        assert_eq!(r, Recorded::Already);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), SAMPLE);
    }

    #[test]
    fn unreadable_json_is_never_overwritten() {
        let (_d, f) = file_with("{ not json");
        let e = record_trust(&f, "/x").unwrap_err();
        assert!(matches!(e, TrustError::Unreadable(_)), "{e}");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{ not json");
        let left: Vec<_> = std::fs::read_dir(f.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 1, "no temp file left: {left:?}");
    }

    /// A session rewrites the file between Irori's read and its rename:
    /// Irori sees the change, starts again from the new file, and both
    /// changes survive.
    #[test]
    fn a_write_underneath_before_the_rename_keeps_both_changes() {
        let (_d, f) = file_with(SAMPLE);
        let mut n = 0;
        let r = record_trust_with(&f, "/Users/me/projects/irori", &mut |s| {
            if s == Stage::BeforeRename && n == 0 {
                n += 1;
                claude_bumps(&f, 179);
            }
        })
        .unwrap();
        assert_eq!(r, Recorded::Written { attempts: 2 });
        assert_eq!(startups(&f), 179, "the session's change kept");
        assert!(is_trusted(&f, "/Users/me/projects/irori"), "and ours");
    }

    /// A session that read the file before Irori's rename writes after it,
    /// dropping Irori's key: the verify catches it and Irori writes it again
    /// on top of the session's file.
    #[test]
    fn a_session_overwriting_ours_after_the_rename_is_caught_and_redone() {
        let (_d, f) = file_with(SAMPLE);
        let stale: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        let mut n = 0;
        let r = record_trust_with(&f, "/Users/me/projects/irori", &mut |s| {
            if s == Stage::AfterRename && n == 0 {
                n += 1;
                let mut v = stale.clone();
                v["numStartups"] = 180.into();
                std::fs::write(&f, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
            }
        })
        .unwrap();
        assert_eq!(r, Recorded::Written { attempts: 2 });
        assert_eq!(startups(&f), 180);
        assert!(is_trusted(&f, "/Users/me/projects/irori"));
    }

    #[test]
    fn a_file_that_never_settles_gives_up_untouched_by_halves() {
        let (_d, f) = file_with(SAMPLE);
        let mut n = 179;
        let e = record_trust_with(&f, "/Users/me/projects/irori", &mut |s| {
            if s == Stage::BeforeRename {
                n += 1;
                claude_bumps(&f, n);
            }
        })
        .unwrap_err();
        assert!(matches!(e, TrustError::Busy), "{e}");
        assert!(!is_trusted(&f, "/Users/me/projects/irori"));
        assert_eq!(startups(&f), n, "the session's last write stands");
        let tmps = std::fs::read_dir(f.parent().unwrap())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".irori-")
            })
            .count();
        assert_eq!(tmps, 0, "no temp files left");
    }

    #[cfg(unix)]
    #[test]
    fn the_files_permissions_are_kept() {
        use std::os::unix::fs::PermissionsExt as _;
        let (_d, f) = file_with(SAMPLE);
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        record_trust(&f, "/Users/me/projects/irori").unwrap();
        let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
