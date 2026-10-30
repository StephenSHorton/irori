//! The inbox as a search source for `msg:<id>` refs
//! (I:internal/inbox/search.go). [`Inbox::search_docs`] and
//! [`Inbox::search_show`] answer in the crate's own shapes ([`MsgDoc`],
//! [`MsgShown`], unpaged); the `search::Source` impl at the end wraps them
//! in the search crate's types and pages the text, as Go's does.
//!
//! A message is searchable once some delivery of it has had a first row
//! outside Held/Dismissed; its readers are its agent sender and the
//! recipients of its non-held deliveries (owners excluded), never empty.

use std::fmt::Write as _;

use gocompat::time::GoTime;

use crate::state::{Drec, Inner};
use crate::{Inbox, Message, SYSTEM_KEY, addr, mode, st, tm};

/// `search.MaxRelated`.
pub const MAX_RELATED: usize = 12;
/// One SearchDocs page when the caller gives no limit.
pub const MAX_SEARCH_DOCS: usize = 500;

/// One message as the index sees it (search.Doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgDoc {
    pub ref_: String,
    pub at: GoTime,
    pub updated: GoTime,
    pub title: String,
    pub body: String,
    pub agent: String,
    pub repo_key: String,
    pub readers: Vec<String>,
}

/// A message's detail for a viewer (search.Shown before paging).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgShown {
    pub ref_: String,
    pub title: String,
    pub text: String,
    pub related: Vec<String>,
}

/// `search.CanRead`.
pub fn can_read(viewer: &str, readers: &[String]) -> bool {
    viewer.is_empty() || readers.is_empty() || readers.iter().any(|r| r == viewer)
}

/// Folds s onto one line.
pub(crate) fn one_line(s: &str) -> String {
    s.split(gocompat::text::is_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn msg_ref(id: &str) -> String {
    format!("msg:{id}")
}

impl Inner {
    /// Whether message id may be found, and since when.
    pub(crate) fn searchable(&self, id: &str) -> Option<GoTime> {
        let m = self.msgs.get(id)?;
        let at = self.visible_at.get(id)?;
        if !self.held_first.contains(id) || tm::before(at, &m.sent_at) {
            return Some(m.sent_at);
        }
        Some(*at)
    }

    fn deliveries_of(&self, id: &str) -> Vec<&Drec> {
        self.order
            .iter()
            .map(|did| &self.dels[did])
            .filter(|d| d.d.message_id == id)
            .collect()
    }

    fn name_of(&self, key: &str) -> String {
        if key == SYSTEM_KEY {
            return "Irori".into();
        }
        if key.is_empty() {
            return String::new();
        }
        if let Some(n) = self.known.get(key).filter(|n| !n.is_empty()) {
            return n.clone();
        }
        if let Some(a) = self.aliases.get(key).and_then(|a| a.last()) {
            return a.clone();
        }
        key.into()
    }

    fn recipient(&self, m: &Message, dels: &[&Drec]) -> String {
        match m.to.kind.as_str() {
            addr::REPO => return format!("repo {}", m.to.value),
            addr::PR => return m.to.value.clone(),
            _ => {}
        }
        if let Some(d) = dels.first()
            && !d.d.to.is_empty()
        {
            return self.name_of(&d.d.to);
        }
        m.to.value.clone()
    }

    /// The agents that may find or show m.
    pub(crate) fn readers(&self, m: &Message, dels: &[&Drec]) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let mut add = |k: &str| {
            if !k.is_empty() && !self.is_owner(k) && !out.iter().any(|o| o == k) {
                out.push(k.into());
            }
        };
        if !m.from.key.is_empty() && !self.is_person(&m.from) && !m.from.is_system() {
            add(&m.from.key);
        }
        for d in dels {
            if d.d.state != st::HELD && d.d.state != st::DISMISSED {
                add(&d.d.to);
            }
        }
        if out.is_empty() {
            out.push(SYSTEM_KEY.into()); // only the person reads it
        }
        out
    }

    fn doc(&self, m: &Message, updated: GoTime, dels: &[&Drec]) -> MsgDoc {
        let mut title = format!("{} -> {}", m.from.name, self.recipient(m, dels));
        if !m.reason.is_empty() {
            title.push_str(": ");
            title.push_str(&m.reason);
        }
        let agent = if !m.from.key.is_empty() && !self.is_person(&m.from) && !m.from.is_system() {
            m.from.key.clone()
        } else if let Some(d) = dels.first()
            && !self.is_owner(&d.d.to)
        {
            d.d.to.clone()
        } else {
            String::new()
        };
        let repo_key = match m.to.kind.as_str() {
            addr::REPO => m.to.value.clone(),
            addr::PR => ledger::parse_target(&m.to.value)
                .map(|r| ledger::repo_key(&r.repo))
                .unwrap_or_default(),
            _ => String::new(),
        };
        MsgDoc {
            ref_: msg_ref(&m.id),
            at: m.sent_at,
            updated,
            title: one_line(&title),
            body: m.text.clone(),
            agent,
            repo_key,
            readers: self.readers(m, dels),
        }
    }

    fn render(&self, m: &Message, title: &str, dels: &[&Drec]) -> String {
        let mut sb = String::new();
        let _ = writeln!(sb, "{title}");
        let _ = writeln!(
            sb,
            "from: {} ({}) trust={}",
            m.from.name, m.from.key, m.trust
        );
        let _ = writeln!(sb, "to: {} {}", m.to.kind, m.to.value);
        let _ = writeln!(sb, "sent: {}", m.sent_at.utc().format_rfc3339());
        if !m.reason.is_empty() {
            let _ = writeln!(sb, "reason: {}", one_line(&m.reason));
        }
        if !m.reply_to.is_empty() {
            let _ = writeln!(sb, "reply to: {}", msg_ref(&m.reply_to));
        }
        if !m.ref_.is_empty() {
            let _ = writeln!(sb, "about: {}", m.ref_);
        }
        if m.notice {
            sb.push_str("notice: from the Irori Board, needs no reply\n");
        }
        sb.push_str("deliveries:\n");
        for d in dels {
            let who = if d.d.to.is_empty() {
                "nobody yet".to_string()
            } else {
                format!("{} ({})", self.name_of(&d.d.to), d.d.to)
            };
            let _ = write!(sb, "  {} to {who}: {}", d.d.id, d.d.state);
            if !d.d.hold.is_empty() {
                let _ = write!(sb, " ({})", d.d.hold);
            }
            if d.d.mode == mode::NEXT_TURN {
                sb.push_str(", next turn");
            }
            if !d.d.note.is_empty() {
                let _ = write!(sb, ", note: {}", one_line(&d.d.note));
            }
            sb.push('\n');
        }
        sb.push('\n');
        sb.push_str(&m.text);
        sb.push('\n');
        sb
    }

    fn related(&self, m: &Message, viewer: &str) -> Vec<String> {
        let readable = |id: &str| {
            self.msgs
                .get(id)
                .is_some_and(|r| can_read(viewer, &self.readers(r, &self.deliveries_of(id))))
        };
        let mut out = vec![];
        if !m.reply_to.is_empty() && self.searchable(&m.reply_to).is_some() && readable(&m.reply_to)
        {
            out.push(msg_ref(&m.reply_to));
        }
        if !m.ref_.is_empty() {
            out.push(m.ref_.clone());
        }
        let mut replies: Vec<&String> = self
            .msgs
            .iter()
            .filter(|(id, r)| r.reply_to == m.id && self.searchable(id).is_some() && readable(id))
            .map(|(id, _)| id)
            .collect();
        replies.sort();
        out.extend(replies.into_iter().map(|id| msg_ref(id)));
        out.truncate(MAX_RELATED);
        out
    }
}

impl Inbox {
    /// The searchable messages updated at or after since, oldest first.
    pub fn search_docs(&self, since: &GoTime, limit: usize) -> Vec<MsgDoc> {
        let limit = if limit == 0 { MAX_SEARCH_DOCS } else { limit };
        let mut out: Vec<MsgDoc> = self.read(|b| {
            b.msgs
                .iter()
                .filter_map(|(id, m)| {
                    let up = b.searchable(id)?;
                    if tm::before(&up, since) {
                        return None;
                    }
                    Some(b.doc(m, up, &b.deliveries_of(id)))
                })
                .collect()
        });
        out.sort_by(|p, q| tm::cmp(&p.updated, &q.updated).then_with(|| p.ref_.cmp(&q.ref_)));
        out.truncate(limit);
        out
    }

    /// One message's full rendering for viewer (`""` = the person), or
    /// `None` when it is not searchable or not theirs to read.
    pub fn search_show(&self, id: &str, viewer: &str) -> Option<MsgShown> {
        self.read(|b| {
            let m = b.msgs.get(id)?;
            let up = b.searchable(id)?;
            let dels = b.deliveries_of(id);
            let doc = b.doc(m, up, &dels);
            if !can_read(viewer, &doc.readers) {
                return None;
            }
            Some(MsgShown {
                ref_: doc.ref_.clone(),
                title: doc.title.clone(),
                text: b.render(m, &doc.title, &dels),
                related: b.related(m, viewer),
            })
        })
    }
}

impl search::Source for Inbox {
    fn search_kinds(&self) -> Vec<search::Kind> {
        vec![search::Kind::Msg]
    }

    fn search_docs(&self, since: GoTime, limit: usize) -> Result<Vec<search::Doc>, search::Error> {
        Ok(Inbox::search_docs(self, &since, limit)
            .into_iter()
            .map(|d| search::Doc {
                r#ref: d.ref_,
                kind: search::Kind::Msg,
                at: d.at,
                updated: d.updated,
                title: d.title,
                body: d.body,
                agent: d.agent,
                repo_key: d.repo_key,
                readers: d.readers,
            })
            .collect())
    }

    fn search_show(
        &self,
        r: &str,
        o: &search::ShowOptions,
    ) -> Result<search::Shown, search::Error> {
        let (kind, id) = search::parse_ref(r)?;
        if kind != search::Kind::Msg {
            return Err(search::Error::NotFoundRef(r.to_string()));
        }
        // Not the viewer's message: as far as they can tell, it isn't there.
        let sh = Inbox::search_show(self, &id, &o.viewer)
            .ok_or_else(|| search::Error::NotFoundRef(r.to_string()))?;
        let (text, more, next) = search::page(&sh.text, o);
        Ok(search::Shown {
            r#ref: sh.ref_,
            kind: search::Kind::Msg,
            title: sh.title,
            text: text.to_string(),
            more,
            next,
            related: sh.related,
        })
    }
}
