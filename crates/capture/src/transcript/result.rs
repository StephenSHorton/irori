//! I:internal/capture/transcript/result.go: tool results into edits and
//! bash runs.

use std::collections::HashSet;

use gocompat::json::{Out, Schema, Value};
use gocompat::time::GoTime;

use super::parser::{Block, Parser, Row, join_text, raw_string};
use super::source::path_key;
use super::target::{Hunk, ToolInput, clip_bytes, decode_input, rel_path, unified_diff};
use super::{BashRun, EXACT, Edit, HEURISTIC, Item, MAX_STDOUT};
use crate::jsonx::{
    exact_members, field_or_zero, get, out_bool, out_int, out_items, out_opt_str, out_str,
    out_strings, trim_space_bytes,
};

/// A tool_use waiting for (or past) its result. `turn_gen` identifies the
/// turn it belongs to (Go compares the turn pointer).
#[derive(Debug, Clone)]
pub(crate) struct ToolUse {
    pub name: String,
    pub input: Option<Vec<u8>>,
    pub turn_gen: u64,
    pub turn_id: String,
    /// In the turn's tools; `None` when past MAX_TOOLS_SHOWN.
    pub index: Option<usize>,
    /// The tool_use row's timestamp.
    pub at: GoTime,
}

#[derive(Debug, Clone, Default)]
struct EditStep {
    old_string: String,
    new_string: String,
    replace_all: bool,
    old_string_c: String,
    new_string_c: String,
    replace_all_c: bool,
}

impl EditStep {
    fn parts(&self) -> (&str, &str, bool) {
        if !self.old_string.is_empty() || !self.new_string.is_empty() {
            return (&self.old_string, &self.new_string, self.replace_all);
        }
        (&self.old_string_c, &self.new_string_c, self.replace_all_c)
    }
}

#[derive(Debug, Clone, Default)]
struct BashEditDiff {
    files: Vec<(String, Vec<Hunk>)>,
    changed_files: Vec<String>,
}

/// The union of the toolUseResult fields the parser reads.
#[derive(Debug, Clone, Default)]
struct ToolResult {
    typ: String,
    file_path: String,
    notebook_path: String,
    old_string: String,
    new_string: String,
    replace_all: bool,
    edits: Vec<EditStep>,
    original_file: Option<String>,
    content: Option<String>,
    structured_patch: Vec<Hunk>,
    staged: bool,
    error: String,
    stdout: Option<String>,
    bash_edit_diff: Option<BashEditDiff>,
    git_operation: Vec<u8>,
    keys: HashSet<String>,
}

fn hunk_schema() -> Schema {
    Schema::object(vec![
        ("oldStart", Schema::Int(64)),
        ("oldLines", Schema::Int(64)),
        ("newStart", Schema::Int(64)),
        ("newLines", Schema::Int(64)),
        ("lines", Schema::Slice(Box::new(Schema::String))),
    ])
}

fn hunk_of(o: &Out) -> Hunk {
    Hunk {
        old_start: out_int(o.get("oldStart")),
        old_lines: out_int(o.get("oldLines")),
        new_start: out_int(o.get("newStart")),
        new_lines: out_int(o.get("newLines")),
        lines: out_strings(o.get("lines")).unwrap_or_default(),
    }
}

fn hunks_of(o: &Out) -> Vec<Hunk> {
    o.items().iter().map(hunk_of).collect()
}

/// Reads an object toolUseResult field by field, so one oddly typed field
/// doesn't lose the rest; `None` for anything that is not an object.
fn decode_result(raw: Option<&[u8]>) -> Option<ToolResult> {
    let raw = trim_space_bytes(raw?);
    if raw.first() != Some(&b'{') {
        return None;
    }
    let v = gocompat::json::parse(raw).ok()?;
    let m = exact_members(&v)?;
    let f = |k: &str, s: &Schema| field_or_zero(get(&m, k), s);
    let mut tr = ToolResult {
        keys: m.iter().map(|(k, _)| k.to_string()).collect(),
        ..ToolResult::default()
    };
    tr.typ = out_str(Some(&f("type", &Schema::String)));
    tr.file_path = out_str(Some(&f("filePath", &Schema::String)));
    tr.notebook_path = out_str(Some(&f("notebook_path", &Schema::String)));
    tr.old_string = out_str(Some(&f("oldString", &Schema::String)));
    tr.new_string = out_str(Some(&f("newString", &Schema::String)));
    tr.replace_all = out_bool(Some(&f("replaceAll", &Schema::Bool)));
    let step = Schema::object(vec![
        ("old_string", Schema::String),
        ("new_string", Schema::String),
        ("replace_all", Schema::Bool),
        ("oldString", Schema::String),
        ("newString", Schema::String),
        ("replaceAll", Schema::Bool),
    ]);
    tr.edits = out_items(Some(&f("edits", &Schema::Slice(Box::new(step)))))
        .iter()
        .map(|o| EditStep {
            old_string: out_str(o.get("old_string")),
            new_string: out_str(o.get("new_string")),
            replace_all: out_bool(o.get("replace_all")),
            old_string_c: out_str(o.get("oldString")),
            new_string_c: out_str(o.get("newString")),
            replace_all_c: out_bool(o.get("replaceAll")),
        })
        .collect();
    let ptr_str = Schema::Ptr(Box::new(Schema::String));
    tr.original_file = out_opt_str(Some(&f("originalFile", &ptr_str)));
    tr.content = out_opt_str(Some(&f("content", &ptr_str)));
    tr.structured_patch = hunks_of(&f(
        "structuredPatch",
        &Schema::Slice(Box::new(hunk_schema())),
    ));
    tr.staged = out_bool(Some(&f("staged", &Schema::Bool)));
    tr.error = out_str(Some(&f("error", &Schema::String)));
    tr.stdout = out_opt_str(Some(&f("stdout", &ptr_str)));
    let bed = Schema::Ptr(Box::new(Schema::object(vec![
        (
            "files",
            Schema::Slice(Box::new(Schema::object(vec![
                ("filePath", Schema::String),
                ("hunks", Schema::Slice(Box::new(hunk_schema()))),
            ]))),
        ),
        ("changedFiles", Schema::Slice(Box::new(Schema::String))),
    ])));
    if let Out::Ptr(Some(b)) = f("bashEditDiff", &bed) {
        tr.bash_edit_diff = Some(BashEditDiff {
            files: out_items(b.get("files"))
                .iter()
                .map(|o| {
                    (
                        out_str(o.get("filePath")),
                        o.get("hunks").map(hunks_of).unwrap_or_default(),
                    )
                })
                .collect(),
            changed_files: out_strings(b.get("changedFiles")).unwrap_or_default(),
        });
    }
    if let Some(g) = get(&m, "gitOperation")
        && matches!(g, Value::Object(_))
        && let Ok(Out::Raw(Some(b))) = gocompat::json::unmarshal_value(g, &Schema::Raw)
    {
        tr.git_operation = b;
    }
    Some(tr)
}

/// Names the tool behind a result whose tool_use was never seen (a file
/// read from the middle, after a compaction).
fn infer_tool(tr: Option<&ToolResult>) -> &'static str {
    let Some(tr) = tr else {
        return "";
    };
    let k = |s: &str| tr.keys.contains(s);
    if k("oldString") || (k("edits") && k("filePath")) {
        "Edit"
    } else if (tr.typ == "create" || tr.typ == "update") && k("content") && k("filePath") {
        "Write"
    } else if k("notebook_path") && k("cell_id") {
        "NotebookEdit"
    } else if k("stdout") && k("stderr") {
        "Bash"
    } else {
        ""
    }
}

/// Bash, and PowerShell on Windows: tools whose input.command is a shell
/// command line.
fn is_shell(name: &str) -> bool {
    name == "Bash" || name == "PowerShell"
}

fn first_non_empty<'a>(ss: &[&'a str]) -> &'a str {
    ss.iter().copied().find(|s| !s.is_empty()).unwrap_or("")
}

/// The path written in a patch header: cwd-relative when under cwd, with
/// forward slashes.
fn patch_path(path: &str, cwd: &str) -> String {
    rel_path(path, cwd).replace('\\', "/")
}

impl Parser {
    /// Handles one tool_result block and, for the row's first block, the
    /// row's toolUseResult.
    pub(crate) fn result(&mut self, r: &Row, at: GoTime, b: &Block, raw: Option<&[u8]>) {
        let id = b.tool_use_id.clone();
        let is_err = b.is_error;
        let c = self.calls.remove(&id);
        if c.is_none() && self.cur.is_none() {
            self.implicit(r, at);
        }
        let cur_gen = self.cur.as_ref().map(|t| t.serial);
        let (turn_gen, turn_id) = match &c {
            Some(c) => (Some(c.turn_gen), c.turn_id.clone()),
            None => {
                let t = self.cur.as_ref().expect("implicit opened a turn");
                (Some(t.serial), t.t.id.clone())
            }
        };
        if let Some(c) = &c
            && Some(c.turn_gen) == cur_gen
            && let Some(i) = c.index
            && let Some(cur) = self.cur.as_mut()
            && i < cur.t.tools.len()
        {
            let tc = &mut cur.t.tools[i];
            tc.done = true;
            tc.is_error = is_err;
        }
        let tr = decode_result(raw);
        let (name, input): (String, ToolInput) = match &c {
            Some(c) => (c.name.clone(), decode_input(c.input.as_deref())),
            None => (infer_tool(tr.as_ref()).to_string(), ToolInput::default()),
        };
        let base = Edit {
            session_id: self.src.session_id.clone(),
            agent_id: self.src.agent_id.clone(),
            turn_id: turn_id.clone(),
            tool_use_id: id.clone(),
            row_uuid: r.uuid.clone(),
            tool: name.clone(),
            cwd: r.cwd.clone(),
            at,
            ..Edit::default()
        };
        let mut edits = Vec::new();
        match (name.as_str(), &tr) {
            ("Edit", Some(tr)) if !is_err && !tr.staged => {
                edits = edit_edit(base, tr, &input, &r.cwd)
            }
            ("Write", Some(tr)) if !is_err && !tr.staged => {
                edits = write_edit(base, tr, &input, &r.cwd)
            }
            ("NotebookEdit", Some(tr)) if !is_err && !tr.staged && tr.error.is_empty() => {
                edits = notebook_edit(base, tr, &input, &r.cwd)
            }
            ("Bash" | "PowerShell", Some(tr)) => {
                if let Some(d) = &tr.bash_edit_diff {
                    edits = bash_edits(base, d, &r.cwd);
                }
            }
            _ => {}
        }
        for e in edits {
            self.add_path(turn_gen, &e.path);
            self.emit(Item::Edit(e));
        }
        if is_shell(&name) {
            let mut run = BashRun {
                tool: name.clone(),
                session_id: self.src.session_id.clone(),
                agent_id: self.src.agent_id.clone(),
                turn_id,
                tool_use_id: id,
                cwd: r.cwd.clone(),
                command: input.command.clone(),
                is_error: is_err,
                at,
                ..BashRun::default()
            };
            if let Some(c) = &c {
                run.started_at = c.at;
            }
            let raw = raw.unwrap_or(&[]);
            if let Some(tr) = &tr {
                if let Some(s) = &tr.stdout {
                    run.stdout = clip_bytes(s, MAX_STDOUT);
                }
                run.git_operation = tr.git_operation.clone();
            } else if raw.first() == Some(&b'"') {
                // An error result is a plain string.
                run.stdout = clip_bytes(&raw_string(Some(raw)), MAX_STDOUT);
            } else {
                let bs = super::parser::content_blocks(b.content.as_deref());
                run.stdout = clip_bytes(&join_text(&bs), MAX_STDOUT);
            }
            self.emit(Item::Bash(run));
        }
    }

    /// Appends `path` to the turn's EditPaths once, while the turn is open.
    fn add_path(&mut self, turn_gen: Option<u64>, path: &str) {
        let Some(cur) = self.cur.as_mut() else {
            return;
        };
        if turn_gen != Some(cur.serial) || path.is_empty() {
            return;
        }
        let k = path_key(path);
        if cur.paths.contains(&k) {
            return;
        }
        cur.paths.insert(k);
        cur.t.edit_paths.push(path.to_string());
    }
}

fn edit_edit(mut e: Edit, tr: &ToolResult, i: &ToolInput, cwd: &str) -> Vec<Edit> {
    e.path = first_non_empty(&[&tr.file_path, &i.file_path]).to_string();
    if e.path.is_empty() {
        return Vec::new();
    }
    e.fidelity = EXACT.into();
    (e.patch, e.adds, e.dels) = unified_diff(&patch_path(&e.path, cwd), &tr.structured_patch);
    let Some(orig) = &tr.original_file else {
        return vec![e];
    };
    e.before = Some(orig.as_bytes().to_vec());
    let single;
    let steps: &[EditStep] = if tr.edits.is_empty() {
        single = [EditStep {
            old_string_c: tr.old_string.clone(),
            new_string_c: tr.new_string.clone(),
            replace_all_c: tr.replace_all,
            ..EditStep::default()
        }];
        &single
    } else {
        &tr.edits
    };
    let mut cur = orig.clone();
    for s in steps {
        let (old, repl, all) = s.parts();
        if old.is_empty() || !cur.contains(old) {
            return vec![e];
        }
        cur = if all {
            cur.replace(old, repl)
        } else {
            cur.replacen(old, repl, 1)
        };
    }
    e.after = Some(cur.into_bytes());
    vec![e]
}

fn write_edit(mut e: Edit, tr: &ToolResult, i: &ToolInput, cwd: &str) -> Vec<Edit> {
    e.path = first_non_empty(&[&tr.file_path, &i.file_path]).to_string();
    if e.path.is_empty() {
        return Vec::new();
    }
    e.fidelity = EXACT.into();
    e.create = tr.typ == "create";
    if let Some(o) = &tr.original_file
        && !e.create
    {
        e.before = Some(o.as_bytes().to_vec());
    }
    let mut hunks = tr.structured_patch.clone();
    if let Some(c) = &tr.content {
        e.after = Some(c.as_bytes().to_vec());
        if hunks.is_empty() && e.create {
            hunks = create_hunk(c);
        }
    }
    (e.patch, e.adds, e.dels) = unified_diff(&patch_path(&e.path, cwd), &hunks);
    vec![e]
}

/// The all-added hunk for a new file's content.
fn create_hunk(content: &str) -> Vec<Hunk> {
    if content.is_empty() {
        return Vec::new();
    }
    let body = content.strip_suffix('\n').unwrap_or(content);
    let lines: Vec<&str> = body.split('\n').collect();
    vec![Hunk {
        old_start: 0,
        old_lines: 0,
        new_start: 1,
        new_lines: lines.len() as i64,
        lines: lines
            .iter()
            .map(|l| format!("+{}", l.strip_suffix('\r').unwrap_or(l)))
            .collect(),
    }]
}

fn notebook_edit(mut e: Edit, tr: &ToolResult, i: &ToolInput, cwd: &str) -> Vec<Edit> {
    e.path = first_non_empty(&[&tr.notebook_path, &tr.file_path, &i.notebook_path]).to_string();
    if e.path.is_empty() {
        return Vec::new();
    }
    e.fidelity = EXACT.into();
    (e.patch, e.adds, e.dels) = unified_diff(&patch_path(&e.path, cwd), &tr.structured_patch);
    vec![e]
}

fn bash_edits(base: Edit, d: &BashEditDiff, cwd: &str) -> Vec<Edit> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for path in &d.changed_files {
        let k = path_key(path);
        if path.is_empty() || seen.contains(&k) {
            continue;
        }
        seen.insert(k.clone());
        let mut e = base.clone();
        e.path = path.clone();
        e.fidelity = HEURISTIC.into();
        if let Some((_, hunks)) = d.files.iter().find(|(fp, _)| path_key(fp) == k) {
            (e.patch, e.adds, e.dels) = unified_diff(&patch_path(path, cwd), hunks);
        }
        out.push(e);
    }
    out
}
