//! `GET /v1/attach` (I:internal/api/stream.go; surface.md §2.6, §7.12): the
//! NDJSON wake stream of a mod session (`?session=&mod=`) or a Grok member
//! (`?member=`).
//!
//! Kept exact: subscribe to the inbox before registering; one registry
//! mutex covers supersede, reachability and wake issuing; a superseded
//! stream sends `superseded` before it closes; member streams count opens
//! per stream; the keepalive ticks on a fixed cadence and catches up on
//! wakes, appends and the session name.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use inbox::{ChangeKind, Inbox, Sender, Wake};

use crate::COUNT_LIMIT;
use crate::attach::{self, Api, Event};
use crate::http::{self, Resp, Sink};
use crate::server::{Handler, POLL, Shared, Stream, attach_secret};

/// The live card's name event for Claude session `key` (card #21); `None`
/// when there is none.
fn name_event(s: &Shared, key: &str) -> Option<Event> {
    s.b.agents()
        .into_iter()
        .find(|c| c.key == key && c.agent == "claude" && c.status != "gone")
        .map(|c| Event {
            name: c.name,
            name_source: c.name_source,
            ai_title: c.ai_title,
            ..Event::of(attach::TYPE_NAME)
        })
}

pub(crate) fn attach(h: &Handler) -> Resp {
    let q = &h.r.query;
    let s = h.s;
    let mut hello = Event {
        v: attach::VERSION,
        api: Some(Api {
            url: format!("http://127.0.0.1:{}", h.port),
            token: s.token_str(),
        }),
        ..Event::of(attach::TYPE_HELLO)
    };
    let (session, member) = (q.get("session"), q.get("member"));
    let st = if !session.is_empty() && member.is_empty() {
        if session.len() > 200
            || session
                .chars()
                .any(|r| r == '/' || gocompat::text::is_control(r))
        {
            return http::error(400, "bad session id");
        }
        let key = format!("{}/{session}", s.b.device().id);
        let Some(named) = name_event(s, &key) else {
            return http::error(
                404,
                &format!(
                    "no live session {} on this device (yet)",
                    gocompat::quote::quote(&session)
                ),
            );
        };
        let secret = attach_secret();
        hello.session = session.clone();
        hello.agent_key = key.clone();
        hello.name = named.name.clone();
        hello.attach_secret = secret.clone();
        hello.name_source = named.name_source.clone();
        hello.ai_title = named.ai_title.clone();
        Stream {
            id: s.streams.next_id(),
            sender: Sender::new(&key, &named.name, "claude"),
            key,
            path: inbox::path::MOD.into(),
            secret,
            kicked: Default::default(),
            appends: attach::mod_at_least(&q.get("mod"), attach::APPEND_SINCE),
            named: Mutex::new(named),
        }
    } else if !member.is_empty() && session.is_empty() {
        let Some(m) = s.b.member_by_secret(&member) else {
            return http::error(403, "unknown member secret");
        };
        hello.agent_key = m.key.clone();
        hello.name = m.name.clone();
        if let Some(i) = m.key.rfind('/') {
            hello.session = m.key[i + 1..].to_string();
        }
        Stream {
            id: s.streams.next_id(),
            sender: Sender::new(&m.key, &m.name, "grok"),
            key: m.key,
            path: inbox::path::CHANNEL.into(),
            secret: String::new(),
            kicked: Default::default(),
            appends: false,
            named: Mutex::new(Event::default()),
        }
    } else {
        return http::error(400, "pass exactly one of session or member");
    };
    let Some(ib) = s.b.inbox() else {
        return http::error(503, "messaging is not running");
    };
    let st = Arc::new(st);
    let shared = h.s.clone();
    Resp::Stream {
        run: Box::new(move |out: Sink| run(&shared, &ib, st, hello, out)),
    }
}

fn run(s: &Arc<Shared>, ib: &Arc<Inbox>, st: Arc<Stream>, hello: Event, out: Sink) {
    let changes = ib.subscribe(); // before anything can make a wake ready
    // Register (superseding an older stream) and mark reachable in one step.
    {
        let mut reg = s.streams.lock();
        if let Some(old) = reg.by_key.remove(&st.key) {
            old.kicked.store(true, Ordering::SeqCst);
            reg.secrets.remove(&old.secret);
        }
        reg.by_key.insert(st.key.clone(), st.clone());
        if !st.secret.is_empty() {
            reg.secrets.insert(st.secret.clone(), st.clone());
        }
        if st.path == inbox::path::CHANNEL {
            s.b.member_stream(&st.key, true);
        }
        ib.set_reachable(&st.key, &st.path, true);
    }
    let _ = serve(s, ib, &st, hello, &out, &changes);
    {
        let mut reg = s.streams.lock();
        if reg.current(&st) {
            reg.by_key.remove(&st.key);
            reg.secrets.remove(&st.secret);
            ib.set_reachable(&st.key, &st.path, false);
        }
        // Every member stream opened one count, superseded or not, so
        // every one closes it.
        if st.path == inbox::path::CHANNEL {
            s.b.member_stream(&st.key, false);
        }
    }
    changes.cancel();
    if st.kicked.load(Ordering::SeqCst) {
        let _ = out.line(&Event::of(attach::TYPE_SUPERSEDED));
    }
}

/// The stream until it ends (`Err`: a write failed, it was superseded or
/// the client left).
fn serve(
    s: &Shared,
    ib: &Inbox,
    st: &Stream,
    hello: Event,
    out: &Sink,
    changes: &inbox::Subscription,
) -> Result<(), ()> {
    out.line(&hello)?;
    // A wake issued to an earlier stream but never confirmed delivered goes
    // out again with the same token; a confirmed one is never re-sent.
    wake(s, ib, st, out, true)?;
    send_append(s, ib, st, out, true)?;
    let mut counts = counts_event(ib, &st.key);
    out.line(&counts)?;

    let every = s.keepalive();
    let mut tick = Instant::now() + every;
    loop {
        if st.kicked.load(Ordering::SeqCst) || out.done() {
            return Err(());
        }
        let wait = tick
            .saturating_duration_since(Instant::now())
            .min(POLL)
            .max(Duration::from_millis(1));
        match changes.queue().recv_timeout(wait) {
            Err(()) => {
                if Instant::now() < tick {
                    continue;
                }
                tick += every;
                if tick < Instant::now() {
                    tick = Instant::now() + every;
                }
                out.keepalive()?;
                // Catch up on a dropped notification (slow subscriber).
                wake(s, ib, st, out, false)?;
                send_append(s, ib, st, out, false)?;
                if st.path == inbox::path::MOD
                    && let Some(ev) = name_event(s, &st.key)
                {
                    let mut named = st.named.lock().unwrap_or_else(|e| e.into_inner());
                    if ev.name != named.name
                        || ev.name_source != named.name_source
                        || ev.ai_title != named.ai_title
                    {
                        *named = ev.clone();
                        drop(named);
                        out.line(&ev)?;
                    }
                }
            }
            Ok(None) => return Err(()),
            Ok(Some(ch)) => {
                if st.kicked.load(Ordering::SeqCst) {
                    return Err(());
                }
                if !ch.to.is_empty() && ch.to != st.key {
                    continue;
                }
                match ch.kind {
                    ChangeKind::HeldNew => {
                        if let Some(ev) = held_event(ib, &ch.delivery_id) {
                            out.line(&ev)?;
                        }
                    }
                    ChangeKind::MovedAway => {
                        if let Some(d) = ib.get(&ch.delivery_id) {
                            out.line(&Event {
                                delivery_id: d.id,
                                to: d.moved_to,
                                ..Event::of(attach::TYPE_MOVED)
                            })?;
                        }
                    }
                    _ => {}
                }
                // Every change for this agent (AppendReady among them)
                // looks for a wake and an append; both are none most of
                // the time.
                wake(s, ib, st, out, false)?;
                send_append(s, ib, st, out, false)?;
                let c = counts_event(ib, &st.key);
                if c.unread != counts.unread || c.held_n != counts.held_n {
                    out.line(&c)?;
                    counts = c;
                }
            }
        }
    }
}

/// Issues and sends the next wake, if any; `resend` first re-sends an
/// issued wake never confirmed delivered.
fn wake(s: &Shared, ib: &Inbox, st: &Stream, out: &Sink, resend: bool) -> Result<(), ()> {
    let w = {
        let reg = s.streams.lock();
        if !reg.current(st) {
            return Err(());
        }
        let mut w: Option<Wake> = None;
        if resend
            && let Some((ow, state)) = ib.outstanding_wake(&st.key)
            && state == inbox::st::QUEUED
        {
            w = Some(ow);
        }
        if w.is_none() {
            match ib.next_wake(&st.key, &st.path) {
                Ok(n) => w = n,
                Err(e) => {
                    crate::logf(format_args!("attach {}: next wake: {e}", st.key));
                    return Ok(());
                }
            }
        }
        w
    };
    match w {
        Some(w) => out.line(&deliver_event(ib, &w)),
        None => Ok(()),
    }
}

/// Issues and sends the next append (next-turn delivery) if `st` takes
/// them; `resend` first re-sends an issued, unconfirmed one.
fn send_append(s: &Shared, ib: &Inbox, st: &Stream, out: &Sink, resend: bool) -> Result<(), ()> {
    if !st.appends {
        return Ok(());
    }
    let w = {
        let reg = s.streams.lock();
        if !reg.current(st) {
            return Err(());
        }
        let mut w = if resend {
            ib.outstanding_append(&st.key)
        } else {
            None
        };
        if w.is_none() {
            match ib.next_append(&st.key) {
                Ok(n) => w = n,
                Err(e) => {
                    crate::logf(format_args!("attach {}: next append: {e}", st.key));
                    return Ok(());
                }
            }
        }
        w
    };
    let Some(w) = w else { return Ok(()) };
    let mut ev = deliver_event(ib, &w);
    ev.kind = attach::TYPE_APPEND.into();
    ev.mode = inbox::mode::NEXT_TURN.into();
    out.line(&ev)
}

/// `w` on the wire: a deliver (mode set only on a next_turn-only push), or
/// the body of an append.
fn deliver_event(ib: &Inbox, w: &Wake) -> Event {
    let mut ev = Event {
        wake_id: w.id.clone(),
        token: w.token.clone(),
        reminder: w.reminder,
        delivery_ids: w.delivery_ids.clone(),
        prompt: w.prompt.clone(),
        overflow: w.overflow.clone(),
        ..Event::of(attach::TYPE_DELIVER)
    };
    if w.mode == inbox::mode::NEXT_TURN {
        ev.mode = inbox::mode::NEXT_TURN.into();
    }
    for id in &w.delivery_ids {
        if let Some(d) = ib.get(id) {
            if ev.from.is_empty() {
                ev.from = d.message.from.name.clone();
            }
            ev.message_ids.push(d.message_id);
        }
    }
    ev
}

fn held_event(ib: &Inbox, id: &str) -> Option<Event> {
    let d = ib.get(id)?;
    if d.state != inbox::st::HELD {
        return None;
    }
    Some(Event {
        delivery_id: d.id,
        from: d.message.from.name.clone(),
        why: d.hold,
        preview: preview(&d.message.text),
        ..Event::of(attach::TYPE_HELD)
    })
}

/// The inbox event for `key`: open deliveries it can see, and those held
/// from it.
fn counts_event(ib: &Inbox, key: &str) -> Event {
    let held = ib
        .list(COUNT_LIMIT)
        .iter()
        .filter(|d| d.to == key && d.state == inbox::st::HELD)
        .count();
    Event {
        unread: ib.for_agent(key, false, COUNT_LIMIT).len() as i64,
        held_n: held as i64,
        ..Event::of(attach::TYPE_INBOX)
    }
}

/// Go `api.Preview`: `text`'s first 120 runes on one line.
pub fn preview(text: &str) -> String {
    let joined = text
        .split(|r: char| gocompat::text::is_space(r) || gocompat::text::is_control(r))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn preview_one_line() {
        assert_eq!(super::preview("  a\n\tb\u{85}c\u{1}d  "), "a b c d");
        let long: String = "é".repeat(130);
        assert_eq!(super::preview(&long).chars().count(), 120);
    }
}
