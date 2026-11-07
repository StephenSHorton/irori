//! Talk (P5): the Rune port of the React app's components/talk/
//! talk-page.tsx, conversation.tsx, threads.tsx and replying-row.tsx, with
//! lib/conversations.ts, talk-sections.ts and talk.ts's helpers in
//! [`logic`].
//!
//! One list beside the open conversation (card #390): Threads (the
//! person's asks to the room), Channels, Direct messages, Agents talking
//! (agent↔agent DMs, a broadcast as one row) and the Archive (DMs whose
//! agents have gone), each folding and remembered per device; resolved
//! threads fold at its foot. The pane shows the open channel or DM
//! ([`conv`]), the open thread ([`thread`]), or "Start a thread".
//!
//! Live: talk:changed re-reads the list and the open conversation's
//! posts, talk:replying carries who is replying, board:changed (and
//! inbox:changed) re-read the threads and the open thread's card.

pub mod conv;
pub mod icons;
pub mod list;
// The pure helpers are also the inbox's and the sidebar's API (talk.ts's
// name_lookup and counts_by_agent, the status words): parts stay unused here.
#[allow(dead_code)]
pub mod logic;
pub mod thread;

use std::collections::{HashMap, HashSet};

use irori_window::cards::CardDetail;
use irori_window::events::{
    EVENT_BOARD_CHANGED, EVENT_INBOX_CHANGED, EVENT_ROSTER_CHANGED, EVENT_TALK_CHANGED, EVENT_TALK_REPLYING,
};
use irori_window::talk::{TalkConversation, TalkPerson, TalkPost, TalkReplying};
use irori_window::threads::ThreadSummary;
use rune::{El, PointerPress, TextEdit, col, row};

use crate::text::Composer;
use crate::ui::{Event, Frame, Route, State};
use logic::SectionKey;

/// What the list's + opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Adding {
    #[default]
    None,
    Channel,
    Dm,
}

/// The open channel's or DM's posts.
#[derive(Default)]
pub struct Posts {
    pub conv: String,
    pub list: Vec<TalkPost>,
    /// The first read landed (a skeleton till then, never on a reload).
    pub loaded: bool,
    pub error: Option<String>,
    /// Bumped on each read: the posts' memo version.
    pub rev: u64,
}

/// Make a card from a post (design §4): the form, filled from it.
pub struct Making {
    pub post_id: String,
    pub where_: String,
    pub board: String,
    pub title: TextEdit,
    pub objective: TextEdit,
    pub busy: bool,
    pub error: String,
}

/// Start a thread on a channel post (design b2).
#[derive(Default)]
pub struct Asking {
    pub post_id: String,
    pub author: String,
    pub edit: TextEdit,
    pub busy: bool,
    pub error: String,
}

#[derive(Default)]
pub struct Model {
    pub convs: Vec<TalkConversation>,
    pub convs_loaded: bool,
    pub convs_error: Option<String>,
    pub threads: Vec<ThreadSummary>,
    pub threads_loaded: bool,
    pub replying: Vec<TalkReplying>,
    pub posts: Posts,
    /// The open thread's card (its answers), and whose it is.
    pub thread: Option<CardDetail>,
    pub thread_id: String,
    pub thread_error: Option<String>,

    // The list.
    pub query: TextEdit,
    /// Each section's fold, as the person set it ("talk.folds").
    pub folds: HashMap<String, bool>,
    folds_read: bool,
    /// Sections whose "Show N more" was pressed.
    pub all_shown: HashSet<SectionKey>,
    /// Broadcast rows unfolded (or folded) by the person.
    pub unfolded: HashMap<String, bool>,
    pub show_resolved: bool,
    pub adding: Adding,
    pub new_channel: TextEdit,
    pub new_error: String,
    pub people: Vec<TalkPerson>,
    pub marking: bool,
    /// The open DM that stays with yours while open (card #473): (open,
    /// keep), keep None until the list has it.
    pub kept: (String, Option<String>),

    // A channel or DM.
    pub post_box: Composer,
    pub post_error: String,
    /// The person flipped "Agents answer now" on the channel's box.
    pub agents_now: Option<bool>,
    /// The members column, as the person toggled it (None: by width).
    pub members: Option<bool>,
    pub leaving: bool,
    pub leave_error: String,
    pub asking: Option<Asking>,
    /// A post's right-click menu: the post and where.
    pub menu: Option<(String, PointerPress)>,
    pub making: Option<Making>,
    /// The newest post this window marked read.
    pub read_to: String,

    // A thread (its reply box is the card package's).
    pub thread_busy: bool,
    pub thread_act_error: String,

    // Start a thread.
    pub ask_box: Composer,
    pub ask_now: Option<bool>,
    pub ask_error: String,
}

pub(crate) fn post_box(s: &State) -> &Composer {
    &s.talk.post_box
}
pub(crate) fn post_box_mut(s: &mut State) -> &mut Composer {
    &mut s.talk.post_box
}
pub(crate) fn ask_box(s: &State) -> &Composer {
    &s.talk.ask_box
}
pub(crate) fn ask_box_mut(s: &mut State) -> &mut Composer {
    &mut s.talk.ask_box
}

const FOLDS: &str = "talk.folds";

/// The conversation open ("" none).
pub fn open_id(s: &State) -> &str {
    match &s.nav.route {
        Route::Talk(Some(id)) => id,
        _ => "",
    }
}

/// The space the list shows. The window has no space switcher yet, so
/// every space's (logic::spaces::ALL_SPACES).
pub fn picked_space(_s: &State) -> &'static str {
    crate::logic::spaces::ALL_SPACES
}

/// Whether section `k` is folded (the person's pick, else its default).
pub fn folded(s: &State, k: SectionKey) -> bool {
    s.talk.folds.get(k.name()).copied().unwrap_or_else(|| k.folded_by_default())
}

pub fn toggle_fold(s: &mut State, k: SectionKey) {
    let now = !folded(s, k);
    s.talk.folds.insert(k.name().to_string(), now);
    let folds = s.talk.folds.clone();
    s.prefs.set(FOLDS, folds);
}

/// Opens conversation `id` ("" for Start a thread).
pub fn open(s: &mut State, id: &str) {
    s.close_card();
    s.go(Route::Talk((!id.is_empty()).then(|| id.to_string())));
    sync(s);
    list::keep_open(s);
}

/// Reads what the open conversation needs when it isn't what's loaded:
/// its posts and box (a channel or DM), or its card (a thread).
pub fn sync(s: &mut State) {
    if !s.talk.folds_read {
        s.talk.folds_read = true;
        s.talk.folds = s.prefs.get(FOLDS).unwrap_or_default();
    }
    let id = open_id(s).to_string();
    if id.is_empty() {
        if s.talk.ask_box.key.is_empty() {
            crate::text::open_draft(s, ask_box_mut, "talk/ask");
        }
        return;
    }
    let is_thread = s.talk.threads.iter().any(|t| t.id == id);
    let is_conv = s.talk.convs.iter().any(|c| c.conv_id == id);
    if is_thread && s.talk.thread_id != id {
        s.talk.thread_id = id.clone();
        s.talk.thread = None;
        s.talk.thread_error = None;
        s.talk.thread_act_error.clear();
        crate::card::open_reply(s, &id);
        load_thread(s);
    } else if is_conv && s.talk.posts.conv != id {
        s.talk.posts = Posts {
            conv: id.clone(),
            ..Posts::default()
        };
        s.talk.post_error.clear();
        s.talk.agents_now = None;
        s.talk.members = None;
        s.talk.leaving = false;
        s.talk.leave_error.clear();
        s.talk.asking = None;
        s.talk.menu = None;
        // An ended huddle's transcript belongs to the conversation it was
        // opened on.
        crate::call::close_transcript(s);
        crate::text::open_draft(s, post_box_mut, &format!("talk/{id}"));
        load_posts(s);
    }
}

fn load_convs(s: &mut State) {
    s.spawn(
        |w| w.talk_conversations(),
        |s, r| {
            match r {
                Ok(cs) => {
                    s.talk.convs = cs;
                    s.talk.convs_error = None;
                }
                Err(e) => s.talk.convs_error = Some(e),
            }
            s.talk.convs_loaded = true;
            sync(s);
            list::keep_open(s);
            mark_read(s);
        },
    );
}

fn load_threads(s: &mut State) {
    s.spawn(
        |w| w.threads_all(),
        |s, ts| {
            s.talk.threads = ts;
            s.talk.threads_loaded = true;
            sync(s);
        },
    );
}

fn load_replying(s: &mut State) {
    s.spawn(|w| w.talk_replying(), |s, r| s.talk.replying = r);
}

pub(crate) fn load_posts(s: &mut State) {
    let conv = s.talk.posts.conv.clone();
    if conv.is_empty() {
        return;
    }
    let ask = conv.clone();
    s.spawn(
        move |w| w.talk_posts(&ask, None, logic::POSTS_PAGE),
        move |s, r| {
            if s.talk.posts.conv != conv {
                return;
            }
            match r {
                Ok(ps) => {
                    s.talk.posts.list = ps;
                    s.talk.posts.error = None;
                }
                Err(e) => s.talk.posts.error = Some(e),
            }
            s.talk.posts.loaded = true;
            s.talk.posts.rev += 1;
            mark_read(s);
        },
    );
}

/// Opening a conversation reads it: the cursor moves to the newest post.
fn mark_read(s: &mut State) {
    let conv = s.talk.posts.conv.clone();
    if conv.is_empty() || open_id(s) != conv {
        return;
    }
    let unread = s.talk.convs.iter().find(|c| c.conv_id == conv).map_or(0, |c| c.unread);
    let Some(last) = s.talk.posts.list.last().map(|p| p.post_id.clone()) else { return };
    if unread == 0 || s.talk.read_to == last {
        return;
    }
    s.talk.read_to = last.clone();
    s.spawn(move |w| w.talk_read(&conv, &last), |_, _| {});
}

pub(crate) fn load_thread(s: &mut State) {
    let id = s.talk.thread_id.clone();
    if id.is_empty() {
        return;
    }
    let ask = id.clone();
    s.spawn(
        move |w| w.card(&ask),
        move |s, r| {
            if s.talk.thread_id != id {
                return;
            }
            match r {
                Ok(d) => {
                    s.talk.thread = Some(d);
                    s.talk.thread_error = None;
                }
                Err(e) => s.talk.thread_error = Some(e),
            }
        },
    );
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" => {
            load_convs(s);
            load_threads(s);
            load_replying(s);
        }
        EVENT_TALK_CHANGED => {
            load_convs(s);
            load_posts(s);
        }
        // Its payload is every conversation's list; the backend's type
        // reads only one way, so read it again (cheap, in memory).
        EVENT_TALK_REPLYING => load_replying(s),
        EVENT_BOARD_CHANGED | EVENT_INBOX_CHANGED => {
            load_threads(s);
            load_thread(s);
        }
        EVENT_ROSTER_CHANGED => load_thread(s),
        _ => {}
    }
}

/// Before each frame (ui::poll): a conversation opened from elsewhere
/// (the sidebar, a notification, a #ref) loads here.
pub fn poll(s: &mut State) -> bool {
    if !matches!(s.nav.route, Route::Talk(_)) || s.nav.card.is_some() {
        return false;
    }
    let id = open_id(s).to_string();
    let stale = if id.is_empty() {
        s.talk.ask_box.key.is_empty()
    } else {
        (s.talk.threads.iter().any(|t| t.id == id) && s.talk.thread_id != id)
            || (s.talk.convs.iter().any(|c| c.conv_id == id) && s.talk.posts.conv != id)
    };
    if stale {
        sync(s);
    }
    stale
}

/// How many conversations wait on the person: answered threads and those
/// no agent can start for, DMs with something new, channels that mention
/// them (the sidebar's Talk count).
pub fn waiting_count(s: &State) -> usize {
    let live = crate::logic::thread::live_roster(&s.data.agents);
    s.talk
        .threads
        .iter()
        .filter(|t| logic::needs_you(&t.state, &t.no_agent))
        .count()
        + logic::talk_waiting(&s.talk.convs, &live)
}

/// The headless scene: the busiest conversation open. `IRORI_TALK_SCENE`
/// picks another pane: `channel` (the newest channel), `thread` (the newest open thread), `dm` (the
/// newest DM), `start` (Start a thread), `search` (the list searched),
/// `make` (Make a card on the busiest conversation's newest post).
pub fn scene(s: &mut State) {
    s.close_card();
    // Headless, the loads run inline.
    load_convs(s);
    load_threads(s);
    load_replying(s);
    let pick = std::env::var("IRORI_TALK_SCENE").unwrap_or_default();
    let id = match pick.as_str() {
        "start" | "search" => String::new(),
        "thread" => s
            .talk
            .threads
            .iter()
            .filter(|t| t.state != irori_window::threads::THREAD_RESOLVED)
            .max_by_key(|t| t.replies)
            .or_else(|| s.talk.threads.first())
            .map(|t| t.id.clone())
            .unwrap_or_default(),
        "channel" => s
            .talk
            .convs
            .iter()
            .filter(|c| c.kind == "channel")
            .max_by_key(|c| logic::at_ms(&c.last_at))
            .map(|c| c.conv_id.clone())
            .unwrap_or_default(),
        "dm" => s
            .talk
            .convs
            .iter()
            .filter(|c| c.kind == "dm" && !c.read_only)
            .max_by_key(|c| logic::at_ms(&c.last_at))
            .map(|c| c.conv_id.clone())
            .unwrap_or_default(),
        _ => busiest(s),
    };
    if pick == "search" {
        s.talk.query.replace_text("re");
    }
    open(s, &id);
    if pick == "make"
        && let Some(p) = s.talk.posts.list.iter().rev().find(|p| p.call.is_none() && p.thread_from.is_none())
    {
        let pid = p.post_id.clone();
        conv::start_making(s, &pid);
    }
}

/// The channel or DM with the most posts (of the newest few), read only
/// for the scene.
fn busiest(s: &State) -> String {
    let w = s.win();
    let mut cands: Vec<&TalkConversation> =
        s.talk.convs.iter().filter(|c| c.kind != "thread" && !c.read_only).collect();
    cands.sort_by_key(|c| std::cmp::Reverse(logic::at_ms(&c.last_at)));
    cands
        .iter()
        .take(25)
        .map(|c| (w.talk_posts(&c.conv_id, None, logic::POSTS_PAGE).map_or(0, |p| p.len()), c.kind == "channel", &c.conv_id))
        .max_by_key(|(n, ch, _)| (*n, *ch))
        .map(|(_, _, id)| (*id).clone())
        .unwrap_or_default()
}

/// The page: the list beside the open conversation.
pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let id = open_id(s).to_string();
    let list = list::view(s, f, &id);
    let pane = if let Some(t) = s.talk.threads.iter().find(|t| t.id == id) {
        thread::view(s, f, t)
    } else if let Some(c) = s.talk.convs.iter().find(|c| c.conv_id == id) {
        conv::view(s, f, c)
    } else if !id.is_empty() && (!s.talk.threads_loaded || !s.talk.convs_loaded) {
        // A link straight to a conversation waits for the lists' first
        // read: a skeleton of the pane till then (card #598).
        col()
            .key("talk-finding")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .padding(16.0)
            .child(crate::widgets::loading::message_rows_shape(f, "talk-finding-rows", 4))
    } else {
        thread::start_view(s, f)
    };
    let page = row()
        .key("talk-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(k.c.bg_base)
        .child(list)
        .child(pane.grow(1.0).min_w(0.0).h_full().min_h(0.0));
    conv::overlays(s, f);
    page
}
