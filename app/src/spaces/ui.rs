//! Small pieces the Spaces pages share: the glass panel, text lines in the
//! React app's sizes and tones, the state bar, recent cards with faces,
//! a confirm step, and the freshness lines.

use rune::tok::TextStyle;
use rune::{Color, El, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Dialog, Kit};

use irori_window::cards::CardSummary;

use super::logic::{self, Seg, Tone};
use crate::ui::{Frame, State, icons};

/// text-[13px], text-xs, text-[11px] in the React app.
pub fn t13(_k: &Kit) -> TextStyle {
    TextStyle::new(13.0, 400.0)
}
pub fn t12(_k: &Kit) -> TextStyle {
    TextStyle::new(12.0, 400.0)
}
pub fn t11(_k: &Kit) -> TextStyle {
    TextStyle::new(11.0, 400.0)
}
pub fn t125(_k: &Kit) -> TextStyle {
    TextStyle::new(12.5, 400.0)
}

pub fn danger(k: &Kit) -> Color {
    k.c.danger.icon
}
pub fn warning(k: &Kit) -> Color {
    k.c.warning.icon
}
pub fn success(k: &Kit) -> Color {
    k.c.success.icon
}
pub fn info(k: &Kit) -> Color {
    k.c.info.icon
}

/// A glass panel (the React app's `glass-panel`).
pub fn panel(k: &Kit) -> El {
    col()
        .radius(10.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
}

/// A wrapped line of text.
pub fn line(s: impl Into<String>, font: TextStyle, color: Color) -> El {
    text(s.into()).font(font).color(color).w_full().wrap()
}

pub fn muted(k: &Kit, s: impl Into<String>) -> El {
    line(s, t12(k), k.c.text_muted)
}

pub fn error_line(k: &Kit, s: impl Into<String>) -> El {
    line(s, t12(k), danger(k))
}

/// The small grey chip ("owner", "default").
pub fn chip(k: &Kit, s: &str) -> El {
    el().px(4.0)
        .radius(4.0)
        .fill(k.c.layer_03)
        .child(text(s.to_string()).font(t11(k)).color(k.c.text_muted))
}

/// A section's heading (h2 text-base font-semibold).
pub fn heading(k: &Kit, s: &str) -> El {
    text(s.to_string())
        .font(TextStyle::new(15.0, 600.0))
        .color(k.c.text_base)
        .px(4.0)
}

/// The state dot: green when ready, amber when not (or warned).
pub fn dot(k: &Kit, ok: bool) -> El {
    el().size(8.0, 8.0)
        .radius(4.0)
        .fill(if ok { success(k) } else { warning(k) })
}

fn tone_color(k: &Kit, t: Tone) -> Color {
    match t {
        Tone::Destructive => danger(k),
        Tone::Info => info(k),
        Tone::Slate => k.alpha(k.c.text_muted, 0.45),
        Tone::Success => success(k),
    }
}

/// The tile's thin bar of card counts by state, with the counts under it.
pub fn state_bar(k: &Kit, key: &str, segs: &[Seg], width: f32) -> El {
    let total: usize = segs.iter().map(|s| s.n).sum();
    let words = logic::segs_text(segs);
    let mut bar = row()
        .key(format!("{key}-bar"))
        .w(width)
        .h(6.0)
        .radius(3.0)
        .clip_rounded(3.0)
        .fill(k.c.layer_03)
        .role("img")
        .label(if words.is_empty() {
            "No cards".to_string()
        } else {
            words.clone()
        });
    if total > 0 {
        for s in segs.iter().filter(|s| s.n > 0) {
            bar = bar.child(
                el().h(6.0)
                    .w(width * s.n as f32 / total as f32)
                    .fill(tone_color(k, s.tone)),
            );
        }
    }
    col().gap(4.0).child(bar).child(
        text(if words.is_empty() {
            "No cards yet".to_string()
        } else {
            words
        })
        .font(t11(k))
        .color(k.c.text_muted),
    )
}

/// A round face with the holder's initial (cards/faces.tsx, cut down).
pub fn face(k: &Kit, name: &str, size: f32) -> El {
    let initial = logic::plain_line(name)
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    el().size(size, size)
        .radius(size / 2.0)
        .fill(k.alpha(k.c.text_accent, 0.22))
        .center()
        .child(
            text(initial)
                .font(TextStyle::new(size * 0.5, 600.0))
                .color(k.c.text_accent),
        )
}

/// A tile's few most recently active cards, with their holders' faces.
pub fn recent_cards(k: &Kit, key: &str, cards: &[&CardSummary]) -> Option<El> {
    if cards.is_empty() {
        return None;
    }
    let mut list = col()
        .key(format!("{key}-recent"))
        .gap(4.0)
        .w_full()
        .label("Recently active cards");
    for c in cards {
        let lead = if c.holder.who.key.is_empty() {
            el().size(20.0, 20.0)
                .border(1.0, k.alpha(k.c.text_muted, 0.4))
                .help("Nobody holds it")
        } else {
            face(k, &c.holder.who.name, 20.0)
        };
        list = list.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(lead)
                .child(
                    text(format!("{}#{}", c.tag, c.short))
                        .font(t11(k).mono())
                        .color(k.c.text_muted),
                )
                .child(
                    text(logic::plain_line(&c.title))
                        .font(t125(k))
                        .color(k.c.text_base)
                        .max_lines(1)
                        .shrink()
                        .min_w(0.0),
                ),
        );
    }
    Some(list)
}

/// The full freshness line under a space's header: "Last synced with your
/// PC 3h ago", amber after a day.
pub fn fresh_line(s: &State, k: &Kit, log: &str) -> Option<El> {
    let f = s.spaces.fresh.get(log);
    let now = logic::now_ms();
    let t = logic::fresh_line(f, now);
    if t.is_empty() {
        return None;
    }
    let old = logic::stale(f, now);
    let color = if old { warning(k) } else { k.c.text_muted };
    let t = if old {
        format!("{t}{}", logic::STALE_TAIL)
    } else {
        t
    };
    Some(
        row()
            .w_full()
            .gap(6.0)
            .center_y()
            .child(icon(&icons::REFRESH, 12.0).color(color))
            .child(text(t).font(t12(k)).color(color).wrap().shrink().min_w(0.0)),
    )
}

/// A confirm step (common/confirm-button.tsx `ConfirmDialog`): the confirm
/// label as the title, the question under it, Cancel and the confirm.
pub fn confirm(
    f: &mut Frame,
    key: &str,
    label: &str,
    question: &str,
    destructive: bool,
    on_confirm: impl Fn(&mut State) + 'static,
    on_cancel: impl Fn(&mut State) + Clone + 'static,
) {
    let k = f.k;
    let cancel = on_cancel.clone();
    let buttons = row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .padding(16.0)
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(move |s: &mut State| cancel(s))
                .build(k),
        )
        .child(
            Button::new(label.to_string())
                .variant(if destructive {
                    ButtonVariant::Danger
                } else {
                    ButtonVariant::Neutral
                })
                .on_press(on_confirm)
                .build(k)
                .default_button(),
        );
    Dialog::new(key.to_string(), label.to_string())
        .description(question.to_string())
        .open(true)
        .fit()
        .on_close(on_cancel)
        .child(buttons)
        .build(k, f.layer);
}
