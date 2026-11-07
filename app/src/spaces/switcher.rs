//! The space switcher (space-switcher.tsx, card #393): at the top of the
//! sidebar, it picks the space Home, Talk and Boards show. All spaces is
//! everything, Needs you across all of it; Personal is you and your
//! agents; then each shared space, "with" whoever is in it. It is the one
//! place for spaces (card #689, which dropped the Spaces tab; Boards shows
//! only the picked space's): at its foot, Space settings for a shared pick
//! (its page: its boards, people, invites and settings) and New or join a
//! space.

use rune::{El, el, icon, row, text};
use rune_kit::{Menu, MenuItem, Placement};

use super::logic::{self, ALL_SPACES, PERSONAL, Space, plain_line};
use super::{current, pick};
use crate::ui::{Frame, Route, State, icons};

/// A space's mark: a layers icon for All spaces, a person for Personal,
/// else the space's initial.
fn mark(f: &Frame, s: &Space) -> El {
    let k = f.k;
    let base = el().size(20.0, 20.0).radius(4.0).center();
    match s.id.as_str() {
        ALL_SPACES => base.fill(k.c.layer_03).child(icon(&icons::SPACES, 12.0).color(k.c.text_muted)),
        PERSONAL => base.fill(k.c.layer_03).child(icon(&icons::USER, 12.0).color(k.c.text_muted)),
        _ => base.fill(k.alpha(k.c.text_accent, 0.2)).child(
            text(
                plain_line(&s.name)
                    .chars()
                    .next()
                    .map_or("?".to_string(), |c| c.to_uppercase().to_string()),
            )
            .font(rune::tok::TextStyle::new(10.5, 600.0))
            .color(k.c.text_accent),
        ),
    }
}

/// The switcher: its trigger (the picked space and who is in it), and the
/// menu of spaces while open. `folded` is the sidebar folded to icons.
pub fn switcher(s: &State, f: &mut Frame) -> El {
    switcher_folded(s, f, false)
}

pub fn switcher_folded(s: &State, f: &mut Frame, folded: bool) -> El {
    let k = f.k;
    let spaces = logic::spaces_of(&s.data.spaces);
    let picked = current(s);
    let cur = spaces.iter().find(|x| x.id == picked).unwrap_or(&spaces[0]).clone();
    let others: Vec<String> = cur.others.iter().map(|o| plain_line(o)).collect();
    let line = logic::with_line(&others);
    let mut trigger = row()
        .key("spaces-switcher-button")
        .role("button")
        .label(format!("Space: {}. Pick a space", plain_line(&cur.name)))
        .help(if folded {
            format!("Space: {}", plain_line(&cur.name))
        } else {
            "Pick the space Home, Talk and Boards show, or make or join one".to_string()
        })
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .h(36.0)
        .gap(8.0)
        .px(if folded { 0.0 } else { 8.0 })
        .center_y()
        .radius(6.0)
        .border(1.0, k.c.border_muted)
        .fill(k.alpha(k.c.bg_base, 0.4))
        .on_press(|s: &mut State| s.spaces.switcher_open = !s.spaces.switcher_open)
        .child(mark(f, &cur));
    if folded {
        trigger = trigger.justify_center();
    } else {
        let mut words = vec![rune::Span::new(0, plain_line(&cur.name).len()).weight(500.0)];
        let mut label = plain_line(&cur.name);
        if !line.is_empty() {
            let start = label.len();
            label.push_str(&format!(" · {line}"));
            words.push(rune::Span::new(start, label.len()).color(k.c.text_muted));
        }
        trigger = trigger
            .child(
                rune::rich(label, words)
                    .font(rune::tok::TextStyle::new(14.0, 400.0))
                    .color(k.c.text_base)
                    .max_lines(1)
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(icon(&icons::CHEVRON_DOWN, 14.0).color(k.c.text_muted));
    }
    let mut menu = Menu::new("spaces-switcher")
        .trigger(trigger)
        .open(s.spaces.switcher_open)
        .placement(Placement::BottomStart)
        .min_width(256.0)
        .on_close(|s: &mut State| s.spaces.switcher_open = false);
    for (i, x) in spaces.iter().enumerate() {
        if i == 2 {
            menu = menu.separator();
        }
        let mut name = plain_line(&x.name);
        if !x.others.is_empty() {
            let o: Vec<String> = x.others.iter().map(|o| plain_line(o)).collect();
            name = format!("{name} · {}", logic::with_line(&o));
        }
        let id = x.id.clone();
        let mut item = MenuItem::new(name)
            .lead(mark(f, x))
            .checked(x.id == cur.id)
            .on_select(move |s: &mut State| {
                pick(s, &id);
                s.spaces.switcher_open = false;
            });
        if i == 0 {
            item = item.description("Needs you across everything");
        } else {
            let fr = logic::short_line(s.spaces.fresh.get(&x.id), logic::now_ms());
            if !fr.is_empty() {
                item = item.description(fr);
            }
        }
        menu = menu.item(item);
    }
    menu = menu.separator();
    if cur.id != ALL_SPACES && cur.id != PERSONAL {
        let id = cur.id.clone();
        menu = menu.item(
            MenuItem::new("Space settings")
                .lead(foot_icon(f, &icons::SETTINGS))
                .on_select(move |s: &mut State| {
                    s.spaces.switcher_open = false;
                    s.close_card();
                    s.go(Route::Board {
                        space: id.clone(),
                        board: None,
                    });
                }),
        );
    }
    menu = menu.item(
        MenuItem::new("New or join a space")
            .lead(foot_icon(f, &icons::PLUS))
            .on_select(super::open_new_space),
    );
    menu.build(k, f.layer)
}

/// A foot item's icon in a mark-sized box, so its words line up with the
/// spaces' names.
fn foot_icon(f: &Frame, ic: &'static rune::Icon) -> El {
    el().size(20.0, 20.0).center().child(icon(ic, 14.0).color(f.k.c.text_muted))
}
