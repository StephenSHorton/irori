//! The composer (the React app's common/mention-textarea.tsx plus the
//! attachments' useFileAttach): a text box with the @ picker and the #
//! picker, files (the clip, a drop, a pasted screenshot or file), a slot
//! for a mic button and a Send. Every box the person talks to agents in
//! uses it: the ask box, comments and thread replies, blocker answers, a
//! message's Reply, Talk.
//!
//! Typing "@" opens a list of agents above (or below) the box, filtered by
//! what follows. Focus stays in the box. Tab enters the list and steps
//! through it (Shift+Tab back, Up and Down too), Enter or a click puts
//! "@name " in the text and records the agent's key, Esc closes the list
//! and keeps the text. While the list is open Enter never sends (with no
//! rows it does nothing). Deleting the "@name" drops its key; pasting one,
//! or typing a space or punctuation straight after an exact name, records
//! it. "#" opens the card list the same way and puts "#12 " in the text.
//! Enter sends, Shift+Enter is a new line.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_window::board::UploadFile;
use irori_window::mentions::MentionOutcome;
use rune::{Align, El, FileDialogOptions, Key, KeyEvent, Side, Span, TextEdit, Tx, col, el, row, text, text_input};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, PopupItem, PopupList};

use super::attach::{self, MAX_UPLOAD_FILES, Pending};
use super::icons;
use super::mentions::{
    self, CardCandidate, Candidate, Mention, MenuKey, Token, add_mention, after, agent_on_rows,
    board_rows, card_token_at, filter_candidates, filter_cards, insert_card_ref, insert_mention,
    is_full, is_gone, keep_mentions, menu_key, move_highlight, row_name, settle, settle_range,
    split_mentions, token_at,
};
use super::refs::{RefStatus, card_status, device_color, tone_color};
use super::rich::plain_line;
use crate::ui::{Frame, State};

/// Where a composer lives in the state: the caller's Model owns it, and
/// hands these two to [`composer`] so the box's handlers can reach it.
pub type Get = fn(&State) -> &Composer;
pub type GetMut = fn(&mut State) -> &mut Composer;

/// What a Send hands the site: the text, the agents it names, the files.
#[derive(Clone)]
pub struct Sent {
    pub text: String,
    pub mentions: Vec<Mention>,
    pub files: Vec<UploadFile>,
}

/// Called with what to send. The site sends it (through `s.act`) and, once
/// it went, calls [`Composer::sent`] (or [`sent`]) to empty the box.
pub type SendFn = Rc<dyn Fn(&mut State, Sent)>;

/// A box's state, owned by the caller's Model.
#[derive(Default)]
pub struct Composer {
    pub edit: TextEdit,
    /// The agents picked, by the "@name"s in the text.
    pub mentions: Vec<Mention>,
    /// Files waiting to go.
    pub files: Vec<Pending>,
    /// Why a paste or drop wasn't taken.
    pub refused: Option<String>,
    /// "Not delivered: …" after a send, for a few seconds.
    pub missed: String,
    missed_at: Option<Instant>,
    /// The drafts key ("" keeps no draft).
    pub key: String,
    /// A send is on its way: the box takes no new files.
    pub sending: bool,
    /// The token start Esc (or a pick) closed the list on.
    dismissed: Option<usize>,
    /// The highlighted row for one token and query; -1 is none.
    hi: (String, i32),
    /// The text before the latest change.
    last: String,
    next_file: u64,
}

impl Composer {
    /// An empty box keeping its draft under `key` ("" for none). The draft
    /// itself loads with [`open_draft`].
    pub fn new(key: &str) -> Composer {
        Composer {
            key: key.to_string(),
            ..Composer::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.edit.text
    }

    /// Nothing typed and no files.
    pub fn is_empty(&self) -> bool {
        self.edit.text.trim().is_empty() && self.files.is_empty()
    }

    /// Puts `t` in the box (mentions whose "@name" left it go).
    pub fn set_text(&mut self, t: &str) {
        self.edit.replace_text(t);
        self.last = t.to_string();
        self.mentions = keep_mentions(&self.mentions, t);
        self.dismissed = None;
    }

    /// Empties the box after a send: text, mentions, files.
    pub fn sent(&mut self) {
        self.edit.clear();
        self.last.clear();
        self.mentions.clear();
        self.files.clear();
        self.refused = None;
        self.dismissed = None;
        self.sending = false;
    }

    /// The "Not delivered: …" line for a send's outcomes.
    pub fn missed(&mut self, outcomes: &[MentionOutcome]) {
        self.missed = mentions::not_delivered(outcomes);
        self.missed_at = Some(Instant::now());
    }

    /// Adds files, at most MAX_UPLOAD_FILES; a refusal says why.
    pub fn add_files(&mut self, got: Vec<Result<Pending, String>>) {
        for r in got {
            match r {
                Ok(p) if self.files.len() < MAX_UPLOAD_FILES => {
                    self.files.push(p);
                    self.refused = None;
                }
                Ok(_) => self.refused = Some(format!("at most {MAX_UPLOAD_FILES} files")),
                Err(e) => self.refused = Some(e),
            }
        }
    }

    fn file_id(&mut self, n: usize) -> u64 {
        let id = self.next_file;
        self.next_file += n as u64;
        id
    }
}

/// Loads the draft kept under `key` into the box (and keeps it there from
/// now on). Call it when the box's page opens.
pub fn open_draft(s: &mut State, get_mut: GetMut, key: &str) {
    let d = super::drafts::draft(s, key);
    let c = get_mut(s);
    c.key = key.to_string();
    c.set_text(&d.text);
    c.mentions = keep_mentions(&d.mentions, &d.text);
}

/// Empties the box after a send and drops its draft.
pub fn sent(s: &mut State, get_mut: GetMut) {
    let key = get_mut(s).key.clone();
    get_mut(s).sent();
    super::drafts::set_draft(s, &key, "", &[]);
    super::drafts::flush(s);
}

/// How the box looks and what it may do.
#[derive(Clone, Default)]
pub struct ComposerOpts {
    pub placeholder: String,
    /// The box's accessible name (the placeholder when empty).
    pub label: String,
    /// Called on Enter or Send. None: Enter does nothing.
    pub on_send: Option<SendFn>,
    /// Show a Send button after the box.
    pub send_button: bool,
    /// A mic button (P6's), right after the box.
    pub mic: Option<El>,
    /// More controls before Send (a "Agents answer now" toggle, say).
    pub extra: Vec<El>,
    /// The card the box is on: its "the agent on #N" row comes first.
    pub card: String,
    /// The shared board the box is on ("" your own): its people and their
    /// agents may be mentioned there.
    pub board: String,
    /// Set: no pickers, and this is why (the box's tooltip).
    pub no_mentions: Option<String>,
    /// Set: no files here, and this is why (the clip shows disabled).
    pub no_files: Option<String>,
    /// No clip at all (and pastes of files go in as text).
    pub hide_files: bool,
    pub disabled: bool,
    /// Rows the box grows between (default 1 to 8).
    pub rows: Option<(usize, usize)>,
    /// The pickers open below the box (default: above, as a composer at
    /// the bottom of a page wants).
    pub popup_below: bool,
    /// Show "Asking @oak only" under the box while it names agents (the
    /// ask box).
    pub asking: bool,
}

/// The picker rows a box offers: the agents (own board's, or this shared
/// board's) and "the agent on #N" for each card an agent is live on.
pub fn candidates(s: &State, board: &str, card: &str) -> Vec<Candidate> {
    let own = board_rows(&s.text.cands, board);
    let on = board_rows(&agent_on_rows(&s.data.cards, &own, card), board);
    let mut all = own;
    all.extend(on);
    all
}

/// The # picker's rows: the window's live cards, with their status.
fn card_candidates(s: &State) -> Vec<(CardCandidate, RefStatus)> {
    s.data
        .cards
        .iter()
        .filter(|c| !c.short.is_empty())
        .map(|c| {
            (
                CardCandidate {
                    id: c.id.clone(),
                    r#ref: format!("{}#{}", c.tag, c.short),
                    short: c.short.clone(),
                    title: plain_line(&c.title),
                },
                card_status(c),
            )
        })
        .collect()
}

/// The picker as the box stands: which token, which list, which rows.
struct Popup {
    token: Option<Token>,
    for_cards: bool,
    open: bool,
    agents: Vec<Candidate>,
    cards: Vec<(CardCandidate, RefStatus)>,
    /// "start:query": the highlight belongs to one token and query.
    at: String,
    highlight: i32,
    /// At the cap a new agent can't be added; one already named can.
    capped: bool,
    taken: Vec<String>,
}

impl Popup {
    fn rows(&self) -> usize {
        if self.for_cards { self.cards.len() } else { self.agents.len() }
    }
    /// The row Enter picks: the highlighted one, else the first.
    fn shown(&self) -> i32 {
        if self.rows() == 0 {
            -1
        } else if self.highlight < 0 {
            0
        } else {
            self.highlight
        }
    }
}

fn popup(s: &State, c: &Composer, board: &str, card: &str, no_mentions: bool) -> Popup {
    let caret = c.edit.range().1;
    let text = &c.edit.text;
    let all = candidates(s, board, card);
    let names: Vec<String> = all.iter().map(row_name).collect();
    let focused = c.edit.focused;
    let agent = if focused && !no_mentions { token_at(text, caret, &names) } else { None };
    let cc = if focused && agent.is_none() && !no_mentions { card_candidates(s) } else { Vec::new() };
    let card_tok = if agent.is_none() && !cc.is_empty() {
        let titles: Vec<String> = cc.iter().map(|(c, _)| c.title.clone()).collect();
        card_token_at(text, caret, &titles)
    } else {
        None
    };
    let for_cards = agent.is_none() && card_tok.is_some();
    let token = agent.or(card_tok);
    let open = token.as_ref().is_some_and(|t| Some(t.start) != c.dismissed);
    let mut p = Popup {
        at: token.as_ref().map(|t| format!("{}:{}", t.start, t.query)).unwrap_or_default(),
        token,
        for_cards,
        open,
        agents: Vec::new(),
        cards: Vec::new(),
        highlight: -1,
        capped: false,
        taken: Vec::new(),
    };
    let kept = keep_mentions(&c.mentions, text);
    p.capped = is_full(&kept);
    p.taken = kept.into_iter().map(|m| m.key).collect();
    if open {
        let q = &p.token.as_ref().expect("open has a token").query;
        if for_cards {
            let rows = filter_cards(&cc.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>(), q);
            p.cards = rows
                .into_iter()
                .filter_map(|r| cc.iter().find(|(c, _)| c.id == r.id).cloned())
                .collect();
        } else {
            p.agents = filter_candidates(&all, q);
        }
    }
    p.highlight = if c.hi.0 == p.at { c.hi.1 } else { -1 };
    p
}

/// The key's DOM name, as lib/mentions.ts `menuKey` reads it.
fn key_name(k: &KeyEvent) -> &'static str {
    match k.key {
        Key::Tab => "Tab",
        Key::Down => "ArrowDown",
        Key::Up => "ArrowUp",
        Key::Enter => "Enter",
        Key::Escape => "Escape",
        _ => "",
    }
}

/// Puts the picked row in the text (as an edit, so undo takes it back).
fn pick(s: &mut State, get: Get, get_mut: GetMut, board: &str, card: &str, i: usize) {
    let p = popup(s, get(s), board, card, false);
    let Some(t) = p.token.clone() else { return };
    let c = get_mut(s);
    let caret = c.edit.range().1;
    let value = c.edit.text.clone();
    let (next_text, next_caret, mentions) = if p.for_cards {
        let Some((row, _)) = p.cards.get(i) else { return };
        let (nt, nc) = insert_card_ref(&value, &t, caret, &row.r#ref);
        (nt, nc, keep_mentions(&c.mentions, &value))
    } else {
        let Some(row) = p.agents.get(i) else { return };
        let kept = keep_mentions(&c.mentions, &value);
        if is_full(&kept) && !kept.iter().any(|m| m.key == row.who.key) {
            return;
        }
        let (nt, nc) = insert_mention(&value, &t, caret, &row.who.name);
        let m = Mention {
            key: row.who.key.clone(),
            name: row.who.name.clone(),
            gone: false,
        };
        let ms = add_mention(&keep_mentions(&kept, &nt), m);
        (nt, nc, ms)
    };
    // The replaced span: the token up to the end of the word at the caret.
    let mut end = caret;
    while let Some(ch) = value[end..].chars().next() {
        if after(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    let ins = &next_text[t.start..next_text.len() - (value.len() - end)];
    c.edit.set_selection(t.start, end);
    c.edit.insert(ins);
    if c.edit.text != next_text {
        c.edit.replace_text(next_text.clone());
    }
    c.edit.set_caret(next_caret);
    c.last = next_text.clone();
    c.mentions = mentions;
    // A name that another one starts with (oak, oak tree) would leave the
    // list open on the new text: it is done.
    c.dismissed = Some(t.start);
    let (key, ms) = (c.key.clone(), c.mentions.clone());
    super::drafts::set_draft(s, &key, &next_text, &ms);
}

/// After each edit: a space or closing punctuation typed after an exact,
/// unique name counts as picking it; a paste (or an undo bringing a
/// mention back) records each exact name in what came in; mentions whose
/// "@name" left the text go.
fn changed(s: &mut State, get_mut: GetMut, board: &str, card: &str, no_mentions: bool) {
    let cands = candidates(s, board, card);
    let names: Vec<String> = cands.iter().map(row_name).collect();
    let c = get_mut(s);
    let v = c.edit.text.clone();
    let at = c.edit.range().1;
    let value = std::mem::take(&mut c.last);
    let mut next = c.mentions.clone();
    let typed_ch = v[..at.min(v.len())].chars().last();
    let typed = typed_ch.is_some_and(|ch| {
        let p = at - ch.len_utf8();
        v.len() == value.len() + ch.len_utf8() && format!("{}{}", &v[..p], &v[at..]) == value
    });
    if !no_mentions && typed && typed_ch.is_some_and(after) {
        let p = at - typed_ch.map_or(0, char::len_utf8);
        let settled = settle(&value, p, &next, &cands);
        if settled != next
            && let Some(t) = token_at(&value, p, &names)
        {
            c.dismissed = Some(t.start);
        }
        next = settled;
    }
    if !no_mentions && !typed && v.len() > value.len() {
        let (a, b) = (v.as_bytes(), value.as_bytes());
        let mut from = 0;
        while from < b.len() && a[from] == b[from] {
            from += 1;
        }
        while !v.is_char_boundary(from) {
            from -= 1;
        }
        let tail = b.len() - from;
        let mut back = 0;
        while back < tail && a[a.len() - 1 - back] == b[b.len() - 1 - back] {
            back += 1;
        }
        let start = v[..(from + 1).min(v.len())].rfind('@').unwrap_or(0);
        next = settle_range(&v, start, v.len() - back, &next, &cands);
    }
    c.mentions = keep_mentions(&next, &v);
    c.last = v.clone();
    // A dismissal lasts while its token does.
    let tok = token_at(&v, at, &names).or_else(|| card_token_at(&v, at, &[]));
    if c.dismissed.is_some() && tok.map(|t| t.start) != c.dismissed {
        c.dismissed = None;
    }
    c.refused = None;
    let (key, ms) = (c.key.clone(), c.mentions.clone());
    super::drafts::set_draft(s, &key, &v, &ms);
}

/// Enter or Send: an exact, unique name at the caret counts as picked;
/// then the site's `on_send` gets the text, mentions and files.
fn send(s: &mut State, get_mut: GetMut, board: &str, card: &str, no_mentions: bool, on_send: &Option<SendFn>) {
    let Some(on_send) = on_send.clone() else { return };
    let cands = candidates(s, board, card);
    let c = get_mut(s);
    if c.sending {
        return;
    }
    let caret = c.edit.range().1;
    if !no_mentions {
        c.mentions = settle(&c.edit.text, caret, &c.mentions, &cands);
    }
    let t = c.edit.text.trim().to_string();
    if t.is_empty() && c.files.is_empty() {
        return;
    }
    let sent = Sent {
        text: t.clone(),
        mentions: keep_mentions(&c.mentions, &c.edit.text),
        files: c.files.iter().map(Pending::upload).collect(),
    };
    on_send(s, sent);
}

/// Files picked, dropped or pasted: read off the UI thread, then added.
fn add_paths(s: &mut State, get_mut: GetMut, paths: Vec<std::path::PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let first = get_mut(s).file_id(paths.len());
    s.spawn(
        move |_| attach::read_files(&paths, first),
        move |s, got| get_mut(s).add_files(got),
    );
}

/// The composer: the box with its pickers, files, clip, mic and Send.
/// `key` names it on the page (prefix it with your package's name).
pub fn composer(s: &State, f: &mut Frame, key: &str, get: Get, get_mut: GetMut, o: &ComposerOpts) -> El {
    let k = f.k;
    let c = get(s);
    let board = o.board.clone();
    let card = o.card.clone();
    let no_m = o.no_mentions.is_some();
    let p = popup(s, c, &board, &card, no_m || o.disabled);
    let box_key = format!("{key}/box");

    // Each recorded "@name" in the box tinted as its chip will be.
    let mut deco = Vec::new();
    let mut at = 0;
    for m in split_mentions(&c.edit.text, &c.mentions) {
        match m {
            mentions::MentionSpan::Text(t) => at += t.len(),
            mentions::MentionSpan::Mention(t, _) => {
                deco.push(Span::new(at, at + t.len()).color(k.c.text_accent).weight(560.0));
                at += t.len();
            }
        }
    }

    let style = rune::tok::TextStyle {
        line_height: (13.0f32 * 1.35 * 2.0).round() / 2.0,
        ..k.ty.ui
    };
    let (b1, c1) = (board.clone(), card.clone());
    let (b2, c2) = (board.clone(), card.clone());
    let (b3, c3) = (board.clone(), card.clone());
    let on_send = o.on_send.clone();
    let no_files = o.no_files.clone();
    let hide_files = o.hide_files;
    let mut field = text_input(&c.edit, style)
        .key(format!("{key}/input"))
        .w_full()
        .color(k.c.text_base)
        .placeholder(o.placeholder.clone(), k.c.text_faint)
        .caret(k.c.text_base, k.selection())
        .label(if o.label.is_empty() { o.placeholder.clone() } else { o.label.clone() })
        .decorations(deco);
    if let Some(why) = &o.no_mentions {
        field = field.help(why.clone());
    }
    if !o.disabled {
        field = field
            .on_edit(rune::edit_access(move |s: &mut State| &mut get_mut(s).edit))
            .on_input_change(move |s: &mut State, _tx: &mut Tx| changed(s, get_mut, &b1, &c1, no_m))
            .on_input_key(move |s: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
                let p = popup(s, get(s), &b2, &c2, no_m);
                let Some(t) = &p.token else { return false };
                match menu_key(key_name(ev), ev.mods.shift, false, p.open, p.rows()) {
                    Some(MenuKey::Next | MenuKey::Prev) => {
                        let dir = if menu_key(key_name(ev), ev.mods.shift, false, true, 1) == Some(MenuKey::Next) { 1 } else { -1 };
                        let i = move_highlight(p.highlight, p.rows(), dir);
                        get_mut(s).hi = (p.at.clone(), i);
                        true
                    }
                    Some(MenuKey::Pick) => {
                        let i = p.shown().max(0) as usize;
                        pick(s, get, get_mut, &b2, &c2, i);
                        true
                    }
                    Some(MenuKey::Hold) => true,
                    Some(MenuKey::Close) => {
                        get_mut(s).dismissed = Some(t.start);
                        true
                    }
                    None => false,
                }
            })
            .on_submit({
                let on_send = on_send.clone();
                let (b, c) = (board.clone(), card.clone());
                move |s: &mut State, _tx: &mut Tx| send(s, get_mut, &b, &c, no_m, &on_send)
            })
            .on_input_paste(move |s: &mut State, content: &rune::ClipboardContent, _tx: &mut Tx| {
                if hide_files {
                    return false;
                }
                let has_files = content.image().is_some() || !content.files().is_empty();
                if !has_files {
                    return false;
                }
                if let Some(why) = &no_files {
                    get_mut(s).refused = Some(why.clone());
                    return true;
                }
                if !content.files().is_empty() {
                    add_paths(s, get_mut, content.files().to_vec());
                    return true;
                }
                if let Some(img) = content.image() {
                    let c = get_mut(s);
                    let id = c.file_id(1);
                    c.add_files(vec![attach::pending("image.png", img.png().to_vec(), id)]);
                }
                true
            });
    }
    let (min_rows, max_rows) = o.rows.unwrap_or((1, 8));
    let mut boxed = col()
        .key(box_key.clone())
        .grow(1.0)
        .min_w(0.0)
        .px(10.0)
        .py(8.0)
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .border(1.0, if c.edit.focused { k.c.border_focus } else { k.c.border_base })
        .child(rune::auto_grow(field, min_rows, max_rows));
    if o.disabled {
        boxed = boxed.layer_opacity(0.5);
    } else if !o.hide_files && o.no_files.is_none() {
        boxed = boxed.on_drop_files(move |s: &mut State, paths: &[std::path::PathBuf], _tx: &mut Tx| {
            add_paths(s, get_mut, paths.to_vec());
        });
    }

    let mut line = row().key(format!("{key}/row")).w_full().gap(4.0).items_end().child(boxed);
    if !o.hide_files {
        let pick_file: Arc<dyn Fn(&mut State, &mut Tx)> = Arc::new(move |_s: &mut State, tx: &mut Tx| {
            let opts = FileDialogOptions {
                title: "Add files".into(),
                ..FileDialogOptions::default()
            };
            tx.open_file(opts, move |s: &mut State, r, _tx| match r {
                Ok(Some(path)) => add_paths(s, get_mut, vec![path]),
                Ok(None) => {}
                Err(e) => get_mut(s).refused = Some(format!("Couldn't open the file picker: {e}")),
            });
        });
        let full = c.files.len() >= MAX_UPLOAD_FILES || c.sending || o.disabled;
        line = line.child(
            el().pb(4.0)
                .child(attach::attach_button(f, &format!("{key}/clip"), o.no_files.as_deref(), full, pick_file)),
        );
    }
    if let Some(mic) = &o.mic {
        line = line.child(el().pb(4.0).child(mic.clone()));
    }
    for e in &o.extra {
        line = line.child(el().pb(4.0).child(e.clone()));
    }
    if o.send_button {
        let on_send = o.on_send.clone();
        let (b, cd) = (b3.clone(), c3.clone());
        line = line.child(
            el().pb(4.0).child(
                IconButton::new(&icons::SEND)
                    .variant(IconButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .label("Send")
                    .help("Send (Enter)")
                    .disabled(o.disabled || c.sending || c.is_empty() || o.on_send.is_none())
                    .on_press(move |s: &mut State| send(s, get_mut, &b, &cd, no_m, &on_send))
                    .build(k)
                    .key(format!("{key}/send")),
            ),
        );
    }

    let mut out = col().key(key.to_string()).w_full().min_w(0.0).gap(6.0);
    let remove: Arc<dyn Fn(&mut State, u64)> = Arc::new(move |s: &mut State, id: u64| {
        get_mut(s).files.retain(|p| p.id != id);
    });
    if let Some(files) = attach::pending_files(
        f,
        &format!("{key}/files"),
        &c.files,
        c.refused.as_deref(),
        c.sending || o.disabled,
        remove,
    ) {
        out = out.child(files);
    }
    out = out.child(line);
    let missed = !c.missed.is_empty()
        && c.missed_at.is_some_and(|t| t.elapsed() < Duration::from_millis(mentions::MISSED_MS));
    if missed {
        out = out.child(
            text(c.missed.clone())
                .key(format!("{key}/missed"))
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .px(4.0)
                .wrap(),
        );
    } else if o.asking {
        let line = mentions::asking_line(&keep_mentions(&c.mentions, &c.edit.text));
        if !line.is_empty() {
            out = out.child(
                text(line)
                    .key(format!("{key}/asking"))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .px(4.0),
            );
        }
    }

    if p.open && !o.disabled {
        picker(s, f, key, &box_key, get, get_mut, &p, o);
    }
    out
}

/// The @ or # list, anchored to the box.
#[allow(clippy::too_many_arguments)]
fn picker(s: &State, f: &mut Frame, key: &str, box_key: &str, get: Get, get_mut: GetMut, p: &Popup, o: &ComposerOpts) {
    let k = f.k;
    let _ = s;
    let query = p.token.as_ref().map(|t| plain_line(&t.query)).unwrap_or_default();
    let mut items = Vec::new();
    let empty;
    if p.for_cards {
        for (i, (c, st)) in p.cards.iter().enumerate() {
            let mut d = st.label.to_string();
            if let Some((a, b)) = st.progress {
                d = format!("{d} · {a}/{b}");
            }
            items.push(
                PopupItem::new(i.to_string(), c.title.clone())
                    .leading(
                        row()
                            .gap(6.0)
                            .center_y()
                            .child(el().size(6.0, 6.0).radius(3.0).fill(tone_color(st.tone)))
                            .child(text(c.r#ref.clone()).font(k.ty.small.mono()).color(k.c.text_muted)),
                    )
                    .description(d),
            );
        }
        empty = if query.is_empty() { "No cards".to_string() } else { format!("No card matches {query}") };
    } else {
        for (i, r) in p.agents.iter().enumerate() {
            items.push(agent_item(f, i, r, p.capped && !p.taken.contains(&r.who.key)));
        }
        empty = if query.is_empty() {
            "No agents to mention".to_string()
        } else {
            format!("No agent named {query}")
        };
    }
    let board = o.board.clone();
    let card = o.card.clone();
    let at = p.at.clone();
    let mut list = PopupList::new(format!("{key}-picker"), box_key.to_string())
        .side(if o.popup_below { Side::Bottom } else { Side::Top })
        .align(Align::Start)
        .offset(4.0)
        .width(320.0)
        .max_height(288.0)
        .items(items)
        .empty_message(empty)
        .label(if p.for_cards { "Cards to mention" } else { "Agents to mention" })
        .on_active(move |s: &mut State, i| get_mut(s).hi = (at.clone(), i as i32))
        .on_choose(move |s: &mut State, id: &str, _tx: &mut Tx| {
            if let Ok(i) = id.parse::<usize>() {
                pick(s, get, get_mut, &board, &card, i);
            }
        });
    if p.shown() >= 0 {
        list = list.active(Some(p.shown() as usize));
    }
    list.build(k, f.layer);
}

/// One agent in the picker: its initial in its device colour, its name,
/// device, status and repo, so two agents with one name tell apart. A gone
/// one says so; past the cap a new one can't be picked.
fn agent_item(f: &Frame, i: usize, r: &Candidate, disabled: bool) -> PopupItem {
    let k = f.k;
    let who = &r.who;
    let gone = is_gone(who);
    let tint = device_color(&who.color).unwrap_or(k.c.text_accent);
    let initial: String = plain_line(&who.name).chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let face = el()
        .size(18.0, 18.0)
        .radius(9.0)
        .fill(tint.alpha(if gone { 0.12 } else { 0.22 }))
        .center()
        .child(text(initial).font(k.ty.text_xsmall_medium).color(if gone { k.c.text_faint } else { tint }));
    let name = if r.card.is_empty() { plain_line(&who.name) } else { mentions::agent_on_label(&r.card) };
    let place = if r.repo.is_empty() {
        r.cwd.rsplit(['/', '\\']).find(|x| !x.is_empty()).unwrap_or("").to_string()
    } else {
        r.repo.clone()
    };
    let status = status_text(who);
    let desc = if gone {
        "gone".to_string()
    } else if !r.card.is_empty() {
        [plain_line(&who.name), status].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")
    } else {
        let mut parts = Vec::new();
        if !who.where_.is_empty() {
            parts.push(plain_line(&who.where_));
        }
        for x in [status, plain_line(&place)] {
            if !x.is_empty() {
                parts.push(x);
            }
        }
        parts.join(" · ")
    };
    PopupItem::new(i.to_string(), name).leading(face).description(desc).disabled(disabled)
}

/// An agent's state in a word (lib/agent-status.ts, cut down).
fn status_text(who: &irori_window::cards::AgentRef) -> String {
    if who.offline {
        return "offline".into();
    }
    match who.status.as_str() {
        "" | "person" => String::new(),
        s => s.replace('_', " "),
    }
}
