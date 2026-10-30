//! I:internal/capture/noise.go: heuristic (shell bashEditDiff) edits are
//! mostly build output. Before one is logged it must name a file inside a
//! known repo that git doesn't ignore, whose extension isn't a binary or
//! build artifact, and add or remove a line; at most 50 of one shell call's
//! edits survive. When git can't say which repo owns a file the edit is
//! kept. Exact edits are never filtered.

use std::collections::{HashMap, HashSet};

use gocompat::text::to_lower_str;
use serde::Serialize;
use sys::cancel::Cancel;

use crate::GIT_TIMEOUT;
use crate::gitlink::GitError;
use crate::gopath;
use crate::roots::{dir_exists, norm_root, rel_to};
use crate::rows::{EditRow, turn_key};
use crate::service::{Inner, lock};
use crate::transcript::{Edit, HEURISTIC, Item};

/// The heuristic edits kept per shell call.
pub const MAX_SHELL_EDITS: usize = 50;
/// The check-ignore cache's bound; cleared when full.
pub const IGNORE_CACHE_MAX: usize = 50000;
/// The path bytes passed to one check-ignore run.
pub const IGNORE_ARG_BYTES: usize = 8000;
/// The per-turn dropped-path sets kept; cleared when full.
pub const NOISE_TURNS_MAX: usize = 4096;

const ARTIFACT_EXTS: &str = "
    .dll .exe .pdb .obj .o .a .lib .so .dylib .exp .ilk .idb .ipdb .iobj .pch .nupkg .snupkg
    .class .jar .war .pyc .pyo .wasm .node .bin .dat .cache .tlog
    .png .jpg .jpeg .gif .webp .ico .bmp .tga .dds .psd .tif .tiff .exr .hdr .ktx
    .mp3 .mp4 .wav .ogg .webm .mov .avi .flac
    .zip .7z .gz .tgz .tar .bz2 .xz .rar .zst .cab .msi
    .ttf .otf .woff .woff2
    .vpk .vmdl_c .vmat_c .vtex_c .vsnd_c .vpcf_c .vmap_c .vanmgrph_c .vagrp_c .vsvg_c .vxml_c .shader_c .vjs_c .vcss_c .sbproj_c
    .rel .dol .iso .rvz .sav
";

/// Heuristic edits discarded as noise (not persisted, not replayed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Dropped {
    /// Binary or build-artifact extension, or under .git/node_modules.
    pub artifact: i64,
    /// Not inside a known git worktree.
    #[serde(rename = "outsideRepo")]
    pub outside_repo: i64,
    /// git check-ignore says the path is ignored.
    pub ignored: i64,
    /// Past [`MAX_SHELL_EDITS`] in one shell call.
    #[serde(rename = "overCap")]
    pub over_cap: i64,
    /// No lines added or removed.
    pub unchanged: i64,
}

/// A path capture never keeps a heuristic edit for.
pub fn is_artifact(path: &str) -> bool {
    let p = to_lower_str(&gopath::to_slash(path));
    let ext = path_ext(&p);
    if !ext.is_empty() && ARTIFACT_EXTS.split_whitespace().any(|e| e == ext) {
        return true;
    }
    p.split('/')
        .any(|seg| seg == ".git" || seg == "node_modules")
}

/// The extension of a forward-slash path ("" when none).
fn path_ext(p: &str) -> &str {
    let base = match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None => p,
    };
    match base.rfind('.') {
        Some(i) if i > 0 => &base[i..],
        _ => "",
    }
}

fn ignore_key(root: &str, rel: &str) -> String {
    format!("{}|{}", norm_root(root), to_lower_str(rel))
}

/// An edit's absolute, cleaned path (as onEdit computes it).
pub(crate) fn edit_abs(e: &Edit) -> String {
    let mut abs = e.path.clone();
    if !gopath::is_abs(&abs) && !e.cwd.is_empty() {
        abs = gopath::join(&[&e.cwd, &abs]);
    }
    gopath::clean(&gopath::from_slash(&abs))
}

/// A logged edit's absolute path.
pub(crate) fn row_abs(e: &EditRow) -> String {
    if !e.root.is_empty() {
        return gopath::join(&[&e.root, &gopath::from_slash(&e.path)]);
    }
    e.path.clone()
}

/// Compares an edit's path with the turn's EditPaths entries.
pub fn edit_path_key(p: &str) -> String {
    to_lower_str(&gopath::to_slash(&gopath::clean(&gopath::from_slash(p))))
}

/// A `.git` entry in some directory above `path`, existing or not.
fn under_git(path: &str) -> bool {
    let mut d = gopath::dir(path);
    loop {
        if std::fs::symlink_metadata(gopath::join(&[&d, ".git"])).is_ok() {
            return true;
        }
        let up = gopath::dir(&d);
        if up == d {
            return false;
        }
        d = up;
    }
}

impl Inner {
    /// Which of `rels` (repo-relative, forward slashes) `root` ignores, by
    /// lowercased rel. A failed run answers "not ignored" for its batch and
    /// caches nothing.
    fn ignored(&self, cancel: &Cancel, root: &str, rels: &[String]) -> HashSet<String> {
        let mut out = HashSet::new();
        let mut ask: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for rel in rels {
            let k = to_lower_str(rel);
            if !seen.insert(k.clone()) {
                continue;
            }
            if let Some(&ig) = lock(&self.ignores).get(&ignore_key(root, rel)) {
                if ig {
                    out.insert(k);
                }
                continue;
            }
            ask.push(rel.clone());
        }
        let mut rest = &ask[..];
        while !rest.is_empty() {
            let (mut n, mut size) = (0, 0);
            while n < rest.len() && (n == 0 || size + rest[n].len() < IGNORE_ARG_BYTES) {
                size += rest[n].len() + 1;
                n += 1;
            }
            let batch = &rest[..n];
            rest = &rest[n..];
            let Some(hits) = self.check_ignore(cancel, root, batch) else {
                continue;
            };
            for rel in batch {
                let k = to_lower_str(rel);
                let ig = hits.contains(&k);
                {
                    let mut c = lock(&self.ignores);
                    if c.len() >= IGNORE_CACHE_MAX {
                        c.clear();
                    }
                    c.insert(ignore_key(root, rel), ig);
                }
                if ig {
                    out.insert(k);
                }
            }
        }
        out
    }

    /// One `git check-ignore` over a batch. Exit 1 means none is ignored;
    /// any other failure is `None`.
    fn check_ignore(
        &self,
        cancel: &Cancel,
        root: &str,
        batch: &[String],
    ) -> Option<HashSet<String>> {
        let mut args: Vec<&str> = vec!["-c", "core.quotePath=false", "check-ignore", "--"];
        args.extend(batch.iter().map(String::as_str));
        match self.opts.git.run(cancel, GIT_TIMEOUT, root, &args) {
            Err(GitError::Exit { code: 1, .. }) => Some(HashSet::new()),
            Err(_) => None,
            Ok(stdout) => Some(
                stdout
                    .split('\n')
                    .map(|p| p.trim_end_matches('\r'))
                    .filter(|p| !p.is_empty())
                    .map(|p| to_lower_str(&gopath::to_slash(p)))
                    .collect(),
            ),
        }
    }

    /// The worktree root that judges a heuristic edit of `abs`, and whether
    /// its owner can't be known (then it is kept unfiltered).
    fn noise_root(&self, cancel: &Cancel, cwd: &str, abs: &str) -> (String, bool) {
        if !gopath::is_abs(abs) || !under_git(abs) {
            return (String::new(), false);
        }
        if !cwd.is_empty() && gopath::is_abs(cwd) && !dir_exists(cwd) && rel_to(cwd, abs).is_some()
        {
            return (String::new(), true);
        }
        let root = self.roots.for_file(cancel, abs);
        let unsure = root.is_empty();
        (root, unsure)
    }

    /// The indexes of `items`' heuristic edits to discard, counted in the
    /// fold's Dropped.
    pub(crate) fn drop_noise(&self, cancel: &Cancel, items: &[Item]) -> HashSet<usize> {
        let mut drop = HashSet::new();
        let mut stats = Dropped::default();
        let mut by_root: HashMap<String, Vec<(usize, String, String)>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let Item::Edit(e) = it else { continue };
            if e.fidelity != HEURISTIC {
                continue;
            }
            let abs = edit_abs(e);
            if is_artifact(&abs) {
                stats.artifact += 1;
                drop.insert(i);
                continue;
            }
            if e.adds == 0 && e.dels == 0 {
                stats.unchanged += 1;
                drop.insert(i);
                continue;
            }
            let (root, unsure) = self.noise_root(cancel, &e.cwd, &abs);
            if unsure {
                continue;
            }
            let Some(rel) = rel_to(&root, &abs) else {
                stats.outside_repo += 1;
                drop.insert(i);
                continue;
            };
            let n = norm_root(&root);
            if !by_root.contains_key(&n) {
                order.push(n.clone());
            }
            by_root.entry(n).or_default().push((i, root, rel));
        }
        for n in &order {
            let cands = &by_root[n];
            let rels: Vec<String> = cands.iter().map(|c| c.2.clone()).collect();
            let ig = self.ignored(cancel, &cands[0].1, &rels);
            for (i, _, rel) in cands {
                if ig.contains(&to_lower_str(rel)) {
                    stats.ignored += 1;
                    drop.insert(*i);
                }
            }
        }
        // Cap what survived, per shell call, in transcript order.
        let mut per_call: HashMap<&str, usize> = HashMap::new();
        for (i, it) in items.iter().enumerate() {
            let Item::Edit(e) = it else { continue };
            if e.fidelity != HEURISTIC || drop.contains(&i) {
                continue;
            }
            let n = per_call.entry(e.tool_use_id.as_str()).or_default();
            *n += 1;
            if *n > MAX_SHELL_EDITS {
                stats.over_cap += 1;
                drop.insert(i);
            }
        }
        if !drop.is_empty() {
            let mut f = self.write();
            f.dropped.artifact += stats.artifact;
            f.dropped.outside_repo += stats.outside_repo;
            f.dropped.ignored += stats.ignored;
            f.dropped.over_cap += stats.over_cap;
            f.dropped.unchanged += stats.unchanged;
        }
        drop
    }

    /// Records that a turn dropped (noisy) or kept an edit for a path. Kept
    /// wins. Only turns that dropped something are tracked.
    pub(crate) fn note_path(&self, key: &str, e: &Edit, noisy: bool) {
        let tk = turn_key(key, &e.agent_id, &e.turn_id);
        let pk = edit_path_key(&e.path);
        let mut f = self.write();
        if !f.noise_paths.contains_key(&tk) {
            if !noisy {
                return;
            }
            if f.noise_paths.len() >= NOISE_TURNS_MAX {
                f.noise_paths.clear();
            }
            let seed: HashMap<String, bool> = f
                .edits_by_turn
                .get(&tk)
                .map(|ix| {
                    ix.iter()
                        .map(|&i| (edit_path_key(&row_abs(&f.edit_list[i])), false))
                        .collect()
                })
                .unwrap_or_default();
            f.noise_paths.insert(tk.clone(), seed);
        }
        let s = f.noise_paths.get_mut(&tk).expect("inserted above");
        if noisy {
            s.entry(pk).or_insert(true);
            return;
        }
        s.insert(pk, false);
    }
}

/// A turn's EditPaths without the dropped heuristic paths.
pub(crate) fn strip_noise(
    noise: &HashMap<String, HashMap<String, bool>>,
    key: &str,
    agent_id: &str,
    turn_id: &str,
    paths: &[String],
) -> Vec<String> {
    let Some(s) = noise.get(&turn_key(key, agent_id, turn_id)) else {
        return paths.to_vec();
    };
    if s.is_empty() {
        return paths.to_vec();
    }
    paths
        .iter()
        .filter(|p| !s.get(&edit_path_key(p)).copied().unwrap_or(false))
        .cloned()
        .collect()
}
