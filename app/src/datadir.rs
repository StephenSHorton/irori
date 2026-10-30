//! The data dir.
//!
//! The rule is Go's `paths.DataDir` (I:internal/paths/paths.go:12-26):
//! `$IRORI_HOME`, else `%LOCALAPPDATA%\irori` on Windows, else `~/.irori`.
//! The window reads device.json through the Core (crates/window); M1's
//! read-only loader went with the shim.

use std::path::{Path, PathBuf};

/// data_dir resolves the data dir from the environment lookup `env`, so
/// tests need not touch the process environment.
pub fn data_dir(env: impl Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    if let Some(dir) = set("IRORI_HOME") {
        return Ok(PathBuf::from(dir));
    }
    if cfg!(windows)
        && let Some(local) = set("LOCALAPPDATA")
    {
        return Ok(Path::new(&local).join("irori"));
    }
    let home = if cfg!(windows) {
        set("USERPROFILE")
    } else {
        set("HOME")
    };
    match home {
        Some(h) => Ok(Path::new(&h).join(".irori")),
        None => Err("paths: no home directory".to_string()),
    }
}

/// process_data_dir is data_dir over the real environment. A dev build
/// (debug assertions on) refuses to run without `IRORI_HOME`, so it never
/// reads the live Go data dir (architecture.md 3.2).
pub fn process_data_dir() -> Result<PathBuf, String> {
    dev_data_dir(|k| std::env::var(k).ok(), cfg!(debug_assertions))
}

/// dev_data_dir is data_dir with the dev-build guard: when `dev` is set,
/// `IRORI_HOME` must name a dir (a copy, never the live one).
pub fn dev_data_dir(env: impl Fn(&str) -> Option<String>, dev: bool) -> Result<PathBuf, String> {
    if dev && env("IRORI_HOME").is_none_or(|v| v.is_empty()) {
        return Err(
            "dev build: set IRORI_HOME to a copy of the data dir; it never reads the live one (architecture.md 3.2)"
                .to_string(),
        );
    }
    data_dir(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn irori_home_wins() {
        let got = data_dir(env(&[
            ("IRORI_HOME", "X:/scratch"),
            ("LOCALAPPDATA", "L:/"),
        ]))
        .unwrap();
        assert_eq!(got, PathBuf::from("X:/scratch"));
    }

    #[test]
    fn empty_irori_home_is_unset() {
        let got = data_dir(env(&[
            ("IRORI_HOME", ""),
            ("LOCALAPPDATA", "L:/la"),
            ("USERPROFILE", "U:/u"),
            ("HOME", "/h"),
        ]))
        .unwrap();
        if cfg!(windows) {
            assert_eq!(got, Path::new("L:/la").join("irori"));
        } else {
            assert_eq!(got, Path::new("/h").join(".irori"));
        }
    }

    #[test]
    fn dev_build_needs_irori_home() {
        let live = [
            ("LOCALAPPDATA", "L:/la"),
            ("HOME", "/h"),
            ("USERPROFILE", "U:/u"),
        ];
        let err = dev_data_dir(env(&live), true).unwrap_err();
        assert!(err.contains("IRORI_HOME"), "{err}");
        let mut empty = live.to_vec();
        empty.push(("IRORI_HOME", ""));
        assert!(dev_data_dir(env(&empty), true).is_err());
        let mut set = live.to_vec();
        set.push(("IRORI_HOME", "X:/scratch"));
        assert_eq!(
            dev_data_dir(env(&set), true).unwrap(),
            PathBuf::from("X:/scratch")
        );
        // A release build keeps Go's rule.
        assert!(dev_data_dir(env(&live), false).is_ok());
    }
}
