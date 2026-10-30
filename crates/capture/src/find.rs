//! I:internal/capture/search.go (named `find` here, beside the `search` crate): Search and Show dispatch, the outside
//! [`search::Source`]s, and the docs capture indexes itself (capture.md
//! §1, §2.7).
//!
//! What capture indexes, in `apply` (so a replay rebuilds it):
//! - `turn:<id>`: title = the prompt's first line, body = prompt + reply,
//!   with every fenced `<irori-message>` hidden (the index is shared by
//!   every viewer); subagent turns included.
//! - `edit:<id>`: title = `<tool> <path> +adds -dels`, body = the path.
//! - `commit:<sha>`: title = subject, body = subject + file paths, agent =
//!   the committer, else the first linked session.
//! - `pr:<ledger key>`: title = `PR owner/name#n (role)`, at = the newest
//!   entry.
//! - `decision:<id>`: title = statement, body = statement + why + anchor
//!   path.
//!
//! Sources (coord: task, comment; inbox: msg) are pulled, not pushed:
//! before each Search, capture asks each for its docs since the newest
//! Updated it has seen from it, and pages until a short page.

use std::sync::{Arc, LazyLock};

use gocompat::time::GoTime;
use search::{Doc, Error, Hit, Kind, Query, ShowOptions, Shown, Source};

use crate::gotime;
use crate::handle::first_line;
use crate::rows::{EditRow, TurnRow};
use crate::service::{Capture, CommitRec, lock};

/// The ref kinds capture answers itself.
pub const OWN_KINDS: [Kind; 6] = [
    Kind::Turn,
    Kind::Edit,
    Kind::Commit,
    Kind::Pr,
    Kind::Decision,
    Kind::Agent,
];

/// How many docs one `search_docs` call asks a source for.
pub const PULL_PAGE: usize = 500;

/// One registered source and how far the index has read it.
struct SourceState {
    src: Arc<dyn Source>,
    kinds: Vec<Kind>,
    /// The newest Updated seen from it.
    since: GoTime,
}

/// The registered outside sources.
#[derive(Default)]
pub(crate) struct Sources {
    list: Vec<SourceState>,
}

impl Sources {
    fn by_kind(&self, k: Kind) -> Option<&SourceState> {
        self.list.iter().find(|s| s.kinds.contains(&k))
    }
}

impl Capture {
    /// Registers a source for its kinds (the core adds coord and inbox
    /// after [`Capture::new`]). A kind capture owns, or one already
    /// registered, panics: that is a wiring bug. Nothing of a refused
    /// source is registered.
    pub fn add_source(&self, src: Arc<dyn Source>) {
        let mut s = lock(&self.inner.srcs);
        let kinds = src.search_kinds();
        for k in &kinds {
            if OWN_KINDS.contains(k) {
                panic!("capture: AddSource: kind {:?} is capture's own", k.as_str());
            }
            if s.by_kind(*k).is_some() {
                panic!(
                    "capture: AddSource: kind {:?} already has a source",
                    k.as_str()
                );
            }
        }
        s.list.push(SourceState {
            src,
            kinds,
            since: GoTime::ZERO,
        });
    }

    /// Whether a source is registered for `k` (tools and tests).
    pub fn has_source(&self, k: Kind) -> bool {
        lock(&self.inner.srcs).by_kind(k).is_some()
    }

    /// Hits for `q`, best first, at most `q.limit` (default
    /// [`search::DEFAULT_LIMIT`], max [`search::MAX_LIMIT`]; one more,
    /// `MAX_LIMIT + 1`, is let through so a caller showing `MAX_LIMIT` can
    /// still tell there is more). Every source is pulled first.
    pub fn search(&self, q: &Query) -> Result<Vec<Hit>, Error> {
        let mut q = q.clone();
        if q.limit <= 0 {
            q.limit = search::DEFAULT_LIMIT;
        }
        q.limit = q.limit.min(search::MAX_LIMIT + 1);
        self.pull();
        self.inner.idx.search(&q).map_err(Error::Other)
    }

    /// Renders one ref ([`Error::BadRef`] for a malformed one,
    /// [`Error::NotFound`] when nothing has it), paged by `o`.
    pub fn show(&self, r: &str, o: &ShowOptions) -> Result<Shown, Error> {
        let (kind, id) = search::parse_ref(r)?;
        if OWN_KINDS.contains(&kind) {
            return Ok(self.render(kind, &id, &o.viewer)?.page(o));
        }
        let src = lock(&self.inner.srcs).by_kind(kind).map(|s| s.src.clone());
        match src {
            Some(src) => src.search_show(&search::make_ref(kind, &id), o),
            None => Err(Error::NotFound),
        }
    }

    /// How many docs the index holds (tools and tests).
    pub fn index_size(&self) -> usize {
        self.inner.idx.size()
    }

    /// Reads every source's new docs into the index. A source that fails is
    /// skipped this time (its error goes to Status) and retried next search.
    fn pull(&self) {
        let inner = &self.inner;
        let mut errs = Vec::new();
        {
            let mut s = lock(&inner.srcs);
            for st in s.list.iter_mut() {
                loop {
                    let docs = match st.src.search_docs(st.since, PULL_PAGE) {
                        Ok(d) => d,
                        Err(e) => {
                            let ks: Vec<&str> = st.kinds.iter().map(|k| k.as_str()).collect();
                            errs.push(format!("capture: search source [{}]: {e}", ks.join(" ")));
                            break;
                        }
                    };
                    let mut newest = st.since;
                    for mut d in docs.iter().cloned() {
                        let Ok((kind, id)) = search::parse_ref(&d.r#ref) else {
                            continue;
                        };
                        if !st.kinds.contains(&kind) {
                            continue; // a source only fills its own kinds
                        }
                        d.r#ref = search::make_ref(kind, &id);
                        d.kind = kind;
                        if let Err(e) = inner.idx.put(&d) {
                            errs.push(format!("capture: search index: {e}"));
                            continue;
                        }
                        let upd = if d.updated.is_zero() { d.at } else { d.updated };
                        if gotime::after(&upd, &newest) {
                            newest = upd;
                        }
                    }
                    let progressed = gotime::after(&newest, &st.since);
                    st.since = newest;
                    // since is inclusive: a full page that didn't move it
                    // forward would come back the same, so stop there.
                    if docs.len() < PULL_PAGE || !progressed {
                        break;
                    }
                }
            }
        }
        for e in errs {
            inner.set_err(e);
        }
    }
}

/// A viewer no fenced message is shown to (no key holds a NUL).
pub(crate) const HIDE_ALL: &str = "\0";

/// Stands in for a fenced message a viewer may not read.
pub const HIDDEN_MESSAGE: &str =
    "[an Irori message, hidden: only its sender and recipients can read it]";

/// One `<irori-message>` block a wake, reminder or append put in a prompt:
/// the opening tag's attributes, then the body up to the closing tag, or
/// to the end when the prompt was clipped inside it.
static FENCED_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    gocompat::regex::from_go(r"(?is)<irori-message\b([^>]*)>.*?(?:</irori-message\s*>|\z)")
        .expect("fenced message pattern")
});
static FENCE_KEY_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| gocompat::regex::from_go(r#"\bkey="([^"]*)""#).expect("fence key pattern"));

/// Replaces each fenced message in a turn's prompt with [`HIDDEN_MESSAGE`]
/// unless `viewer` may read it: the person (""), the turn's agent (`owner`,
/// the recipient) or the message's sender (its key attribute).
pub fn hide_messages(prompt: &str, viewer: &str, owner: &str) -> String {
    if viewer.is_empty()
        || viewer == owner
        || !gocompat::text::to_lower_str(prompt).contains("<irori-message")
    {
        return prompt.to_string();
    }
    FENCED_RE
        .replace_all(prompt, |caps: &regex::Captures<'_>| {
            let attrs = caps.get(1).map_or("", |m| m.as_str());
            match FENCE_KEY_RE.captures(attrs) {
                Some(k) if k.get(1).map_or("", |m| m.as_str()) == viewer => caps[0].to_string(),
                _ => HIDDEN_MESSAGE.to_string(),
            }
        })
        .into_owned()
}

pub(crate) fn turn_doc(r: &TurnRow) -> Doc {
    let prompt = hide_messages(&r.turn.prompt, HIDE_ALL, &r.key);
    let mut body = prompt.clone();
    if !r.turn.reply.is_empty() {
        body.push('\n');
        body.push_str(&r.turn.reply);
    }
    Doc {
        r#ref: search::make_ref(Kind::Turn, &r.id),
        kind: Kind::Turn,
        at: r.turn.started_at,
        updated: GoTime::ZERO,
        title: first_line(&prompt),
        body,
        agent: r.key.clone(),
        repo_key: r.repo_key.clone(),
        readers: Vec::new(),
    }
}

pub(crate) fn edit_title(r: &EditRow) -> String {
    format!("{} {} +{} -{}", r.tool, r.path, r.adds, r.dels)
}

pub(crate) fn edit_doc(r: &EditRow) -> Doc {
    Doc {
        r#ref: search::make_ref(Kind::Edit, &r.id),
        kind: Kind::Edit,
        at: r.at,
        updated: GoTime::ZERO,
        title: edit_title(r),
        body: r.path.clone(),
        agent: r.key.clone(),
        repo_key: r.repo_key.clone(),
        readers: Vec::new(),
    }
}

pub(crate) fn commit_doc(rec: &CommitRec) -> Doc {
    let c = &rec.row.commit;
    let mut body = c.subject.clone();
    for f in c.files.as_deref().unwrap_or(&[]) {
        body.push('\n');
        body.push_str(&f.path);
    }
    Doc {
        r#ref: search::make_ref(Kind::Commit, &gocompat::text::to_lower_str(&c.sha)),
        kind: Kind::Commit,
        at: c.at,
        updated: GoTime::ZERO,
        title: c.subject.clone(),
        body,
        agent: commit_agent(rec.link.as_ref()),
        repo_key: rec.row.repo_key.clone(),
        readers: Vec::new(),
    }
}

/// Who a commit belongs to: its committer, else the first session behind
/// its linked edits.
fn commit_agent(l: Option<&crate::rows::CommitLink>) -> String {
    let Some(l) = l else { return String::new() };
    if let Some(c) = &l.committer
        && !c.key.is_empty()
    {
        return c.key.clone();
    }
    l.sessions.first().cloned().unwrap_or_default()
}

/// Ledger roles as the ledger orders them: author, touched, reviewer.
fn role_rank(r: &ledger::Role) -> i32 {
    if *r == ledger::Role::AUTHOR {
        0
    } else if *r == ledger::Role::TOUCHED {
        1
    } else {
        2
    }
}

/// Orders a PR's entries best first (as the ledger's owners are).
pub(crate) fn sort_owners(es: &mut [ledger::Entry]) {
    es.sort_by(|a, b| {
        role_rank(&a.role)
            .cmp(&role_rank(&b.role))
            .then_with(|| gotime::cmp(&a.at, &b.at))
            .then_with(|| a.session.cmp(&b.session))
    });
}

/// Folds `e` into a PR's entries as the ledger does: one entry per
/// (session, role), keeping the earliest at and the first non-empty URL.
pub(crate) fn merge_owner(es: &mut Vec<ledger::Entry>, e: ledger::Entry) {
    for x in es.iter_mut() {
        if x.session == e.session && x.role == e.role {
            if gotime::before(&e.at, &x.at) {
                x.at = e.at;
            }
            if x.url.is_empty() {
                x.url = e.url;
            }
            return;
        }
    }
    es.push(e);
}

/// A PR's doc from all its entries, best owner first; at is the newest
/// entry, so a PR someone just touched lists as recent.
pub(crate) fn pr_doc(es: &[ledger::Entry]) -> Doc {
    let best = &es[0];
    let mut at = GoTime::ZERO;
    let mut body = format!("PR {}#{} {}", best.repo, best.number, best.url);
    for e in es {
        if gotime::after(&e.at, &at) {
            at = e.at;
        }
        body.push_str(&format!("\n{} {}", e.role.as_str(), e.source));
    }
    Doc {
        r#ref: best.key.clone(),
        kind: Kind::Pr,
        at,
        updated: GoTime::ZERO,
        title: format!("PR {}#{} ({})", best.repo, best.number, best.role.as_str()),
        body,
        agent: best.session.clone(),
        repo_key: ledger::repo_key(&best.repo),
        readers: Vec::new(),
    }
}

pub(crate) fn decision_doc(d: &coord::decision::Decision) -> Doc {
    let mut body = d.statement.clone();
    if !d.why.is_empty() {
        body.push('\n');
        body.push_str(&d.why);
    }
    let mut rk = String::new();
    if let Some(a) = &d.anchor {
        rk = a.repo_key.clone();
        body.push('\n');
        body.push_str(&a.path);
    }
    Doc {
        r#ref: search::make_ref(Kind::Decision, &d.id),
        kind: Kind::Decision,
        at: d.at,
        updated: GoTime::ZERO,
        title: d.statement.clone(),
        body,
        agent: d.by.key.clone(),
        repo_key: rk,
        readers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(session: &str, role: ledger::Role, min: i64, url: &str) -> ledger::Entry {
        ledger::Entry {
            key: "pr:a/b#1".into(),
            session: session.into(),
            role,
            at: GoTime::unix(1_700_000_000 + min * 60, 0),
            url: url.into(),
            ..Default::default()
        }
    }

    /// TestMergeOwner.
    #[test]
    fn merge_owner_cases() {
        let mut es = Vec::new();
        merge_owner(&mut es, e("a", ledger::Role::AUTHOR, 5, ""));
        merge_owner(&mut es, e("a", ledger::Role::AUTHOR, 2, "u1"));
        merge_owner(&mut es, e("a", ledger::Role::AUTHOR, 9, "u2"));
        merge_owner(&mut es, e("a", ledger::Role::REVIEWER, 1, ""));
        assert_eq!(es.len(), 2);
        assert_eq!(es[0].at, GoTime::unix(1_700_000_000 + 120, 0));
        assert_eq!(es[0].url, "u1");
        assert_eq!(es[1].role, ledger::Role::REVIEWER);
    }
}
