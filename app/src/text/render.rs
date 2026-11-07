//! Rich text on screen (the React app's common/rich-text.tsx): untrusted
//! multi-line text with its links, code, lists, mentions and card refs
//! usable. One Rune text element per paragraph or list item, its pieces as
//! spans, links and pills over byte ranges; one `on_link` per element reads
//! what was pressed from the link's id ("url:", "copy:", "card:").
//!
//! The pieces as Rune draws them:
//! - a link: what the agent wrote, its host in full weight and the rest
//!   dimmed, underlined; it opens in the person's browser (`tx.open_url`);
//! - `code`: mono on a muted ground; a click copies it (a copy glyph sits
//!   before it: Rune can't place a button inside a run of text);
//! - "@name" a mention names: a chip in the agent's device colour, struck
//!   through when it is gone;
//! - "#12": a chip with a dot in the card's status colour, the ref, its
//!   title cut short and its steps; a click opens the card.

use std::rc::Rc;

use irori_window::cards::CardMention;
use rune::tok::TextStyle;
use rune::{Color, El, InlineIcon, Link, LinkPaint, PillStyle, Span, Tx, col, el, rich, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};

use super::icons;
use super::mentions::{MentionSpan, Named, card_refs, is_gone, split_mentions};
use super::refs::{self, device_color};
use super::rich::{Block, Inline, LinkPiece, plain_line, rich_blocks, rich_inline, safe_href};
use crate::ui::{Frame, State};
use crate::widgets::bits::tone_solid;

impl Named for CardMention {
    fn name(&self) -> &str {
        &self.name
    }
}

/// How rich text draws: its type and colour, the mentions its "@name"s
/// name, and whether its links and code respond (false where it sits inside
/// a button: they then only look the part).
#[derive(Clone)]
pub struct RichOpts<'a> {
    pub font: Option<TextStyle>,
    pub color: Option<Color>,
    pub mentions: &'a [CardMention],
    pub interactive: bool,
    /// Lines past this many end in "…" (0: all).
    pub max_lines: usize,
    /// Let the person select and copy it (true by default).
    pub selectable: bool,
}

impl Default for RichOpts<'_> {
    fn default() -> Self {
        RichOpts {
            font: None,
            color: None,
            mentions: &[],
            interactive: true,
            max_lines: 0,
            selectable: true,
        }
    }
}

/// Multi-line rich text: paragraphs and lists, selectable, its links and
/// card refs live. `key` must be unique on the page (prefix it with your
/// package's name).
pub fn rich_text(s: &State, f: &mut Frame, key: &str, src: &str) -> El {
    rich_with(s, f, key, src, &RichOpts::default())
}

/// [`rich_text`] with options (mentions, type, colour, not interactive).
pub fn rich_with(s: &State, f: &mut Frame, key: &str, src: &str, o: &RichOpts) -> El {
    let k = f.k;
    let blocks = rich_blocks(src);
    let mut out = col().key(key.to_string()).w_full().min_w(0.0).gap(6.0);
    if o.selectable {
        out = out.selection_group(true);
    }
    let font = o.font.unwrap_or(k.ty.ui);
    for (i, b) in blocks.iter().enumerate() {
        match b {
            Block::Para(lines) => {
                let mut run = Run::default();
                for (j, l) in lines.iter().enumerate() {
                    if j > 0 {
                        run.push("\n", None);
                    }
                    pieces(s, f, &mut run, l, o, font);
                }
                out = out.child(run.build(f, &format!("{key}/p{i}"), o, font));
            }
            Block::List(items) => {
                let mut list = col().key(format!("{key}/l{i}")).w_full().gap(2.0);
                for (j, it) in items.iter().enumerate() {
                    let marker = if it.marker == "*" || it.marker == "-" {
                        "•".to_string()
                    } else {
                        it.marker.clone()
                    };
                    let mut run = Run::default();
                    pieces(s, f, &mut run, &it.inline, o, font);
                    list = list.child(
                        row()
                            .key(format!("{key}/l{i}/{j}"))
                            .w_full()
                            .gap(6.0)
                            .child(text(marker).font(font).color(k.c.text_muted))
                            .child(
                                run.build(f, &format!("{key}/l{i}/{j}/t"), o, font)
                                    .grow(1.0)
                                    .min_w(0.0),
                            ),
                    );
                }
                out = out.child(list);
            }
        }
    }
    out
}

/// One line of rich text (a question, an answer, a note): its links, code,
/// refs and mentions, no paragraphs or lists.
pub fn rich_line(s: &State, f: &mut Frame, key: &str, src: &str, o: &RichOpts) -> El {
    let font = o.font.unwrap_or(f.k.ty.ui);
    let mut run = Run::default();
    pieces(s, f, &mut run, &rich_inline(src), o, font);
    run.build(f, key, o, font)
}

/// A text run being built: its content and what styles its byte ranges.
#[derive(Default)]
struct Run {
    s: String,
    spans: Vec<Span>,
    links: Vec<Link>,
    pills: Vec<(std::ops::Range<usize>, PillStyle)>,
    strikes: Vec<(usize, usize, Option<Color>)>,
    icons: Vec<(usize, InlineIcon)>,
    /// Card refs it couldn't resolve: looked up once it shows.
    lookup: Vec<String>,
}

impl Run {
    /// Appends `t`, with `span` (its range filled in) over it.
    fn push(&mut self, t: &str, span: Option<Span>) -> std::ops::Range<usize> {
        let start = self.s.len();
        self.s.push_str(t);
        let r = start..self.s.len();
        if let Some(mut sp) = span {
            sp.start = r.start as u32;
            sp.end = r.end as u32;
            self.spans.push(sp);
        }
        r
    }

    fn build(self, f: &Frame, key: &str, o: &RichOpts, font: TextStyle) -> El {
        let k = f.k;
        let mut e = rich(self.s, self.spans)
            .key(key.to_string())
            .font(font)
            .color(o.color.unwrap_or(k.c.text_base))
            .w_full()
            .min_w(0.0)
            .wrap_anywhere();
        if o.max_lines > 0 {
            e = e.max_lines(o.max_lines);
        }
        for l in self.links {
            e = e.link(l);
        }
        if !self.pills.is_empty() {
            e = e.pills(self.pills);
        }
        if !self.strikes.is_empty() {
            e = e.strikes(self.strikes);
        }
        for (at, i) in self.icons {
            e = e.inline_icon(at, i);
        }
        if o.interactive {
            e = e.on_link(on_link);
        }
        if o.selectable {
            e = e.selectable(true);
        }
        if !self.lookup.is_empty() && o.interactive {
            let want = Rc::new(self.lookup);
            e = e.on_visible(move |s: &mut State, on, _| {
                if on {
                    refs::prefetch_refs(s, want.as_ref().clone());
                }
            });
        }
        e
    }
}

/// What a press on a link in rich text does, by its id.
pub fn on_link(s: &mut State, id: &str, tx: &mut Tx) {
    if let Some(href) = id.strip_prefix("url:") {
        // Checked again: only ever http(s) leaves the app.
        if let Some(h) = safe_href(href)
            && let Err(e) = tx.open_url(&h)
        {
            s.error("Couldn't open the link", &e.to_string());
        }
    } else if let Some(t) = id.strip_prefix("copy:") {
        tx.copy(t.to_string());
        COPIED.with(|c| *c.borrow_mut() = Some((id.to_string(), std::time::Instant::now())));
        tx.wake_after(COPIED_FOR + std::time::Duration::from_millis(20));
    } else if let Some(card) = id.strip_prefix("card:") {
        s.open_card(card);
    }
}

fn pieces(s: &State, f: &Frame, run: &mut Run, ps: &[Inline], o: &RichOpts, font: TextStyle) {
    let k = f.k;
    for p in ps {
        match p {
            Inline::Text(t) => {
                if o.mentions.is_empty() {
                    card_ref_text(s, f, run, t, o);
                    continue;
                }
                for m in split_mentions(t, o.mentions) {
                    match m {
                        MentionSpan::Text(t) => card_ref_text(s, f, run, &t, o),
                        MentionSpan::Mention(t, m) => mention_chip(f, run, &t, m),
                    }
                }
            }
            Inline::Code(t) => {
                let at = run.s.len();
                let r = run.push(
                    t,
                    Some(
                        Span::new(0, 0)
                            .mono()
                            .size(font.size * 0.92)
                            .bg(k.c.layer_03, 2.0)
                            .color(k.c.text_strong),
                    ),
                );
                if o.interactive {
                    let id = format!("copy:{t}");
                    let done = copied(&id);
                    let (glyph, ink) = if done {
                        (&crate::ui::icons::CHECK, k.c.success.fg)
                    } else {
                        (&icons::COPY, k.c.text_faint)
                    };
                    run.icons.push((
                        at,
                        InlineIcon::glyph(glyph)
                            .size((font.size * 0.85).round())
                            .gap(3.0)
                            .color(ink),
                    ));
                    let tip = if done {
                        "Copied".to_string()
                    } else {
                        format!("Copy: {t}")
                    };
                    run.links.push(
                        Link::new(r, id)
                            .role("button")
                            .label(if done {
                                "Copied".to_string()
                            } else {
                                format!("Copy {t}")
                            })
                            .help(tip)
                            .hover(LinkPaint::new().bg(k.c.hover)),
                    );
                }
            }
            Inline::Link(l) => link_piece(f, run, l, o),
        }
    }
}

fn link_piece(f: &Frame, run: &mut Run, l: &LinkPiece, o: &RichOpts) {
    let k = f.k;
    let accent = k.c.text_accent;
    let dim = Span::new(0, 0)
        .color(accent.alpha(0.78))
        .underline(accent.alpha(0.4));
    let start = run.s.len();
    run.push(&l.pre, Some(dim.clone()));
    run.push(
        &l.host,
        Some(
            Span::new(0, 0)
                .color(accent)
                .bold()
                .underline(accent.alpha(0.4)),
        ),
    );
    run.push(&l.rest, Some(dim));
    if !l.shown_host.is_empty() {
        run.push(" (", None);
        run.push(&l.shown_host, Some(Span::new(0, 0).color(accent).bold()));
        run.push(")", None);
    }
    if o.interactive {
        run.links.push(
            Link::new(start..run.s.len(), format!("url:{}", l.href))
                .label(format!("Open {} in your browser", l.href))
                .help(format!("Open {} in your browser", l.href))
                .hover(LinkPaint::new().underline(accent)),
        );
    }
}

/// Text with each card ref ("#12", "mlg#12") made a chip: a card the
/// window holds at once, any other once the backend has looked it up. A
/// ref nothing has stays text.
fn card_ref_text(s: &State, f: &Frame, run: &mut Run, t: &str, o: &RichOpts) {
    if !t.contains('#') {
        run.push(t, None);
        return;
    }
    let mut last = 0;
    for r in card_refs(t) {
        let label = &t[r.start..r.end];
        let Some(hit) = refs::resolve(s, &r) else {
            if o.interactive {
                let rt = super::mentions::ref_text(&r);
                if !s.text.refs.contains_key(&rt) && !run.lookup.contains(&rt) {
                    run.lookup.push(rt);
                }
            }
            continue;
        };
        run.push(&t[last..r.start], None);
        ref_chip(f, run, label, &hit, o);
        last = r.end;
    }
    run.push(&t[last..], None);
}

/// A card ref chip (card #343): a dot in the card's status colour, the
/// ref, its title cut short and its steps; the whole title, its state and
/// steps in its tooltip.
fn ref_chip(f: &Frame, run: &mut Run, label: &str, hit: &refs::Resolved, o: &RichOpts) {
    let k = f.k;
    let st = &hit.status;
    let name = plain_line(&hit.title);
    let steps = st
        .progress
        .map(|(d, t)| format!("{d}/{t}"))
        .unwrap_or_default();
    let tip = [
        name.as_str(),
        st.label,
        &if steps.is_empty() {
            String::new()
        } else {
            format!("{steps} steps")
        },
    ]
    .iter()
    .filter(|x| !x.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" · ");
    let start = run.s.len();
    run.push(
        "●\u{2009}",
        Some(Span::new(0, 0).color(tone_solid(f, st.tone)).size(8.0)),
    );
    run.push(
        label,
        Some(Span::new(0, 0).color(k.c.text_accent).weight(560.0)),
    );
    if !name.is_empty() {
        run.push(" ", None);
        let short = cut(&name, 22);
        let c = if st.closed() {
            k.c.text_muted
        } else {
            k.c.text_base.alpha(0.85)
        };
        run.push(&short, Some(Span::new(0, 0).color(c)));
    }
    if !steps.is_empty() {
        run.push(" ", None);
        run.push(
            &steps,
            Some(Span::new(0, 0).color(k.c.text_muted).size(11.0)),
        );
    }
    let r = start..run.s.len();
    run.pills
        .push((r.clone(), PillStyle::new().bg(k.c.text_accent.alpha(0.1))));
    if o.interactive {
        run.links.push(
            Link::new(r, format!("card:{}", hit.id))
                .role("button")
                .label(format!("{label} {tip}"))
                .help(tip)
                .hover(LinkPaint::new().bg(k.c.text_accent.alpha(0.2))),
        );
    }
}

/// One "@name" a mention names: in the agent's device colour; gone (stored
/// gone, or gone now) is greyed and struck through.
fn mention_chip(f: &Frame, run: &mut Run, t: &str, m: &CardMention) {
    let k = f.k;
    let gone = m.gone || is_gone(&m.who);
    if gone {
        let r = run.push(
            t,
            Some(
                Span::new(0, 0)
                    .color(k.c.text_muted.alpha(0.7))
                    .weight(560.0),
            ),
        );
        run.strikes.push((r.start, r.end, None));
        let help = format!("{} is gone; not delivered", plain_line(&m.name));
        run.links
            .push(Link::new(r, format!("mention:{}", m.who.key)).help(help));
        return;
    }
    let tint = device_color(&m.who.color).unwrap_or(k.c.text_accent);
    let r = run.push(t, Some(Span::new(0, 0).color(tint).weight(560.0)));
    run.pills
        .push((r.clone(), PillStyle::new().bg(tint.alpha(0.14))));
    let mut help = plain_line(if m.who.name.is_empty() {
        &m.name
    } else {
        &m.who.name
    });
    if !m.who.where_.is_empty() {
        help = format!("{help} · {}", plain_line(&m.who.where_));
    }
    run.links
        .push(Link::new(r, format!("mention:{}", m.who.key)).help(help));
}

/// `s` cut to `n` characters, with "…" when cut.
pub fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

/// `code` on its own (outside rich text), with a Copy beside it.
pub fn code(f: &Frame, key: &str, t: &str) -> El {
    let k = f.k;
    row()
        .key(key.to_string())
        .gap(2.0)
        .center_y()
        .min_w(0.0)
        .child(
            el().px(4.0)
                .py(1.0)
                .radius(3.0)
                .fill(k.c.layer_03)
                .min_w(0.0)
                .child(
                    text(t.to_string())
                        .font(k.ty.mono)
                        .color(k.c.text_strong)
                        .wrap_anywhere()
                        .selectable(true),
                ),
        )
        .child(copy_button(f, &format!("{key}/copy"), t, "Copy"))
}

thread_local! {
    /// The Copy button pressed last and when: it shows a check for a
    /// moment (rich-text.tsx CopyButton's `done`).
    static COPIED: std::cell::RefCell<Option<(String, std::time::Instant)>> = const { std::cell::RefCell::new(None) };
}

/// How long a Copy shows its check.
const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1500);

/// Whether the Copy with this id was pressed a moment ago.
fn copied(id: &str) -> bool {
    COPIED.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|(k, at)| k == id && at.elapsed() < COPIED_FOR)
    })
}

/// A one-click Copy for `t`: copies it and shows a check for a moment.
pub fn copy_button(f: &Frame, key: &str, t: &str, label: &str) -> El {
    let v = t.to_string();
    let id = key.to_string();
    let done = copied(&id);
    IconButton::new(if done {
        &crate::ui::icons::CHECK
    } else {
        &icons::COPY
    })
    .variant(IconButtonVariant::GhostMuted)
    .control_size(ControlSize::Mini)
    .label(if done {
        "Copied".to_string()
    } else {
        format!("{label} {t}")
    })
    .help(if done {
        "Copied".to_string()
    } else {
        format!("{label}: {t}")
    })
    .on_press_tx(move |_s: &mut State, tx: &mut Tx| {
        tx.copy(v.clone());
        COPIED.with(|c| *c.borrow_mut() = Some((id.clone(), std::time::Instant::now())));
        // Back to the copy glyph once the moment is over.
        tx.wake_after(COPIED_FOR + std::time::Duration::from_millis(20));
    })
    .build(f.k)
    .key(key.to_string())
}

/// A link an agent put on a blocker or an option (`kind` "url", "path" or
/// "commit"): a URL opens in the person's browser (its label and host, the
/// full URL on hover); a repo path or commit can't be opened from here, so
/// it shows in full with a Copy.
pub fn link_chip(f: &Frame, key: &str, kind: &str, reference: &str, label: &str) -> El {
    let k = f.k;
    let r = plain_line(reference);
    let href = if kind == "url" { safe_href(&r) } else { None };
    let chip = row()
        .key(key.to_string())
        .h(24.0)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .radius(4.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.surface_raised_base);
    if let Some(href) = href {
        let host = super::rich::host_of(&href);
        let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
        let shown = {
            let l = plain_line(label);
            if l.is_empty() { host.clone() } else { l }
        };
        let mut c = chip.px(8.0).child(
            text(shown.clone())
                .font(k.ty.small_medium)
                .color(k.c.text_base)
                .truncate()
                .shrink(),
        );
        if shown != host {
            c = c.child(
                text(host)
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .truncate()
                    .shrink(),
            );
        }
        let open = r.clone();
        return c
            .child(rune::icon(&icons::EXTERNAL_LINK, 12.0).color(k.c.icon_muted))
            .help(format!("Open {r} in your browser"))
            .cursor(rune::Cursor::Pointer)
            .hover(k.c.hover)
            .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                if let Some(h) = safe_href(&open)
                    && let Err(e) = tx.open_url(&h)
                {
                    s.error("Couldn't open the link", &e.to_string());
                }
            });
    }
    let what = match kind {
        "commit" => format!("Commit {r}"),
        "url" => format!("Link {r} (not opened from here)"),
        _ => format!("Repo path {r}"),
    };
    chip.pl(8.0)
        .pr(2.0)
        .help(what)
        .child(
            text(r.clone())
                .font(k.ty.small.mono())
                .color(k.c.text_base)
                .truncate()
                .shrink(),
        )
        .child(copy_button(f, &format!("{key}/copy"), &r, "Copy"))
}

/// A card's state as a dot in its status colour (card #343).
pub fn ref_dot(f: &Frame, st: &refs::RefStatus) -> El {
    el().size(6.0, 6.0).radius(3.0).fill(tone_solid(f, st.tone))
}
