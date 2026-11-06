//! Agents mentioning agents (card #77; board.md "Agents mention too"): the
//! `mention` arg and `@name` in text, resolved against every device's
//! roster, and the "Mentioned: … Not delivered: …" line.

use coord::mentions::{MISS_AMBIGUOUS, MISS_CAP, Miss, Resolved};
use coord::{Mention, NOTICE_MENTIONED, Notice};
use inbox::notice::NoticeResult;

use crate::args::Args;
use crate::format::one_line;
use crate::{Caller, Inner, Names};

/// What one mention came to, for the result line.
pub(crate) enum Got {
    Woke,
    Held(String),
    Failed(String),
    Gone,
    /// Another person or their agent on a shared board (card #186): the
    /// mention is on the card, and their own devices tell them.
    Board,
    /// The same in a space's channel (card #461): the post names them, and
    /// their own devices wake them.
    Space,
}

/// The roster as coord's resolver reads it.
fn cards(s: &Inner) -> Vec<coord::roster::AgentCard> {
    s.b.agents()
        .into_iter()
        .map(|a| coord::roster::AgentCard {
            key: a.key,
            device_id: a.device_id,
            name: a.name,
            cwd: a.cwd,
            status: a.status,
            repo_key: a.repo_key,
            ..Default::default()
        })
        .collect()
}

impl Inner {
    /// The caller's mentions in `text` and `given`, writing on card `id`
    /// ("" for none). On a shared board's card they may also name the
    /// board's other people and their agents (card #186).
    pub(crate) fn resolve_mentions(
        &self,
        c: &Caller,
        id: &str,
        text: &str,
        given: &[String],
    ) -> Resolved {
        let mut agents = cards(self);
        if !id.is_empty()
            && let Some(sb) = self.b.shared_boards()
            && let Some(b) = sb.board_of(id)
        {
            agents.extend(sb.board_mentionables(&b.board_id));
        }
        coord::mentions::resolve(&agents, &c.sender.key, text, given)
    }

    /// `ms` with each key as card `id`'s log knows it. A key the log
    /// can't map (its agent left) comes back gone.
    pub(crate) fn log_mentions(&self, id: &str, ms: &[Mention]) -> Vec<Mention> {
        ms.iter()
            .map(|m| match self.on(id, std::slice::from_ref(&m.key)) {
                Ok((_, ks)) => Mention {
                    key: ks.into_iter().next().unwrap_or_default(),
                    ..m.clone()
                },
                Err(_) => Mention {
                    gone: true,
                    ..m.clone()
                },
            })
            .collect()
    }

    /// `ms` for card `id`: as its log writes them, and as this device
    /// knows them with the log's gone flags (for the result line).
    pub(crate) fn mentions_on(&self, id: &str, ms: &[Mention]) -> (Vec<Mention>, Vec<Mention>) {
        let logm = self.log_mentions(id, ms);
        let shown = ms
            .iter()
            .zip(&logm)
            .map(|(p, l)| Mention {
                gone: l.gone,
                ..p.clone()
            })
            .collect();
        (logm, shown)
    }

    /// Mentions on card `id` that nothing stores (a blocker's question,
    /// an unblock note): woken, made watchers; the result line.
    pub(crate) fn mention_only(&self, c: &Caller, id: &str, text: &str, a: &Args) -> String {
        self.mention_with(c, id, text, a, None).1
    }

    /// As [`Inner::mention_only`], telling `rich` too (the unblock notice):
    /// a mentioned agent it reaches hears it woken, once (coord's
    /// mention_into). Returns `rich`'s results and the result line.
    pub(crate) fn mention_with(
        &self,
        c: &Caller,
        id: &str,
        text: &str,
        a: &Args,
        rich: Option<Notice>,
    ) -> (Vec<NoticeResult>, String) {
        let res = self.resolve_mentions(c, id, text, &a.strs("mention"));
        let tell_rich = |n: Option<Notice>| n.map(|n| self.tell(&c.sender, n)).unwrap_or_default();
        if res.mentions.is_empty() && res.misses.is_empty() {
            return (tell_rich(rich), String::new());
        }
        let failed = |e: &dyn std::fmt::Display| {
            format!(
                "
Not delivered: the mentions ({}).",
                one_line(&e.to_string(), 160)
            )
        };
        let (logm, shown) = self.mentions_on(id, &res.mentions);
        let n = self.roster();
        let mut ns = match self.on(id, std::slice::from_ref(&c.sender.key)) {
            Ok((co, ks)) => match co.mention(&ks[0], id, &logm, text) {
                Ok(ns) => ns,
                Err(e) => return (tell_rich(rich), failed(&e)),
            },
            Err(e) => return (tell_rich(rich), failed(&e)),
        };
        let mut rich = rich;
        if let Some(r) = rich.as_mut() {
            coord::mention_into(r, &mut ns);
        }
        let told = tell_rich(rich);
        let mut mres = Vec::new();
        for x in ns {
            mres.extend(self.tell(&c.sender, x));
        }
        (
            told,
            mention_line(&outcomes(&shown, &mres), &res.misses, &n),
        )
    }

    /// Routes `ns` from the caller: the mention notice's results come
    /// back apart from the others'.
    pub(crate) fn tell_all(
        &self,
        c: &Caller,
        ns: Vec<Notice>,
    ) -> (Vec<NoticeResult>, Vec<NoticeResult>) {
        let (mut other, mut mentioned) = (Vec::new(), Vec::new());
        for n in ns {
            let m = n.kind == NOTICE_MENTIONED;
            let rs = self.tell(&c.sender, n);
            if m {
                mentioned.extend(rs);
            } else {
                other.extend(rs);
            }
        }
        (other, mentioned)
    }
}

/// Each of `ms` (private keys) as the mention notice's results left it.
pub(crate) fn outcomes(ms: &[Mention], rs: &[NoticeResult]) -> Vec<(Mention, Got)> {
    ms.iter()
        .map(|m| {
            if m.gone {
                return (m.clone(), Got::Gone);
            }
            // A board key is another person or their agent: this device's
            // own agents resolve to private keys (card #186).
            if coord::board_form(&m.key) {
                return (m.clone(), Got::Board);
            }
            let got = match rs.iter().find(|r| r.to == m.key) {
                None => Got::Failed("not sent".into()),
                Some(r) => match &r.err {
                    Some(e) => Got::Failed(e.to_string()),
                    None if r.delivery.state == inbox::st::HELD => {
                        Got::Held(r.delivery.hold.clone())
                    }
                    None => Got::Woke,
                },
            };
            (m.clone(), got)
        })
        .collect()
}

fn named(m: &Mention) -> String {
    let n = one_line(&m.name, 80);
    if n.is_empty() {
        one_line(&m.key, 120)
    } else {
        n
    }
}

fn miss_text(m: &Miss, n: &Names<'_>) -> String {
    let name = one_line(&m.name, 80);
    if m.reason == MISS_AMBIGUOUS {
        let keys: Vec<String> = m
            .candidates
            .iter()
            .map(|k| {
                let cwd = n.cards.get(k).map(|c| c.cwd.clone()).unwrap_or_default();
                if cwd.is_empty() {
                    one_line(k, 120)
                } else {
                    format!("{}, {}", one_line(k, 120), one_line(&cwd, 120))
                }
            })
            .collect();
        return format!(
            "{name} is ambiguous: mention one by key ({})",
            keys.join("; ")
        );
    }
    if m.reason == MISS_CAP {
        return format!("{name}: at most {} mentions per write", coord::MAX_MENTIONS);
    }
    if m.is_key() {
        return format!("no agent has key {name}");
    }
    format!("\"@{name}\" names no live agent")
}

/// The line a tool's result gains when it had mentions ("" when none).
pub(crate) fn mention_line(got: &[(Mention, Got)], misses: &[Miss], n: &Names<'_>) -> String {
    if got.is_empty() && misses.is_empty() {
        return String::new();
    }
    let (mut ok, mut not) = (Vec::new(), Vec::new());
    for (m, g) in got {
        let who = named(m);
        match g {
            Got::Woke => ok.push(format!("{who} (woke)")),
            Got::Held(h) => ok.push(format!("{who} (held by {})", one_line(h, 80))),
            Got::Failed(e) => not.push(format!("{who}: not sent: {}", one_line(e, 160))),
            Got::Gone => not.push(format!("{who} is gone")),
            Got::Board => ok.push(format!("{who} (in the space)")),
            Got::Space => ok.push(format!("{who} (in the space)")),
        }
    }
    not.extend(misses.iter().map(|m| miss_text(m, n)));
    let mut out = String::from("\n");
    if !ok.is_empty() {
        out += &format!("Mentioned: {}.", ok.join(", "));
    }
    if !not.is_empty() {
        if !ok.is_empty() {
            out += " ";
        }
        out += &format!("Not delivered: {}.", not.join("; "));
    }
    out
}

impl Inner {
    /// Sends `m`'s mentioned agents their copies; the result line.
    pub(crate) fn message_mentions(
        &self,
        c: &Caller,
        m: &inbox::Message,
        d: &inbox::Delivery,
        res: &Resolved,
    ) -> String {
        if res.mentions.is_empty() && res.misses.is_empty() {
            return String::new();
        }
        let n = self.roster();
        let to_name = n
            .cards
            .get(&d.to)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let rs = self
            .b
            .inbox()
            .send_mention_copies(&c.sender, m, &d.to, &to_name);
        mention_line(&copy_outcomes(&res.mentions, &rs, &d.to), &res.misses, &n)
    }

    /// `m`'s live mentions watch the card its Ref names, on whichever
    /// log holds it (a shared board's too); "" or a line saying they
    /// couldn't.
    pub(crate) fn watch_mentioned(&self, c: &Caller, m: &inbox::Message) -> String {
        let Some(tid) = crate::notice::card_for(self.b.coord(), &m.ref_) else {
            return String::new();
        };
        let mut keys = vec![c.sender.key.clone()];
        keys.extend(m.mentions.iter().filter(|x| !x.gone).map(|x| x.key.clone()));
        if keys.len() == 1 {
            return String::new();
        }
        let res = self
            .on(&tid, &keys)
            .and_then(|(co, ks)| co.watch_for(&ks[0], &tid, &ks[1..]));
        match res {
            Ok(_) => String::new(),
            Err(e) => format!(
                "
The mentioned agents don't watch the card: {}.",
                one_line(&e.to_string(), 160)
            ),
        }
    }
}

/// `coord::Mention`s as the inbox stores them.
pub(crate) fn to_inbox(ms: &[Mention]) -> Vec<inbox::Mention> {
    ms.iter()
        .map(|m| inbox::Mention {
            key: m.key.clone(),
            name: m.name.clone(),
            gone: m.gone,
        })
        .collect()
}

/// What a message's copies came to.
pub(crate) fn copy_outcomes(
    ms: &[Mention],
    rs: &[(inbox::Mention, inbox::Result<inbox::Delivery>)],
    skip: &str,
) -> Vec<(Mention, Got)> {
    ms.iter()
        .filter(|m| m.key != skip)
        .map(|m| {
            if m.gone {
                return (m.clone(), Got::Gone);
            }
            let got = match rs.iter().find(|(x, _)| x.key == m.key) {
                None => Got::Failed("not sent".into()),
                Some((_, Err(e))) => Got::Failed(e.to_string()),
                Some((_, Ok(d))) if d.state == inbox::st::HELD => Got::Held(d.hold.clone()),
                Some(_) => Got::Woke,
            };
            (m.clone(), got)
        })
        .collect()
}
