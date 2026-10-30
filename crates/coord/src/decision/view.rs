//! The decisions fold (I:internal/decision/view.go).

use std::collections::HashMap;
use std::sync::RwLock;

use super::{Decision, Error, Party};
use crate::gojson::omit;

/// Narrows [`View::list`]. Zero values mean any.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Anchored in this repo.
    pub repo_key: String,
    /// Anchored at this repo-relative path (exact match).
    pub path: String,
    /// Case-insensitive substring of statement or why.
    pub query: String,
    /// Include superseded decisions.
    pub all: bool,
    /// Default 50, max 500.
    pub limit: i64,
}

/// A decision in a list, with who replaced it (Go embeds `Decision`, so
/// its fields come first, in order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listed {
    pub decision: Decision,
    pub superseded_by: Vec<String>,
}

impl serde::Serialize for Listed {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let d = &self.decision;
        let mut st = s.serialize_struct("Listed", 10)?;
        st.serialize_field("id", &d.id)?;
        st.serialize_field("by", &d.by)?;
        st.serialize_field("statement", &d.statement)?;
        if !omit(&d.why) {
            st.serialize_field("why", &d.why)?;
        }
        if let Some(a) = &d.anchor {
            st.serialize_field("anchor", a)?;
        }
        if !omit(&d.supersedes) {
            st.serialize_field("supersedes", &d.supersedes)?;
        }
        if let Some(src) = &d.source {
            st.serialize_field("source", src)?;
        }
        st.serialize_field("trust", &d.trust)?;
        st.serialize_field("at", &d.at)?;
        if !omit(&self.superseded_by) {
            st.serialize_field("supersededBy", &self.superseded_by)?;
        }
        st.end()
    }
}

#[derive(Default)]
struct Inner {
    by_id: HashMap<String, (Decision, u64)>,
    /// target -> superseding ids, in add order.
    superseded_by: HashMap<String, Vec<String>>,
    seq: u64,
}

/// Folds decisions. Safe for concurrent use.
#[derive(Default)]
pub struct View {
    inner: RwLock<Inner>,
}

impl View {
    pub fn new() -> View {
        View::default()
    }

    /// Folds one decision (idempotent on id). Supersedes naming unknown
    /// ids are kept and apply if those ids arrive later.
    pub fn add(&self, d: Decision) {
        if d.id.is_empty() {
            return;
        }
        let mut v = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if v.by_id.contains_key(&d.id) {
            return;
        }
        v.seq += 1;
        let seq = v.seq;
        for t in &d.supersedes {
            if t.is_empty() || *t == d.id {
                continue;
            }
            let list = v.superseded_by.entry(t.clone()).or_default();
            if !list.contains(&d.id) {
                list.push(d.id.clone());
            }
        }
        v.by_id.insert(d.id.clone(), (d, seq));
    }

    pub fn get(&self, id: &str) -> Option<Decision> {
        let v = self.inner.read().unwrap_or_else(|e| e.into_inner());
        v.by_id.get(id).map(|(d, _)| d.clone())
    }

    /// The person (`by.kind == "user"`) may supersede anything; an agent
    /// only decisions whose `by.key` is its own.
    pub fn can_supersede(&self, by: &Party, target: &str) -> Result<(), Error> {
        let v = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some((d, _)) = v.by_id.get(target) else {
            return Err(Error::NoDecision);
        };
        if by.kind == "user" || (!by.key.is_empty() && by.key == d.by.key) {
            return Ok(());
        }
        Err(Error::Supersede)
    }

    /// Matches, newest `at` first, then the later added first.
    pub fn list(&self, f: &Filter) -> Vec<Listed> {
        let limit = if f.limit <= 0 { 50 } else { f.limit }.min(500) as usize;
        let fpath = f.path.replace('\\', "/");
        let q = gocompat::text::to_lower_str(crate::names::trim_space(&f.query));
        let v = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut hits: Vec<&(Decision, u64)> = v
            .by_id
            .iter()
            .filter(|(id, (d, _))| {
                if !f.all && v.superseded_by.get(*id).is_some_and(|l| !l.is_empty()) {
                    return false;
                }
                if !f.repo_key.is_empty()
                    && d.anchor.as_ref().is_none_or(|a| a.repo_key != f.repo_key)
                {
                    return false;
                }
                if !fpath.is_empty() && d.anchor.as_ref().is_none_or(|a| a.path != fpath) {
                    return false;
                }
                if !q.is_empty()
                    && !gocompat::text::to_lower_str(&d.statement).contains(&q)
                    && !gocompat::text::to_lower_str(&d.why).contains(&q)
                {
                    return false;
                }
                true
            })
            .map(|(_, e)| e)
            .collect();
        hits.sort_by(|a, b| crate::gotime::cmp(&b.0.at, &a.0.at).then_with(|| b.1.cmp(&a.1)));
        hits.truncate(limit);
        hits.into_iter()
            .map(|(d, _)| Listed {
                decision: d.clone(),
                superseded_by: v.superseded_by.get(&d.id).cloned().unwrap_or_default(),
            })
            .collect()
    }
}
