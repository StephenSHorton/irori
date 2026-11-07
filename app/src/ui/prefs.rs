//! The window's own saved settings (what the React app kept in
//! localStorage: the theme, the sidebar's width, filters, folds): one JSON
//! file in the data dir, `window-prefs.json`, so a scratch copy keeps its
//! own and the live app's stay untouched.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Clone, Debug, Default)]
pub struct Prefs {
    path: Option<PathBuf>,
    map: Map<String, Value>,
}

impl Prefs {
    pub fn load(dir: &Path) -> Prefs {
        let path = dir.join("window-prefs.json");
        let map = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Map<String, Value>>(&b).ok())
            .unwrap_or_default();
        Prefs {
            path: Some(path),
            map,
        }
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.map
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn get_or<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.get(key).unwrap_or(default)
    }

    /// Sets and saves (best effort: a write that fails keeps the value for
    /// this run).
    pub fn set<T: Serialize>(&mut self, key: &str, v: T) {
        let Ok(v) = serde_json::to_value(v) else {
            return;
        };
        if self.map.get(key) == Some(&v) {
            return;
        }
        self.map.insert(key.to_string(), v);
        self.save();
    }

    pub fn remove(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.save();
        }
    }

    fn save(&self) {
        let Some(p) = &self.path else { return };
        if let Ok(b) = serde_json::to_vec_pretty(&self.map) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_saves_and_load_reads_back() {
        let d = tempfile::tempdir().unwrap();
        let mut p = Prefs::load(d.path());
        assert_eq!(p.get::<u32>("w"), None);
        p.set("w", 240u32);
        let q = Prefs::load(d.path());
        assert_eq!(q.get::<u32>("w"), Some(240));
        assert_eq!(q.get_or("missing", 7u32), 7);
    }
}
