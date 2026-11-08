//! No tool, row or agent can write Claude Code trust (card #449,
//! docs/design/spawned-agents.md §7 "The pre-trust flow" point 4, §14
//! part 5's done-when).
//!
//! Trust is written only by `irori_window::claude_trust::record_trust`,
//! which the window calls from the person's Trust and start and from Settings
//! › Agents' confirmed Trust… (card #456). This test
//! holds the two things that keep it that way:
//! - the crates agents act through (tools, core, coord, inbox, mcp, api,
//!   attach) don't depend on the window crate, so none of them can call it;
//! - no code outside the window crate and the app names `record_trust` or
//!   writes `hasTrustDialogAccepted`.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The crates that may depend on irori-window: itself, the app, and the
/// dev tool (xtask bindings).
const MAY_DEPEND: &[&str] = &["window", "dev"];

#[test]
fn no_agent_facing_crate_depends_on_the_window() {
    let crates = root().join("crates");
    let mut checked = 0;
    for e in std::fs::read_dir(&crates).unwrap() {
        let dir = e.unwrap().path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let toml = dir.join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&toml) else {
            continue;
        };
        checked += 1;
        if MAY_DEPEND.contains(&name.as_str()) {
            continue;
        }
        assert!(
            !text.contains("irori-window"),
            "crates/{name} depends on irori-window, so it could write trust"
        );
    }
    assert!(checked > 20, "read the crates ({checked})");
    for must_not in ["tools", "core", "coord", "inbox", "mcp", "api", "attach"] {
        assert!(
            crates.join(must_not).join("Cargo.toml").exists(),
            "{must_not}"
        );
    }
}

/// The limit on agents starting agents (card #575) has the same shape: only
/// the window's Settings command calls the Core's setter, and the tools
/// crate holds no way to it.
#[test]
fn only_the_window_sets_the_limit_on_agents_starting_agents() {
    let mut files = Vec::new();
    rust_files(&root().join("crates"), &mut files);
    rust_files(&root().join("app"), &mut files);
    let mut callers = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        if text.contains(".set_agent_starts(") {
            callers.push(f);
        }
    }
    let rel = |p: &PathBuf| {
        p.strip_prefix(root())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    };
    for f in &callers {
        let f = rel(f);
        assert!(
            f == "crates/window/src/agents.rs"
                || f == "crates/core/src/service.rs"
                || f.starts_with("crates/window/tests/")
                || f.starts_with("crates/core/tests/"),
            "{f} sets the limit: only the app's Settings may"
        );
    }
    assert!(callers.len() >= 2, "{callers:?}");
    // The Core's one pass-through, for the window.
    let core = std::fs::read_to_string(root().join("crates/core/src/service.rs")).unwrap();
    assert_eq!(core.matches(".set_agent_starts(").count(), 1);
    let tools = std::fs::read_to_string(root().join("crates/tools/src/lib.rs")).unwrap();
    assert!(
        !tools.contains("fn set_agent"),
        "the tools' Backend can't set it"
    );
}

/// "Agents review other people's PRs without asking" (merge-desk §12.8) has
/// the same shape: only the window's Settings command calls the Core's
/// setter, and the tools crate holds no way to it.
#[test]
fn only_the_window_sets_reviews_without_asking() {
    let mut files = Vec::new();
    rust_files(&root().join("crates"), &mut files);
    rust_files(&root().join("app"), &mut files);
    let mut callers = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        if text.contains(".set_reviews_unasked(") {
            callers.push(f);
        }
    }
    let rel = |p: &PathBuf| {
        p.strip_prefix(root())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    };
    for f in &callers {
        let f = rel(f);
        // The inbox's own setter is core's way to push the value in; the
        // value it holds grants nothing by itself.
        assert!(
            f == "crates/window/src/agents.rs"
                || f == "crates/core/src/service.rs"
                || f == "crates/core/src/service/spawn_agents.rs"
                || f.starts_with("crates/window/tests/")
                || f.starts_with("crates/core/tests/")
                || f.starts_with("crates/inbox/"),
            "{f} sets reviews without asking: only the app's Settings may"
        );
    }
    // The Core's one pass-through, for the window.
    let core = std::fs::read_to_string(root().join("crates/core/src/service.rs")).unwrap();
    assert_eq!(core.matches(".set_reviews_unasked(").count(), 2);
    let tools = std::fs::read_to_string(root().join("crates/tools/src/lib.rs")).unwrap();
    assert!(
        !tools.contains("fn set_reviews"),
        "the tools' Backend can't set it"
    );
    // Only the person's own rows count when the setting is read back.
    let spawn = std::fs::read_to_string(root().join("crates/core/src/service/spawn.rs")).unwrap();
    assert!(spawn.contains("ev.actor != \"user\""));
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            if p.file_name()
                .is_some_and(|n| n == "target" || n == "node_modules")
            {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn only_the_window_writes_trust() {
    let r = root();
    let mut files = Vec::new();
    rust_files(&r.join("crates"), &mut files);
    rust_files(&r.join("app"), &mut files);
    let window = r.join("crates/window");
    let mut bad = Vec::new();
    for f in &files {
        if f.starts_with(&window) || f.starts_with(r.join("app")) {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            // Reading the key (the roster's TrustSet, part 1's check) is
            // fine; naming the writer, or setting the key, is not.
            if code.contains("record_trust")
                || (code.contains("hasTrustDialogAccepted") && code.contains("true"))
            {
                bad.push(format!("{}:{}: {}", f.display(), i + 1, code));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "trust written outside the window:\n{}",
        bad.join("\n")
    );
}
