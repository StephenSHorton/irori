//! Notes (P7 Notes; docs/design/notes.md §9, card #413): the Rune port of
//! the React app's components/notes/notes-page.tsx and parts.tsx, over
//! lib/notes.ts (in [`logic`]). The short rules, gotchas, decisions,
//! pointers and preferences agents get on wake. Notes are the person's own
//! (card #616): one list, read only by their agents; an agent's note, edit
//! or removal applies at once. Each row says who wrote it, and the person
//! edits or removes any of them. The list filters by kind and a search.

pub mod logic;

use rune::tok::TextStyle;
use rune::{Color, El, ScrollSpec, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, Select, SelectAppearance,
    TextArea, TextInput,
};

use irori_window::events::EVENT_BOARD_CHANGED;
use irori_window::notes::{NoteEditInput, NoteInput, NoteView};

use crate::spaces::logic::{now_ms, plain_block, plain_line, relative_time};
use crate::ui::{Event, Frame, Route, State, icons};
use logic::{NOTE_KINDS, NOTE_MAX_CHARS};

/// New note or Edit.
#[derive(Default)]
pub struct NoteForm {
    /// The note edited; None for New note.
    pub editing: Option<String>,
    pub title: String,
    pub text: TextEdit,
    pub kind: String,
    pub tags: TextEdit,
    pub error: String,
    pub busy: bool,
    pub kind_open: bool,
}

pub struct Model {
    pub list: Vec<NoteView>,
    /// The first read hasn't landed.
    pub loading: bool,
    pub error: Option<String>,
    /// The kind chip picked: "all" or a kind.
    pub kind: &'static str,
    pub query: TextEdit,
    pub form: Option<NoteForm>,
    /// A failed Remove, under its row: (note id, why).
    pub row_error: Option<(String, String)>,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            list: Vec::new(),
            loading: true,
            error: None,
            kind: "all",
            query: TextEdit::new(""),
            form: None,
            row_error: None,
        }
    }
}

fn reload(s: &mut State) {
    s.spawn(
        |w| w.notes_list(false),
        |s, r| {
            s.notes_page.loading = false;
            match r {
                Ok(v) => {
                    s.notes_page.list = v;
                    s.notes_page.error = None;
                }
                Err(e) => s.notes_page.error = Some(plain_line(&e)),
            }
        },
    );
}

/// Note rows fold into the board's changes: board:changed covers a note
/// written, edited or removed, here or synced.
pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" | EVENT_BOARD_CHANGED => reload(s),
        _ => {}
    }
}

/// The headless scene that shows it (`irori --snap x.png --scene notes`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Notes);
}

/// Opens New note (also the palette's).
pub fn open_new_note(s: &mut State) {
    s.notes_page.form = Some(NoteForm {
        title: "New note".into(),
        kind: "rule".into(),
        tags: TextEdit::new("everywhere"),
        ..NoteForm::default()
    });
}

fn open_edit(s: &mut State, n: &NoteView) {
    s.notes_page.form = Some(NoteForm {
        editing: Some(n.id.clone()),
        title: "Edit note".into(),
        text: TextEdit::new(n.text.clone()),
        kind: n.kind.clone(),
        tags: TextEdit::new(n.tags.join(", ")),
        ..NoteForm::default()
    });
}

fn t(size: f32) -> TextStyle {
    TextStyle::new(size, 400.0)
}

/// Who a key names: you, a space member, or an agent by its roster name.
fn who(s: &State, key: &str) -> String {
    if key.is_empty() {
        return "someone".into();
    }
    if key.starts_with("user:") {
        return "You".into();
    }
    if let Some(id) = key.strip_prefix("person:") {
        for b in &s.data.spaces {
            if let Some(m) = b.members.iter().find(|m| m.person_id == id) {
                return if m.me { "You".into() } else { plain_line(&m.name) };
            }
        }
        return "a member".into();
    }
    s.data
        .agents
        .iter()
        .find(|a| a.card.key == key)
        .map(|a| plain_line(&a.card.name))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "unknown agent".into())
}

/// A board: tag names a space by its tag.
fn board_name(s: &State, tag: &str) -> String {
    s.data
        .spaces
        .iter()
        .find(|b| b.tag.eq_ignore_ascii_case(tag))
        .map_or_else(|| tag.to_string(), |b| plain_line(&b.name))
}

fn kind_color(k: &Kit, kind: &str) -> Color {
    match kind {
        "rule" => k.c.danger.icon,
        "gotcha" => k.c.warning.icon,
        "decision" => k.c.success.icon,
        "pointer" => k.c.info.icon,
        _ => k.c.text_muted,
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// A note's kind, as a tinted badge.
fn kind_badge(k: &Kit, kind: &str) -> El {
    let c = kind_color(k, kind);
    el().h(20.0)
        .px(6.0)
        .radius(5.0)
        .center()
        .fill(k.alpha(c, 0.16))
        .child(text(capital(kind)).font(TextStyle::new(10.0, 600.0)).color(c))
}

/// A tag's chip.
fn tag_chip(k: &Kit, label: String, raw: &str) -> El {
    el().px(6.0)
        .py(2.0)
        .radius(5.0)
        .border(1.0, k.c.border_muted)
        .help(raw.to_string())
        .child(text(label).font(t(10.5)).color(k.c.text_muted))
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let m = &s.notes_page;
    let head = row()
        .key("notes-header")
        .w_full()
        .gap(12.0)
        .center_y()
        .flex_wrap()
        .child(text("Notes").font(TextStyle::new(18.0, 600.0)).color(k.c.text_base))
        .child(el().grow(1.0))
        .child(
            TextInput::new(&m.query)
                .placeholder("Search notes")
                .label("Search notes")
                .leading_icon(&icons::SEARCH)
                .width(Some(224.0))
                .text_style(t(12.5))
                .on_edit(|s: &mut State| &mut s.notes_page.query)
                .build(k)
                .key("notes-search"),
        )
        .child(
            Button::new("New note")
                .icon(&icons::PLUS)
                .on_press(open_new_note)
                .build(k)
                .key("notes-new"),
        );
    let mut page = col()
        .key("notes-body")
        .w_full()
        .gap(16.0)
        .px(24.0)
        .pt(14.0)
        .pb(24.0)
        .child(head)
        .child(
            text("Notes stay on your devices and only your agents read them, even when a note is about a shared space. Agents write and change them as they learn; edit or remove any here. Agents get the few that match their card on wake.")
                .font(t(13.0))
                .color(k.c.text_muted)
                .w_full()
                .max_w(768.0)
                .wrap(),
        );
    if let Some(e) = &m.error {
        page = page.child(
            text(format!("Could not load the notes: {e}"))
                .font(t(12.0))
                .color(k.c.danger.icon)
                .w_full()
                .wrap(),
        );
    }
    if !m.loading {
        page = page.child(list(s, f));
    }
    form_dialog(s, f);
    col()
        .key("notes-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            col()
                .key("notes-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(page),
        )
}

/// The kind chips and the notes.
fn list(s: &State, f: &Frame) -> El {
    let k = f.k;
    let m = &s.notes_page;
    let mut chips = row().key("notes-kinds").gap(6.0).flex_wrap().role("group").label("Kind");
    for c in logic::kind_counts(&m.list) {
        let on = m.kind == c.kind;
        let label = if c.kind == "all" { "All".to_string() } else { format!("{}s", capital(c.kind)) };
        let kind = c.kind;
        let mut chip = row()
            .key(format!("notes-kind-{kind}"))
            .role("button")
            .label(label.clone())
            .checked(on)
            .focusable(true)
            .h(26.0)
            .px(10.0)
            .gap(5.0)
            .center_y()
            .radius(13.0)
            .on_press(move |s: &mut State| s.notes_page.kind = kind)
            .child(text(label).font(t(12.0)).color(if on { k.c.text_base } else { k.c.text_muted }))
            .child(text(c.count.to_string()).font(t(12.0)).color(k.c.text_muted));
        chip = if on { chip.fill(k.c.layer_03) } else { chip.border(1.0, k.c.border_muted) };
        chips = chips.child(chip);
    }
    let shown = logic::shown(&m.list, m.kind, &m.query.text);
    let body = if shown.is_empty() {
        el().key("notes-empty")
            .w_full()
            .px(16.0)
            .py(24.0)
            .radius(8.0)
            .border(1.0, k.c.border_muted)
            .center()
            .child(
                text(if !m.query.text.trim().is_empty() || m.kind != "all" {
                    "No notes match."
                } else {
                    "No notes yet. Agents add them as they learn, or write one with New note."
                })
                .font(t(13.0))
                .color(k.c.text_muted)
                .wrap(),
            )
    } else {
        let mut l = col()
            .key("notes-list")
            .w_full()
            .radius(10.0)
            .clip_rounded(10.0)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_muted)
            .label("Notes");
        let n = shown.len();
        for (i, note) in shown.into_iter().enumerate() {
            l = l.child(note_row(s, f, note));
            if i + 1 < n {
                l = l.child(el().w_full().h(1.0).fill(k.alpha(k.c.border_muted, 0.7)));
            }
        }
        l
    };
    col().key("notes-section").w_full().gap(8.0).label("Your notes").child(chips).child(body)
}

fn note_row(s: &State, f: &Frame, n: &NoteView) -> El {
    let k = f.k;
    let (edit, remove) = (n.clone(), n.id.clone());
    let top = row()
        .w_full()
        .gap(8.0)
        .items_start()
        .child(kind_badge(k, &n.kind).mt(1.0))
        .child(
            text(plain_block(&n.text))
                .font(t(13.0))
                .color(k.c.text_base)
                .wrap()
                .selectable(true)
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            IconButton::new(&icons::PENCIL)
                .variant(IconButtonVariant::Ghost)
                .label("Edit the note")
                .help("Edit the note")
                .on_press(move |s: &mut State| open_edit(s, &edit))
                .build(k),
        )
        .child(
            IconButton::new(&icons::TRASH)
                .variant(IconButtonVariant::Ghost)
                .label("Remove the note")
                .help("Remove the note: no agent gets it any more")
                .on_press(move |s: &mut State| {
                    let id = remove.clone();
                    s.notes_page.row_error = None;
                    s.spawn(
                        {
                            let id = id.clone();
                            move |w| w.note_remove(&id, "")
                        },
                        move |s, r| match r {
                            Ok(_) => reload(s),
                            Err(e) => s.notes_page.row_error = Some((id, plain_line(&e))),
                        },
                    );
                })
                .build(k),
        );
    let mut c = col()
        .key(format!("notes-row-{}", n.id))
        .w_full()
        .gap(6.0)
        .px(16.0)
        .py(12.0)
        .child(top)
        .child(meta(s, f, n));
    if let Some((id, e)) = &s.notes_page.row_error {
        if *id == n.id {
            c = c.child(text(e.clone()).font(t(12.0)).color(k.c.danger.icon).wrap());
        }
    }
    c
}

/// A row's second line: tags, source, author and accepter, and Still
/// true? when Irori found something it names has changed.
fn meta(s: &State, f: &Frame, n: &NoteView) -> El {
    let k = f.k;
    let mut r = row().w_full().gap(6.0).flex_wrap().center_y();
    let bn = |tag: &str| board_name(s, tag);
    for tg in &n.tags {
        r = r.child(tag_chip(k, logic::tag_label(tg, &bn), tg));
    }
    let src = logic::source_label(&n.source);
    if !src.is_empty() && !n.task_id.is_empty() {
        let id = n.task_id.clone();
        r = r.child(
            el().key(format!("notes-src-{}", n.id))
                .role("button")
                .label(src.clone())
                .focusable(true)
                .px(6.0)
                .py(2.0)
                .radius(5.0)
                .fill(k.c.layer_03)
                .help("Open the card it came from")
                .on_press(move |s: &mut State| s.open_card(&id))
                .child(text(src).font(t(11.0)).color(k.c.text_base)),
        );
    }
    let mut by = who(s, &n.by);
    if !n.accepted_by.is_empty() && n.accepted_by != n.by {
        by = format!("{by} · kept by {}", who(s, &n.accepted_by));
    }
    let when = relative_time(n.updated.unix_milli_value(), now_ms());
    r = r.child(text(format!("{by} · {when}")).font(t(11.0)).color(k.c.text_muted));
    if !n.stale.is_empty() {
        let w = k.c.warning.icon;
        r = r.child(
            row()
                .gap(4.0)
                .center_y()
                .help("Irori found something it names has changed")
                .child(icon(&icons::ALERT_TRIANGLE, 12.0).color(w))
                .child(text(format!("Still true? {}", plain_line(&n.stale))).font(t(11.0)).color(w)),
        );
    }
    r
}

// ---- the form ---------------------------------------------------------------

fn form_mut(s: &mut State) -> &mut NoteForm {
    s.notes_page.form.get_or_insert_with(NoteForm::default)
}

fn submit(s: &mut State) {
    let Some(fm) = s.notes_page.form.as_mut() else { return };
    if fm.busy {
        return;
    }
    let tags = logic::split_tags(&fm.tags.text);
    let problem = logic::note_problem(&fm.kind, &fm.text.text, &tags);
    if !problem.is_empty() {
        fm.error = problem;
        return;
    }
    fm.busy = true;
    fm.error.clear();
    let (kind, body, tags) = (fm.kind.clone(), fm.text.text.trim().to_string(), logic::clean_tags(&tags).0);
    let editing = fm.editing.clone();
    s.spawn(
        move |w| match editing {
            Some(id) => w
                .note_edit(
                    &id,
                    NoteEditInput {
                        kind: Some(kind),
                        text: Some(body),
                        tags: Some(tags),
                    },
                )
                .map(|_| ()),
            None => w.note_new(NoteInput { kind, text: body, tags }).map(|_| ()),
        },
        |s, r| match r {
            Ok(()) => {
                s.notes_page.form = None;
                reload(s);
            }
            Err(e) => {
                let fm = form_mut(s);
                fm.busy = false;
                fm.error = plain_line(&e);
            }
        },
    );
}

/// New note and Edit: kind, text and where it applies. Every note is the
/// person's own (card #616), so there is no scope to pick.
fn form_dialog(s: &State, f: &mut Frame) {
    let Some(fm) = &s.notes_page.form else { return };
    let k = f.k;
    let chars = fm.text.text.trim().chars().count();
    let tags = logic::split_tags(&fm.tags.text);
    let kinds: Vec<String> = NOTE_KINDS.iter().map(|x| capital(x)).collect();
    let kind_sel = Select::new("notes-kind", &kinds)
        .label("Kind")
        .appearance(SelectAppearance::Field)
        .width(160.0)
        .current(Some(&capital(&fm.kind)))
        .open(fm.kind_open)
        .on_toggle(|s: &mut State| {
            let fm = form_mut(s);
            fm.kind_open = !fm.kind_open;
        })
        .on_close(|s: &mut State| form_mut(s).kind_open = false)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
            let fm = form_mut(s);
            fm.kind = v.to_lowercase();
            fm.kind_open = false;
        })
        .build(k, f.layer);
    let label = |s: &str| text(s.to_string()).font(t(12.5)).color(k.c.text_base).w(80.0);
    let mut applies = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(
            TextInput::new(&fm.tags)
                .label("Applies to")
                .width(None)
                .text_style(t(12.0).mono())
                .on_edit(|s: &mut State| &mut form_mut(s).tags)
                .build(k)
                .key("notes-form-tags")
                .w_full(),
        )
        .child(
            text("Comma-separated: everywhere, repo:<repo key>, path:<repo key>:<path>, board:<tag>, machine:<device>, topic:<word>")
                .font(t(11.0))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    if !tags.is_empty() {
        let mut chips = row().gap(4.0).flex_wrap();
        for tg in &tags {
            chips = chips.child(tag_chip(k, logic::tag_label(tg, &|_| String::new()), tg));
        }
        applies = applies.child(chips);
    }
    let mut body = col()
        .key("notes-form")
        .w_full()
        .gap(12.0)
        .px(16.0)
        .pb(16.0)
        .child(
            // Rune's dialog description doesn't wrap in a fitted panel.
            text("A short fact an agent should know before it acts. Long material belongs in the repo's docs.")
                .font(k.ty.ui.lh(20.0))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        )
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(
                    TextArea::new(&fm.text)
                        .label("The note")
                        .rows(3)
                        .width(None)
                        .on_edit(|s: &mut State| &mut form_mut(s).text)
                        .build(k)
                        .key("notes-form-text")
                        .w_full(),
                )
                .child(
                    row().w_full().justify_end().child(
                        text(format!("{chars}/{NOTE_MAX_CHARS}"))
                            .font(t(11.0))
                            .color(if chars > NOTE_MAX_CHARS { k.c.danger.icon } else { k.c.text_muted }),
                    ),
                ),
        )
        .child(row().w_full().gap(12.0).center_y().child(label("Kind")).child(kind_sel))
        .child(row().w_full().gap(12.0).items_start().child(label("Applies to").pt(4.0)).child(applies));
    if !fm.error.is_empty() {
        body = body.child(text(fm.error.clone()).font(t(12.0)).color(k.c.danger.icon).w_full().wrap());
    }
    let busy = fm.busy;
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .justify_end()
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .disabled(busy)
                    .on_press(|s: &mut State| s.notes_page.form = None)
                    .build(k),
            )
            .child(
                Button::new(if busy { "Saving…" } else { "Keep" })
                    .disabled(busy)
                    .on_press(submit)
                    .build(k),
            ),
    );
    Dialog::new("notes-form-dialog", fm.title.clone())
        .width(512.0)
        .fit()
        .open(true)
        .on_close(move |s: &mut State| {
            if !busy {
                s.notes_page.form = None;
            }
        })
        .child(body)
        .build(k, f.layer);
}
