//! Text (F0b): the shared text pieces the card page, Talk, Home's ask box
//! and the message dialog use. The Rune port of the React app's
//! common/rich-text.tsx + lib/rich.ts, common/mention-textarea.tsx +
//! lib/mentions.ts, lib/drafts.ts, and attachments/* + lib/attachments.ts,
//! uploads.ts, clipboard.ts.
//!
//! # API (for the card page, Talk, Home and the message dialog)
//!
//! **Rich text** (agents' text: links, `code`, lists, mentions, card refs;
//! never markdown or HTML):
//! - [`rich_text`]`(s, f, key, &str) -> El`: paragraphs and lists,
//!   selectable; links open in the browser (`tx.open_url`), code copies on
//!   a click, "#12" chips open the card (`s.open_card`).
//! - [`rich_with`]`(s, f, key, &str, &RichOpts)`: with `mentions` (a
//!   comment's `Vec<CardMention>`: their "@name"s become chips), `font`,
//!   `color`, `max_lines`, `interactive: false` (inside a button).
//! - [`rich_line`] for one line; [`code`], [`copy_button`], [`link_chip`]
//!   (a blocker's or option's link: url/path/commit), [`ref_dot`].
//! - Card refs the window doesn't hold (done cards, other boards, old
//!   threads) are looked up off the UI thread when the text comes on screen
//!   ([`refs::prefetch_text`] does it ahead) and show as chips once found.
//! - Pure logic: [`rich::rich_blocks`], [`rich::rich_inline`],
//!   [`rich::safe_href`], [`rich::plain_line`], [`rich::plain_block`];
//!   [`refs::card_status`] / [`refs::ref_status`] (a card's chip status).
//!
//! **The composer** (a box with the @ and # pickers, files, mic, Send):
//! - Own a [`Composer`] in your Model (`pub ask: Composer`), and pass two
//!   fn pointers that reach it:
//!   ```ignore
//!   fn ask(s: &State) -> &Composer { &s.home.ask }
//!   fn ask_mut(s: &mut State) -> &mut Composer { &mut s.home.ask }
//!   let opts = ComposerOpts {
//!       placeholder: "Ask your agents…".into(),
//!       send_button: true,
//!       asking: true,
//!       mic: Some(crate::call::mic_button(s, f)),   // P6's, when it lands
//!       on_send: Some(Rc::new(|s: &mut State, m: Sent| {
//!           ask_mut(s).sending = true;
//!           let ms = text::mentions::to_ts(&m.mentions);
//!           s.act("Ask", move |w| w.start_thread(&m.text, &ms, &m.files, ""), |s, r| {
//!               text::sent(s, ask_mut);              // empties the box, drops its draft
//!               ask_mut(s).missed(&r.mentions);     // "Not delivered: …" if any
//!           });
//!       })),
//!       ..ComposerOpts::default()
//!   };
//!   text::composer(s, f, "home-ask", ask, ask_mut, &opts)
//!   ```
//!   (On a failed send, set `sending = false` again.)
//! - Enter sends, Shift+Enter is a new line; the pickers list
//!   `Window::mention_candidates` (kept on `s.text.cands`, fresh on roster
//!   changes) and `s.data.cards`. `ComposerOpts::card` puts "the agent on
//!   #N" for the box's card first; `board` is the shared board it is on;
//!   `no_mentions` / `no_files` turn those off with a reason.
//! - Drafts: [`open_draft`]`(s, ask_mut, "c1/comment")` when the page
//!   opens loads what was typed there before; the box then keeps it as the
//!   person types (in the prefs, `text.drafts`). [`drafts::draft`] and
//!   [`drafts::set_draft`] for a plain box.
//! - `Sent { text, mentions, files }`: mentions as [`mentions::Mention`]
//!   ([`mentions::to_ts`] for the backend), files as `UploadFile`s.
//!
//! **Attachments**:
//! - [`attachment_list`]`(s, f, key, &[File], compact)`: pictures as
//!   thumbnails (loaded as bytes through `Window::serve_attachment` when they
//!   come on screen, a big one on a click; full size in a dialog), the rest
//!   as chips with Save and Open. [`File::all`]`(&comment.attachments)`
//!   reads any of the backend's file refs.
//! - [`attach::file_kind_icon`], [`attach::human_size`],
//!   [`attach::looks_like_image`], [`attach::can_open`].
//!
//! The demo scene `irori --snap x.png --scene text` shows rich text samples,
//! the attachments found in the data, and a composer with its picker open.

// The API above is for the wave-2 pages (card, Talk, Home, the message
// dialog); until they use it, parts of it are used only by the demo.
#![allow(dead_code, unused_imports)]

pub mod attach;
pub mod composer;
pub mod drafts;
pub mod icons;
pub mod mentions;
pub mod refs;
pub mod render;
pub mod rich;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use irori_window::cards::{AgentRef, CardMention};
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_ROSTER_CHANGED};
use irori_window::threads::RefTarget;
use rune::{El, ScrollSpec, col, row, text};

pub use attach::{File, attachment_list};
pub use composer::{Composer, ComposerOpts, Get, GetMut, SendFn, Sent, composer, open_draft, sent};
pub use render::{RichOpts, code, copy_button, link_chip, ref_dot, rich_line, rich_text, rich_with};

use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {
    /// The mention picker's rows (`Window::mention_candidates`).
    pub cands: Vec<mentions::Candidate>,
    /// Card refs the backend looked up: "#12" or "mlg#12" to what it names
    /// (None: nothing, or more than one).
    pub refs: HashMap<String, Option<RefTarget>>,
    pub refs_pending: HashSet<String>,
    /// Bumped when the board changes: an older lookup's answer is stale.
    pub refs_rev: u64,
    /// Files loaded for showing, by sha256.
    pub files: HashMap<String, attach::Loaded>,
    /// Big pictures the person asked to preview.
    pub wanted: HashSet<String>,
    /// A Save or Open on its way, by sha256.
    pub busy: HashMap<String, &'static str>,
    /// What a Save or Open came to (and whether it failed), by sha256.
    pub notes: HashMap<String, (String, bool)>,
    /// The picture shown full size.
    pub full: Option<File>,
    pub drafts: drafts::Drafts,
    /// The demo page (scene "text") shows instead of the route's page.
    pub demo: bool,
    pub demo_box: Composer,
    demo_files: Vec<File>,
}

fn load_cands(s: &mut State) {
    s.spawn(
        |w| w.mention_candidates(),
        |s, cs| {
            if !cs.is_empty() || !s.text.demo {
                s.text.cands = cs.into_iter().map(mentions::Candidate::from).collect();
            }
        },
    );
}

/// The refs already looked up, asked again: a done card's state, a
/// renamed title.
fn refresh_refs(s: &mut State) {
    s.text.refs_rev += 1;
    let refs: Vec<String> = s.text.refs.keys().cloned().collect();
    if refs.is_empty() {
        return;
    }
    let rev = s.text.refs_rev;
    s.spawn(
        move |w| refs.into_iter().map(|r| (w.card_ref(&r), r)).collect::<Vec<_>>(),
        move |s, found| {
            if s.text.refs_rev == rev {
                for (t, r) in found {
                    s.text.refs.insert(r, t);
                }
            }
        },
    );
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" | EVENT_ROSTER_CHANGED | irori_window::boards::EVENT_BOARDS_CHANGED => load_cands(s),
        EVENT_BOARD_CHANGED => refresh_refs(s),
        _ => {}
    }
    drafts::flush(s);
}

/// What the text package draws over the page: a picture shown full size.
pub fn overlay(s: &State, f: &mut Frame) {
    attach::full_view(s, f);
}

fn demo_box(s: &State) -> &Composer {
    &s.text.demo_box
}
fn demo_box_mut(s: &mut State) -> &mut Composer {
    &mut s.text.demo_box
}

/// The demo page (scene "text"): rich text samples, attachments and a
/// composer, as the other pages use them.
pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let card = s.data.cards.iter().find(|c| !c.short.is_empty());
    let r = card.map(|c| format!("#{}", c.short)).unwrap_or_else(|| "#1".into());
    let sample = format!(
        "Two things before I go on: 1) open huggingface.co/facebook/dinov3 and click Request access 2) type `! hf auth login` in the Mac session.\n\n\
         I asked @oak to check {r}, and @cedar too. See https://github.com/acme/app/pull/57, then mlg#999 for the old one.\n\
         The fix is in board.go and cards.ts (not links); https://exämple.com/x shows its punycode.\n\n\
         - run `cargo test -p irori-app`\n- then push"
    );
    let who = |name: &str, color: &str, status: &str| AgentRef {
        key: format!("agent:{name}"),
        name: name.into(),
        color: color.into(),
        status: status.into(),
        where_: "Macbook Pro (this one)".into(),
        ..AgentRef::default()
    };
    let ms = vec![
        CardMention {
            name: "oak".into(),
            who: who("oak", "teal", "idle"),
            gone: false,
        },
        CardMention {
            name: "cedar".into(),
            who: who("cedar", "orange", "gone"),
            gone: true,
        },
    ];
    let section = |title: &str| {
        text(title.to_string())
            .font(k.ty.small_medium)
            .color(k.c.text_muted)
    };
    let body = col()
        .key("text-demo/body")
        .w_full()
        .max_w(760.0)
        .gap(14.0)
        .padding(24.0)
        .child(section("Rich text"))
        .child(
            col()
                .w_full()
                .padding(14.0)
                .radius(8.0)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_base)
                .child(rich_with(
                    s,
                    f,
                    "text-demo/rich",
                    &sample,
                    &RichOpts {
                        mentions: &ms,
                        ..RichOpts::default()
                    },
                )),
        )
        .child(section("One line, not interactive"))
        .child(rich_line(
            s,
            f,
            "text-demo/line",
            "Open https://example.com/a_(b) or run `make` (inside a button)",
            &RichOpts {
                interactive: false,
                ..RichOpts::default()
            },
        ))
        .child(section("Links on a blocker"))
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(link_chip(f, "text-demo/l1", "url", "https://www.notion.so/team/plan", "The plan"))
                .child(link_chip(f, "text-demo/l2", "path", "crates/window/src/attachments.rs", ""))
                .child(link_chip(f, "text-demo/l3", "commit", "305f7d75", ""))
                .child(code(f, "text-demo/code", "sh scripts/check.sh")),
        )
        .child(section("Attachments"))
        .child(attachment_list(s, f, "text-demo/files", &s.text.demo_files, false))
        .child(section("Composer"))
        .child(composer(
            s,
            f,
            "text-demo/box",
            demo_box,
            demo_box_mut,
            &ComposerOpts {
                placeholder: "Message your agents… (@ to mention, # for a card)".into(),
                send_button: true,
                asking: true,
                popup_below: true,
                on_send: Some(Rc::new(|s: &mut State, m: Sent| {
                    s.toast("Sent (demo)", &m.text);
                    sent(s, demo_box_mut);
                })),
                ..ComposerOpts::default()
            },
        ))
        .child(rune::el().h(320.0));
    col()
        .key("text-demo")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, "Text", vec![]))
        .child(
            col()
                .key("text-demo/scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(body),
        )
}

/// Every file ref (an object with a sha256) in some JSON.
fn file_refs(v: &serde_json::Value, out: &mut Vec<File>) {
    match v {
        serde_json::Value::Object(m) => {
            if m.get("sha256").and_then(|x| x.as_str()).is_some_and(|x| !x.is_empty()) {
                if let Ok(f) = serde_json::from_value::<File>(v.clone())
                    && !out.iter().any(|x| x.sha256 == f.sha256)
                {
                    out.push(f);
                }
                return;
            }
            for x in m.values() {
                file_refs(x, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| file_refs(x, out)),
        _ => {}
    }
}

/// The headless scene (`irori --snap x.png --scene text`): the demo page,
/// with the attachments the data holds and a composer whose @ picker is
/// open.
pub fn scene(s: &mut State) {
    s.text.demo = true;
    // The attachments on the newest cards' comments, and two plain files.
    let ids: Vec<String> = s.data.cards.iter().take(40).map(|c| c.id.clone()).collect();
    let mut files = Vec::new();
    if let Some(w) = s.w.clone() {
        for id in ids {
            if let Ok(d) = w.card(&id)
                && let Ok(v) = serde_json::to_value(&d)
            {
                file_refs(&v, &mut files);
            }
            if files.len() >= 4 {
                break;
            }
        }
    }
    files.truncate(4);
    files.push(File {
        sha256: "0".repeat(64),
        name: "build-log.txt".into(),
        size: 48_213,
        mime: "text/plain".into(),
    });
    files.push(File {
        sha256: "1".repeat(64),
        name: "design.pdf".into(),
        size: 2_400_000,
        mime: "application/pdf".into(),
    });
    for f in &files {
        if attach::looks_like_image(f) && f.size <= attach::MAX_AUTO_PREVIEW {
            attach::load(s, &f.sha256, false);
        }
    }
    s.text.demo_files = files;
    // The picker's rows: the data's agents, else samples.
    if s.text.cands.iter().filter(|c| !mentions::is_gone(&c.who)).count() < 2 {
        let sample = |name: &str, color: &str, status: &str, device: &str, repo: &str| mentions::Candidate {
            who: AgentRef {
                key: format!("agent:{name}"),
                name: name.into(),
                status: status.into(),
                color: color.into(),
                where_: if device.is_empty() { "Macbook Pro (this one)".into() } else { device.into() },
                device: device.into(),
                ..AgentRef::default()
            },
            repo: repo.into(),
            ..mentions::Candidate::default()
        };
        s.text.cands = vec![
            sample("oak", "teal", "working", "", "irori"),
            sample("oak tree", "teal", "idle", "", "irori"),
            sample("fern", "orange", "idle", "Mac mini", "site"),
            sample("cedar", "pink", "gone", "", "irori"),
        ];
    }
    let c = &mut s.text.demo_box;
    c.set_text("Can you take a look at this with @o");
    c.edit.focused = true;
}
