//! Local tasks (architecture.md 7.1). There are no CI checks; these run
//! from scripts/check.sh and by hand. Std only, so `cargo xtask` builds in
//! seconds. (The TS bindings export went with the React frontend, #674.)
//!
//! - `cargo xtask no-third-party`: Irori talks only to its own relay and
//!   devices. Fails when Cargo.lock holds a public discovery crate (pkarr,
//!   mDNS, swarm-discovery) or code uses iroh's N0 preset or default
//!   relays. n0's four public relays are the one exception (decision 43,
//!   card #328): behind the per-device switch in irori_net::public, never a
//!   preset.
//! - `cargo xtask one-test-binary`: each crate's integration tests are one
//!   binary, tests/it/main.rs (#566). A test that changes process state
//!   (env, cwd) or counts this process's sockets can't share a process:
//!   add it to [`ALONE`] with why at its top.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Test files that must stay their own binary.
const ALONE: &[&str] = &[
    "crates/capture/tests/repo_git_env.rs",
    "crates/core/tests/no_public_contact.rs",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["no-third-party"] => no_third_party(&root()),
        ["one-test-binary"] => one_test_binary(&root()),
        _ => Err(vec![
            "usage: cargo xtask no-third-party | one-test-binary".into(),
        ]),
    };
    match r {
        Ok(ok) => {
            println!("{ok}");
            ExitCode::SUCCESS
        }
        Err(errs) => {
            for e in errs {
                eprintln!("xtask: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Every .rs file under `dir`, skipping target/.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn no_third_party(root: &Path) -> Result<String, Vec<String>> {
    let mut bad = Vec::new();
    for line in read(&root.join("Cargo.lock")).lines() {
        let Some(name) = line
            .strip_prefix("name = \"")
            .and_then(|l| l.strip_suffix('"'))
        else {
            continue;
        };
        if name == "pkarr"
            || name == "swarm-discovery"
            || name.starts_with("mdns")
            || name.starts_with("iroh-mdns")
        {
            bad.push(format!("discovery crate in Cargo.lock: {name}"));
        }
    }
    let mut files = Vec::new();
    for d in ["crates", "app"] {
        rust_files(&root.join(d), &mut files);
    }
    for f in files {
        for (i, line) in read(&f).lines().enumerate() {
            // Code, not comments.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("presets::N0") || line.contains("RelayMode::Default") {
                let rel = f.strip_prefix(root).unwrap_or(&f).display().to_string();
                bad.push(format!("public relay or discovery preset: {rel}:{}", i + 1));
            }
        }
    }
    if bad.is_empty() {
        Ok("no-third-party: ok".into())
    } else {
        Err(bad)
    }
}

fn one_test_binary(root: &Path) -> Result<String, Vec<String>> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("crates"))
        .map(|rd| rd.flatten().map(|e| e.path().join("tests")).collect())
        .unwrap_or_default();
    dirs.push(root.join("app/tests"));
    let mut bad = Vec::new();
    for dir in dirs.iter().filter(|d| d.is_dir()) {
        let rs: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "rs"))
                    .collect()
            })
            .unwrap_or_default();
        let it = dir.join("it");
        // A crate with one test file is one binary already.
        if !it.is_dir() && rs.len() <= 1 {
            continue;
        }
        for f in &rs {
            let rel = f
                .strip_prefix(root)
                .unwrap_or(f)
                .to_string_lossy()
                .replace('\\', "/");
            if !ALONE.contains(&rel.as_str()) {
                bad.push(format!(
                    "one-test-binary: move {rel} to {}/it/ and add it to main.rs",
                    dir.strip_prefix(root).unwrap_or(dir).display()
                ));
            }
        }
        // A file main.rs doesn't name never compiles, so its tests never run.
        let main = read(&it.join("main.rs"));
        let mods: Vec<String> = std::fs::read_dir(&it)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "rs"))
                    .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        for m in mods.iter().filter(|m| *m != "main") {
            if !main.lines().any(|l| l.trim() == format!("mod {m};")) {
                bad.push(format!(
                    "one-test-binary: add `mod {m};` to {}/it/main.rs",
                    dir.strip_prefix(root).unwrap_or(dir).display()
                ));
            }
        }
    }
    if bad.is_empty() {
        Ok("one-test-binary: ok".into())
    } else {
        Err(bad)
    }
}
