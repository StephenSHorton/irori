//! PR-addressed messages (I:internal/inbox/pr.go).

use gocompat::quote::quote;
use gocompat::time::GoTime;

use crate::messages::clip;
use crate::policy::Cards;
use crate::state::{Inner, trim};
use crate::{Error, ErrorKind, MAX_NOTE, Message, Result, addr};

/// Where a pr send goes.
pub(crate) struct PrRoute {
    pub key: String,
    pub to: String,
    pub repo: String,
    pub for_owner: String,
}

/// `pr:owner/name#n`, `owner/name#n` or a GitHub PR URL as the ledger key
/// `pr:<owner/name>#<n>` (lowercase).
pub fn normalize_pr(s: &str) -> Result<String> {
    match ledger::parse_target(s) {
        Some(r) => Ok(ledger::key(&r.repo, r.number)),
        None if trim(s).is_empty() => Err(Error::wrap(ErrorKind::BadPr, "say which PR")),
        None => Err(Error::wrap(
            ErrorKind::BadPr,
            format!(
                "{}; use pr:<owner>/<name>#<number>, <owner>/<name>#<number> or a github.com/<owner>/<name>/pull/<number> URL",
                quote(&clip(s, 120))
            ),
        )),
    }
}

/// The repo key a PR message's work falls back to: its delivery's repo,
/// or for owner-routed work the PR's repo; `""` for a non-PR message.
pub(crate) fn pr_fallback_repo(m: Option<&Message>, repo_key: &str) -> String {
    let Some(m) = m else {
        return repo_key.to_string();
    };
    if !repo_key.is_empty() || m.to.kind != addr::PR {
        return repo_key.to_string();
    }
    match ledger::parse_target(&m.to.value) {
        Some(r) => ledger::repo_key(&r.repo),
        None => String::new(),
    }
}

impl Inner {
    /// The best-ranked live owner, else the repo's front desk with For
    /// naming the owner, else nobody (held unowned).
    pub(crate) fn route_pr(
        &self,
        me: &str,
        value: &str,
        cards: &Cards,
        now: &GoTime,
    ) -> Result<PrRoute> {
        let key = normalize_pr(value)?;
        let r = ledger::parse_target(&key).expect("normalized PR key parses");
        let mut out = PrRoute {
            key: key.clone(),
            to: String::new(),
            repo: String::new(),
            for_owner: String::new(),
        };
        let owners = self.owners.as_ref().map(|f| f(&key)).unwrap_or_default();
        let mut would = String::new();
        for o in owners {
            if o.is_empty() || o == me {
                continue;
            }
            if cards.contains_key(&o) {
                out.to = o;
                return Ok(out);
            }
            if would.is_empty() {
                would = o;
            }
        }
        out.repo = ledger::repo_key(&r.repo);
        out.to = self.desk(&out.repo, cards, now, &[me]);
        if !would.is_empty() {
            out.for_owner = clip(&self.describe(&would, cards), MAX_NOTE);
        }
        Ok(out)
    }
}
