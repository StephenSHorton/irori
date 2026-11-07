//! The Board screen in Rune (card #674): the person's cards from
//! crates/window's `Window::cards` (the list the React Board reads), in the
//! Board's groups, with a card's page beside it: goal, done-when, steps,
//! open blockers, the thread and a comment box.
//!
//! The state refreshes from the window backend's own pumps: `board:changed`
//! and `roster:changed` (shell.rs) re-read the cards and the open card.

use std::sync::Arc;

use irori_window::Window;
use irori_window::cards::{CardDetail, CardSummary};
use rune::tok::{self, Theme, type_};
use rune::{Color, Cx, El, ScrollSpec, TextEdit, Tx, col, el, row, text, text_input};

/// Which cards the Board shows (the React Board's filter bar, cut down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Decide,
    Working,
}

#[derive(Default)]
pub struct State {
    pub window: Option<Arc<Window>>,
    pub cards: Vec<CardSummary>,
    pub filter: Filter,
    /// The open card's id and page.
    pub open: Option<String>,
    pub detail: Option<CardDetail>,
    pub draft: TextEdit,
    /// The last failed action, shown under the comment box.
    pub error: String,
    /// The tray, kept alive with the state: Rune's on Windows and Linux,
    /// tray-icon's on macOS (shell.rs).
    pub tray: Option<Box<dyn std::any::Any>>,
}

impl State {
    pub fn new(window: Arc<Window>) -> State {
        let mut s = State {
            window: Some(window),
            ..State::default()
        };
        s.refresh();
        s
    }

    /// Re-reads the cards, and the open card's page.
    pub fn refresh(&mut self) {
        let Some(w) = &self.window else { return };
        let mut cards = w.cards();
        cards.sort_by_key(|c| std::cmp::Reverse(c.updated_at.unix_milli_value()));
        self.cards = cards;
        if let Some(id) = self.open.clone() {
            self.detail = w.card(&id).ok();
        }
    }

    /// A card's press: opens its page, or closes it when it is open.
    pub fn select(&mut self, id: &str) {
        if self.open.as_deref() == Some(id) {
            self.open = None;
            self.detail = None;
            return;
        }
        self.open_card(id);
    }

    /// Opens a card's page (a desktop card's Open: never closes it). A
    /// draft for another card doesn't follow.
    pub fn open_card(&mut self, id: &str) {
        if self.open.as_deref() != Some(id) {
            self.draft.clear();
        }
        self.open = Some(id.to_string());
        self.detail = self.window.as_ref().and_then(|w| w.card(id).ok());
        self.error.clear();
    }

    /// The headless `detail` scene: the first card that needs you, else
    /// the newest.
    pub fn select_first(&mut self) {
        let pick = self
            .cards
            .iter()
            .find(|c| group(c) == Group::NeedsYou)
            .or_else(|| self.cards.first())
            .map(|c| c.id.clone());
        if let Some(id) = pick {
            self.select(&id);
        }
    }

    fn send_comment(&mut self) {
        let (Some(w), Some(id)) = (&self.window, &self.open) else {
            return;
        };
        let body = self.draft.text.trim().to_string();
        if body.is_empty() {
            return;
        }
        match w.comment(id, &body, "", &[], &[]) {
            Ok(_) => {
                self.draft.clear();
                self.error.clear();
                self.refresh();
            }
            Err(e) => self.error = e,
        }
    }
}

/// The Board's groups, in the order the page shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    NeedsYou,
    Working,
    Waiting,
    Open,
    NotYet,
}

const GROUPS: [(Group, &str); 5] = [
    (Group::NeedsYou, "Needs you"),
    (Group::Working, "Working"),
    (Group::Waiting, "Waiting"),
    (Group::Open, "Open"),
    (Group::NotYet, "Not yet"),
];

fn group(c: &CardSummary) -> Group {
    if c.open_blockers > 0 || c.ready {
        Group::NeedsYou
    } else if c.later.is_some() {
        Group::NotYet
    } else if c.waiting_on > 0 || c.state == "input_required" {
        Group::Waiting
    } else if c.state == "working" || c.state == "in_review" {
        Group::Working
    } else {
        Group::Open
    }
}

fn shown(c: &CardSummary, f: Filter) -> bool {
    match f {
        Filter::All => true,
        Filter::Decide => group(c) == Group::NeedsYou,
        Filter::Working => group(c) == Group::Working,
    }
}

fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::BLACK)
}

/// A state's colour: amber for a decision, green for work, blue for review.
fn state_color(c: &CardSummary) -> Color {
    match group(c) {
        Group::NeedsYou => hex("#e0a526"),
        Group::Working if c.state == "in_review" => hex("#5b9cf0"),
        Group::Working => hex("#3fb37f"),
        Group::Waiting => hex("#9a8cf0"),
        Group::Open | Group::NotYet => hex("#8b8f98"),
    }
}

fn state_label(c: &CardSummary) -> &'static str {
    if c.ready {
        return "Ready for you";
    }
    if c.open_blockers > 0 {
        return "Needs a decision";
    }
    match c.state.as_str() {
        "working" => "Working",
        "in_review" => "In review",
        "input_required" => "Waiting",
        "submitted" if c.later.is_some() => "Not yet",
        "submitted" => "Open",
        _ => "Open",
    }
}

fn card_ref(c: &CardSummary) -> String {
    if c.short.is_empty() {
        return "new".into();
    }
    format!("{}#{}", c.tag, c.short)
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let body = row()
        .key("body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(cards_pane(s, t))
        .child(match &s.detail {
            Some(d) => detail_pane(s, d, t),
            None => el().key("no-detail").w(0.0),
        });
    col()
        .key("root")
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(header(s, t))
        .child(body)
}

fn header(s: &State, t: Theme) -> El {
    let decide = s
        .cards
        .iter()
        .filter(|c| group(c) == Group::NeedsYou)
        .count();
    let working = s
        .cards
        .iter()
        .filter(|c| group(c) == Group::Working)
        .count();
    let mut tabs = row().key("filters").gap(6.0).center_y();
    for (f, label, n) in [
        (Filter::All, "All", s.cards.len()),
        (Filter::Decide, "Needs a decision", decide),
        (Filter::Working, "Working", working),
    ] {
        let on = s.filter == f;
        tabs = tabs.child(
            row()
                .key(format!("filter-{label}"))
                .role("tab")
                .label(label)
                .checked(on)
                .focusable(true)
                .h(26.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(13.0)
                .fill(if on { t.control_off } else { t.bg })
                .border(1.0, t.card_border)
                .on_press(move |s: &mut State| s.filter = f)
                .child(text(label).font(type_::LABEL).color(t.text))
                .child(
                    text(n.to_string())
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                ),
        );
    }
    row()
        .key("header")
        .w_full()
        .h(52.0)
        .px(20.0)
        .gap(16.0)
        .center_y()
        .border(1.0, t.separator)
        .child(
            text("Irori")
                .font(rune::tok::TextStyle::new(16.0, 600.0))
                .color(t.text),
        )
        .child(text("Board").font(type_::TITLE).color(t.text_secondary))
        .child(el().grow(1.0))
        .child(tabs)
}

fn cards_pane(s: &State, t: Theme) -> El {
    let mut list = col().key("groups").w_full().gap(20.0).padding(20.0);
    let mut any = false;
    for (g, label) in GROUPS {
        let cards: Vec<&CardSummary> = s
            .cards
            .iter()
            .filter(|c| group(c) == g && shown(c, s.filter))
            .collect();
        if cards.is_empty() {
            continue;
        }
        any = true;
        let mut grid = row()
            .key(format!("grid-{label}"))
            .w_full()
            .gap(12.0)
            .flex_wrap();
        for c in cards.iter().copied() {
            grid = grid.child(card(c, s.open.as_deref() == Some(c.id.as_str()), t));
        }
        list = list.child(
            col()
                .key(format!("group-{label}"))
                .w_full()
                .gap(10.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(text(label).font(type_::SECTION).color(t.text))
                        .child(
                            text(cards.len().to_string())
                                .font(type_::SECTION)
                                .color(t.text_secondary),
                        ),
                )
                .child(grid),
        );
    }
    if !any {
        list = list.child(
            text("No cards here.")
                .key("empty")
                .font(type_::BODY)
                .color(t.text_secondary),
        );
    }
    col().key("cards-pane").grow(1.0).h_full().min_h(0.0).child(
        list.scroll(ScrollSpec {
            stick_bottom: false,
        })
        .grow(1.0)
        .min_h(0.0),
    )
}

fn card(c: &CardSummary, open: bool, t: Theme) -> El {
    let id = c.id.clone();
    let color = state_color(c);
    let mut body = col()
        .key(format!("card-{}", c.id))
        .role("button")
        .label(c.title.clone())
        .focusable(true)
        .w(300.0)
        .padding(12.0)
        .gap(6.0)
        .radius(8.0)
        .fill(t.card)
        .border(
            if open { 2.0 } else { 1.0 },
            if open { color } else { t.card_border },
        )
        .on_press(move |s: &mut State| s.select(&id))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(card_ref(c))
                        .font(type_::MONO_CAPTION.mono())
                        .color(t.text_secondary),
                )
                .child(el().grow(1.0))
                .child(text(state_label(c)).font(type_::LABEL).color(color)),
        )
        .child(
            text(c.title.clone())
                .font(type_::TITLE)
                .color(t.text)
                .w_full()
                .max_lines(2),
        );
    if c.open_blockers > 0 && !c.blocker_question.is_empty() {
        body = body.child(
            text(c.blocker_question.clone())
                .font(type_::BODY)
                .color(color)
                .w_full()
                .max_lines(3),
        );
    }
    if c.progress.total > 0 {
        let frac = c.progress.done as f32 / c.progress.total as f32;
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    el().grow(1.0)
                        .h(4.0)
                        .radius(2.0)
                        .fill(t.control_off)
                        .child(el().w(276.0 * 0.78 * frac).h(4.0).radius(2.0).fill(color)),
                )
                .child(
                    text(format!("{}/{}", c.progress.done, c.progress.total))
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                ),
        );
    }
    let who = &c.holder.who;
    let held = if who.name.is_empty() {
        "Nobody holds it".to_string()
    } else if c.holder.others > 0 {
        format!("{} +{}", who.name, c.holder.others)
    } else {
        who.name.clone()
    };
    body = body.child(
        text(held)
            .font(type_::LABEL)
            .color(t.text_secondary)
            .w_full()
            .max_lines(1),
    );
    if let Some(l) = &c.left_off {
        body = body.child(
            text(format!("Left off: {}", l.text))
                .font(type_::BODY)
                .color(t.text_secondary)
                .w_full()
                .max_lines(2),
        );
    }
    body
}

fn detail_pane(s: &State, d: &CardDetail, t: Theme) -> El {
    let c = &d.summary;
    let color = state_color(c);
    let mut page = col().key("detail").w_full().gap(14.0).padding(20.0);
    page = page
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(card_ref(c))
                        .font(type_::MONO_CAPTION.mono())
                        .color(t.text_secondary),
                )
                .child(text(state_label(c)).font(type_::LABEL).color(color))
                .child(el().grow(1.0))
                .child({
                    // The clipboard: Rune's own (Tx::copy), no plugin.
                    let r = format!("{}, task:{}", card_ref(c), c.id);
                    text("Copy ref")
                        .key("copy-ref")
                        .role("button")
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .on_press_tx(move |_: &mut State, tx: &mut Tx| tx.copy(r.clone()))
                })
                .child(
                    text("Close")
                        .key("close")
                        .role("button")
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .on_press(|s: &mut State| {
                            s.open = None;
                            s.detail = None;
                        }),
                ),
        )
        .child(
            text(c.title.clone())
                .font(rune::tok::TextStyle::new(17.0, 600.0))
                .color(t.text)
                .w_full()
                .wrap(),
        );
    if !d.goal.is_empty() {
        page = page.child(section("Objective", &d.goal, t));
    }
    if !d.done_when.is_empty() {
        page = page.child(section("Done when", &d.done_when, t));
    }
    if !d.steps.is_empty() {
        let done = d.steps.iter().filter(|s| s.done).count();
        let mut steps = col()
            .key("steps")
            .w_full()
            .gap(6.0)
            .child(label(&format!("Steps {done}/{}", d.steps.len()), t));
        for st in &d.steps {
            let (mark, mc) = if st.done {
                ("✓", hex("#3fb37f"))
            } else if st.doing {
                ("▸", hex("#e0a526"))
            } else {
                ("○", t.text_secondary)
            };
            steps = steps.child(
                row()
                    .key(format!("step-{}", st.id))
                    .w_full()
                    .gap(8.0)
                    .child(text(mark).font(type_::BODY).color(mc).w(14.0))
                    .child(
                        text(st.text.clone())
                            .font(type_::BODY)
                            .color(if st.done { t.text_secondary } else { t.text })
                            .grow(1.0)
                            .min_w(0.0)
                            .wrap(),
                    ),
            );
        }
        page = page.child(steps);
    }
    for b in d.blockers.iter().filter(|b| b.resolved.is_none()) {
        let mut bl = col()
            .key(format!("blocker-{}", b.id))
            .w_full()
            .gap(6.0)
            .padding(12.0)
            .radius(8.0)
            .border(1.0, hex("#e0a526"))
            .child(label("Needs a decision", t))
            .child(
                text(b.question.clone())
                    .font(type_::TITLE)
                    .color(t.text)
                    .w_full()
                    .wrap(),
            );
        if !b.context.is_empty() {
            bl = bl.child(
                text(b.context.clone())
                    .font(type_::BODY)
                    .color(t.text_secondary)
                    .w_full()
                    .wrap(),
            );
        }
        for (i, o) in b.options.iter().enumerate() {
            let rec = if o.recommended { "  (recommended)" } else { "" };
            bl = bl.child(
                text(format!("{}. {}{rec}", i + 1, o.answer))
                    .font(type_::BODY)
                    .color(t.text)
                    .w_full()
                    .wrap(),
            );
        }
        page = page.child(bl);
    }
    if !d.thread.is_empty() {
        let mut th = col()
            .key("thread")
            .w_full()
            .gap(10.0)
            .child(label("Thread", t));
        let from = d.thread.len().saturating_sub(12);
        for it in &d.thread[from..] {
            th = th.child(
                col()
                    .key(format!("item-{}", it.id))
                    .w_full()
                    .gap(2.0)
                    .child(
                        text(if it.who.name.is_empty() {
                            it.kind.clone()
                        } else {
                            it.who.name.clone()
                        })
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                    )
                    .child(
                        text(it.text.clone())
                            .font(type_::BODY)
                            .color(t.text)
                            .w_full()
                            .wrap()
                            .max_lines(8),
                    ),
            );
        }
        page = page.child(th);
    }
    page = page.child(comment_box(s, t));
    col()
        .key("detail-pane")
        .w(440.0)
        .h_full()
        .min_h(0.0)
        .border(1.0, t.separator)
        .fill(t.card)
        .child(
            page.scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
        )
}

fn comment_box(s: &State, t: Theme) -> El {
    let field = text_input(&s.draft, type_::BODY)
        .key("comment")
        .placeholder("Comment on this card…", t.text_secondary)
        .w_full()
        .min_h(60.0)
        .padding(8.0)
        .radius(6.0)
        .border(1.0, t.card_border)
        .fill(t.bg)
        .on_edit(|s: &mut State| &mut s.draft)
        .on_submit(|s: &mut State, _: &mut Tx| s.send_comment());
    let send = row()
        .key("send")
        .role("button")
        .label("Comment")
        .focusable(true)
        .h(28.0)
        .px(12.0)
        .center_y()
        .radius(6.0)
        .fill(tok::accent(0, true))
        .on_press(|s: &mut State| s.send_comment())
        .child(text("Comment").font(type_::LABEL).color(hex("#ffffff")));
    let mut c = col()
        .key("comment-box")
        .w_full()
        .gap(8.0)
        .child(field)
        .child(row().w_full().child(el().grow(1.0)).child(send));
    if !s.error.is_empty() {
        c = c.child(
            text(s.error.clone())
                .font(type_::BODY)
                .color(hex("#e5534b"))
                .w_full()
                .wrap(),
        );
    }
    c
}

fn label(s: &str, t: Theme) -> El {
    text(s.to_string())
        .font(type_::SECTION)
        .color(t.text_secondary)
}

fn section(title: &str, body: &str, t: Theme) -> El {
    col().w_full().gap(4.0).child(label(title, t)).child(
        text(body.to_string())
            .font(type_::BODY)
            .color(t.text)
            .w_full()
            .wrap(),
    )
}
