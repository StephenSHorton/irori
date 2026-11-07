//! A card's small pieces (the React app's cards/bits.tsx): the tone badge
//! every chip is drawn with, the state pill, progress bar, holder and ask
//! lines, scope, side, PR and owner chips, the short ref, the working
//! spinner and the "X asks:" line. The words come from
//! [`crate::logic::cards`]; these only draw them.

use irori_window::cards::{AgentRef, CardAsk, CardSummary};
use rune::{Color, El, Tx, el, icon, row, text};

use crate::logic::agent_status::{ms, now_ms};
use crate::logic::cards::{self as lc, Tone, ask, card_tone, percent};
use crate::logic::names::display_name;
use crate::logic::needs::pr_chip as pr_chip_of;
use crate::logic::owners::card_owner;
use crate::logic::plain::plain_line;
use crate::ui::theme::status;
use crate::ui::{Frame, State, icons};

/// A tone's fill, ink and border (the React app's badge variants).
pub fn tone_colors(f: &Frame, t: Tone) -> (Color, Color, Color) {
    let c = &f.k.c;
    match t {
        Tone::Info => (c.info.bg, c.info.fg, c.info.border),
        Tone::Destructive => (c.danger.bg, c.danger.fg, c.danger.border),
        Tone::Success => (c.success.bg, c.success.fg, c.success.border),
        Tone::Warning => (c.warning.bg, c.warning.fg, c.warning.border),
        Tone::Slate => (c.layer_02, c.text_muted, c.border_muted),
        Tone::Outline => (Color::default(), c.text_muted, c.border_base),
        Tone::Secondary => (c.layer_03, c.text_base, c.layer_03),
    }
}

/// A tone's solid colour, for bars and dots.
pub fn tone_solid(f: &Frame, t: Tone) -> Color {
    match t {
        Tone::Info => status::review(),
        Tone::Destructive => status::failed(),
        Tone::Success => status::working(),
        Tone::Warning => status::decide(),
        Tone::Slate | Tone::Outline | Tone::Secondary => f.k.c.text_muted,
    }
}

/// A chip: `label` in `tone`, round (a calm state) or square, with an
/// optional tooltip.
pub fn badge(f: &Frame, key: &str, label: &str, tone: Tone, round: bool, tip: &str) -> El {
    let (bg, fg, border) = tone_colors(f, tone);
    let mut b = row()
        .key(key.to_string())
        .h(20.0)
        .px(7.0)
        .center_y()
        .min_w(0.0)
        .radius(if round { 10.0 } else { 5.0 })
        .fill(bg)
        .border(1.0, border)
        .child(
            text(label.to_string())
                .font(f.k.ty.text_xsmall_medium)
                .color(fg)
                .max_lines(1)
                .shrink(),
        );
    if !tip.is_empty() {
        b = b.help(tip.to_string());
    }
    b
}

/// The card's first badge: Not yet, Ask, its blockers, a wait, In
/// review, In progress… (bits.tsx StatePill).
pub fn state_pill(f: &Frame, key: &str, c: &CardSummary) -> El {
    let p = lc::state_pill(c, now_ms());
    let b = badge(f, key, &p.label, p.tone, p.round, &p.tip);
    // A long wait truncates to its card's width; the tooltip has it all.
    if c.waiting_on > 0 { b.shrink() } else { b }
}

/// A card's done steps over all steps in its status colour; `None`
/// without steps.
pub fn progress_bar(f: &Frame, key: &str, c: &CardSummary) -> Option<El> {
    let p = percent(c.progress.done, c.progress.total)?;
    let k = f.k;
    let fill = tone_solid(f, card_tone(c));
    Some(
        el().key(key.to_string())
            .role("progressbar")
            .label(format!("{} of {} steps", c.progress.done, c.progress.total))
            .w_full()
            .h(4.0)
            .fill(k.c.layer_03)
            .child(
                el().key(format!("{key}/fill"))
                    .w_frac(p as f32 / 100.0)
                    .h_full()
                    .fill(fill),
            ),
    )
}

fn line(
    f: &Frame,
    key: &str,
    warn: bool,
    dot: Option<Color>,
    words: String,
    extra: Option<El>,
) -> El {
    let k = f.k;
    let mut r = row().key(key.to_string()).gap(6.0).center_y().min_w(0.0);
    if let Some(d) = dot {
        r = r.child(el().size(7.0, 7.0).radius(3.5).fill(d).a11y_hidden());
    }
    r = r.child(
        text(words)
            .font(rune::tok::TextStyle::new(12.5, 400.0))
            .color(if warn {
                status::decide()
            } else {
                k.c.text_muted
            })
            .max_lines(1)
            .shrink(),
    );
    if let Some(e) = extra {
        r = r.child(e);
    }
    r
}

/// Who holds the card and what that agent is up to, by the live roster
/// (bits.tsx HolderLine).
pub fn holder_line(s: &State, f: &Frame, key: &str, c: &CardSummary) -> El {
    let live = s
        .data
        .agents
        .iter()
        .find(|a| a.card.key == c.holder.who.key)
        .map(|a| (a.card.status.as_str(), ms(&a.card.status_since)));
    let h = lc::holder_line(&c.holder, c.returned.as_ref(), live, now_ms());
    line(f, key, h.warn, h.dot.then(status::working), h.text, None)
}

/// An open ask's line in place of the holder line (bits.tsx AskLine).
pub fn ask_line(f: &Frame, key: &str, a: &CardAsk) -> El {
    let dot = match a.state.as_str() {
        ask::TAKEN => Some(status::working()),
        ask::HANDED => Some(status::review()),
        _ => None,
    };
    let on = a.taker.as_ref().filter(|t| !t.device.is_empty()).map(|t| {
        text(format!("on {}", plain_line(&t.device)))
            .font(f.k.ty.text_xsmall)
            .color(f.k.c.text_muted)
    });
    line(f, key, lc::ask_warns(a), dot, lc::ask_line(a), on)
}

/// Where the card lives: "My devices", or its shared board's name.
pub fn scope_badge(f: &Frame, key: &str, c: &CardSummary) -> El {
    let l = plain_line(&c.scope_label);
    badge(
        f,
        key,
        if l.is_empty() { "My devices" } else { &l },
        Tone::Outline,
        false,
        "",
    )
}

/// The work/personal side a card is on (card #494), and its wait when
/// the device that picks an agent doesn't run it (#495).
pub fn side_badge(f: &Frame, key: &str, c: &CardSummary) -> Option<El> {
    let side = plain_line(&c.side);
    if side.is_empty() {
        return None;
    }
    let wait = plain_line(&c.side_wait);
    Some(if wait.is_empty() {
        badge(
            f,
            key,
            &side,
            Tone::Secondary,
            false,
            &format!("On the {side} side: devices that run {side} take it up"),
        )
    } else {
        badge(
            f,
            key,
            &format!("{side}: waits, {wait}"),
            Tone::Outline,
            false,
            &format!("Waits: {wait}. It is never handed to another device."),
        )
    })
}

/// "waits on PR #N" on an in-review card (card #471): one press opens it.
pub fn pr_chip(f: &Frame, key: &str, c: &CardSummary) -> Option<El> {
    let p = pr_chip_of(c)?;
    let (bg, fg, border) = tone_colors(f, Tone::Warning);
    let url = p.url.clone();
    Some(
        row()
            .key(key.to_string())
            .role("link")
            .focusable(true)
            .h(20.0)
            .px(7.0)
            .gap(4.0)
            .center_y()
            .radius(5.0)
            .fill(bg)
            .border(1.0, border)
            .cursor(rune::Cursor::Pointer)
            .help(p.title)
            .on_press_tx(move |_: &mut State, tx: &mut Tx| {
                let _ = tx.open_url(&url);
            })
            .child(text(p.text).font(f.k.ty.text_xsmall).color(fg))
            .child(icon(&icons::EXTERNAL_LINK, 11.0).color(fg)),
    )
}

/// Whose a shared board's card is (card #268, #432); nothing on a private
/// card or a board that is only the person's.
pub fn owner_chip(s: &State, f: &Frame, key: &str, c: &CardSummary) -> Option<El> {
    let o = card_owner(c, &s.data.spaces)?;
    let name = plain_line(&o.name);
    let tip = if o.me {
        "Yours: you decide Done and answer its questions".to_string()
    } else if o.known {
        format!("{name}'s card: they decide Done and answer its questions")
    } else {
        "Made by someone who is no longer in the space".to_string()
    };
    let (bg, fg, border) = tone_colors(f, if o.me { Tone::Info } else { Tone::Outline });
    Some(
        row()
            .key(key.to_string())
            .h(20.0)
            .px(7.0)
            .gap(4.0)
            .center_y()
            .radius(5.0)
            .fill(bg)
            .border(1.0, border)
            .help(tip)
            .child(icon(&icons::USER, 12.0).color(fg))
            .child(text(name).font(f.k.ty.text_xsmall).color(fg).max_lines(1)),
    )
}

/// The card's short ref, "mlg#12", with what it was when it was renumbered.
pub fn short_ref(f: &Frame, key: &str, c: &CardSummary) -> Option<El> {
    let short = plain_line(&c.short);
    if short.is_empty() {
        return None;
    }
    let tag = plain_line(&c.tag);
    let was = plain_line(&c.short_was);
    let k = f.k;
    let tip = if was.is_empty() {
        format!("Agents know this card as {tag}#{short}")
    } else {
        format!(
            "Was {tag}#{was}: a card made on another device while they were apart had {tag}#{was} too, so each now has a letter"
        )
    };
    let mut r = row()
        .key(key.to_string())
        .gap(4.0)
        .center_y()
        .help(tip)
        .child(
            text(format!("{tag}#{short}"))
                .font(k.ty.text_xsmall.mono())
                .color(k.c.text_muted),
        );
    if !was.is_empty() {
        r = r.child(
            text(format!("(was {tag}#{was})"))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted),
        );
    }
    Some(r)
}

/// An agent is working on the card right now: a turning ring (a still dot
/// under reduced motion, Rune's own).
pub fn spinner(f: &Frame, key: &str) -> El {
    el().key(key.to_string())
        .role("img")
        .label("An agent is working on it")
        .help("An agent is working on it")
        .child(
            rune_kit::Loader::new()
                .size(14.0)
                .color(f.k.c.icon_accent)
                .build(f.k),
        )
}

/// Who asks a blocker's question, above it (card #491).
pub fn asker(f: &Frame, key: &str, who: &AgentRef) -> El {
    let k = f.k;
    row()
        .key(key.to_string())
        .gap(4.0)
        .child(
            text(display_name(who))
                .font(rune::tok::TextStyle::new(11.5, 500.0))
                .color(k.c.text_base),
        )
        .child(
            text("asks:")
                .font(rune::tok::TextStyle::new(11.5, 400.0))
                .color(k.c.text_muted),
        )
}
