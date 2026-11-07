//! Settings > People and invites (settings-dialog.tsx PeoplePane and
//! YourName, lib/settings.ts useYourName): your one name (card #183),
//! everyone on your spaces, space by space, and joining with an invite.

use rune::{El, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, TextInput};

use crate::ui::{Event, Frame, Route, State};

use super::{hint, muted_line};

/// The person's one name being edited (Settings and the profile page each
/// keep one): what's saved, the field, and the line under it.
#[derive(Default)]
pub struct NameEdit {
    /// None until read.
    pub name: Option<String>,
    pub edit: TextEdit,
    pub saved: bool,
    pub error: Option<String>,
}

impl NameEdit {
    pub fn set(&mut self, name: String) {
        self.edit = TextEdit::new(name.clone());
        self.name = Some(name);
    }
    /// The field differs from the saved name.
    pub fn changed(&self) -> bool {
        self.name.as_deref().is_some_and(|n| self.edit.text.trim() != n)
    }
    /// "Saved." shows after a save until the field is edited again (the
    /// field's accessor runs for focus and caret too, so it can't clear it).
    pub fn shows_saved(&self) -> bool {
        self.saved && !self.changed()
    }
}

/// JoinInvite's form (boards/parts.tsx): the link pasted, a join on its
/// way, and why the last one failed.
#[derive(Default)]
pub struct Join {
    pub link: TextEdit,
    pub busy: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Model {
    pub name: NameEdit,
    pub join: Join,
    /// The spaces are read: "No spaces yet" only after that (card #598).
    pub spaces_read: bool,
}

pub fn load(s: &mut State) {
    load_name(s, |s| &mut s.settings.people.name);
    s.spawn(|w| w.boards_list(), |s, v| {
        s.data.spaces = v;
        s.settings.people.spaces_read = true;
    });
    // An invite opened elsewhere shows here in place of the form.
    crate::spaces::reload_invite_now(s);
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// Reads the name into a NameEdit (keeping an edit in progress).
pub fn load_name(s: &mut State, at: fn(&mut State) -> &mut NameEdit) {
    s.spawn(|w| w.get_your_name(), move |s, n| {
        let ne = at(s);
        if !ne.changed() {
            ne.set(n);
        }
    });
}

/// Saves the field (Enter or Save).
pub fn save_name(s: &mut State, at: fn(&mut State) -> &mut NameEdit) {
    let ne = at(s);
    if !ne.changed() || ne.edit.text.trim().is_empty() {
        return;
    }
    let draft = ne.edit.text.clone();
    ne.error = None;
    s.spawn(move |w| w.set_your_name(&draft), move |s, r| match r {
        Ok(kept) if !kept.is_empty() => {
            s.data.your_name = kept.clone();
            let ne = at(s);
            ne.set(kept);
            ne.saved = true;
        }
        Ok(_) => at(s).error = Some("A name can't be empty.".into()),
        Err(e) => at(s).error = Some(e),
    });
}

/// The name field and Save; Escape drops the edit.
pub fn name_field(f: &Frame, key: &str, ne: &NameEdit, width: Option<f32>, at: fn(&mut State) -> &mut NameEdit) -> El {
    let k = f.k;
    let field = TextInput::new(&ne.edit)
        .label("Your name")
        .width(width)
        .disabled(ne.name.is_none())
        .on_edit(move |s: &mut State| &mut at(s).edit)
        .on_submit(move |s: &mut State, _: &mut Tx| save_name(s, at))
        .on_key(move |s: &mut State, ev: &rune::KeyEvent, _: &mut Tx| {
            if ev.key == rune::Key::Escape {
                let ne = at(s);
                if ne.changed() {
                    let n = ne.name.clone().unwrap_or_default();
                    ne.set(n);
                    return true;
                }
            }
            false
        })
        .build(k)
        .key(format!("{key}-field"));
    let field = if width.is_none() { field.grow(1.0).min_w(0.0) } else { field };
    row()
        .key(key.to_string())
        .w_full()
        .gap(8.0)
        .center_y()
        .child(field)
        .child(
            Button::new("Save")
                .variant(ButtonVariant::Outline)
                .disabled(!ne.changed() || ne.edit.text.trim().is_empty())
                .on_press(move |s: &mut State| save_name(s, at))
                .build(k),
        )
}

fn your_name(s: &State, f: &Frame) -> El {
    let k = f.k;
    let ne = &s.settings.people.name;
    let line = match (&ne.error, ne.shows_saved()) {
        (Some(e), _) => e.clone(),
        (None, true) => "Saved. Your other devices and your spaces pick it up as they sync.".into(),
        _ => "What people in your spaces see you as, on all your devices.".into(),
    };
    col()
        .key("settings-your-name")
        .w_full()
        .gap(6.0)
        .child(text("Your name").font(k.ty.ui_medium).color(k.c.text_base))
        .child(name_field(f, "settings-name", ne, Some(256.0), |s| &mut s.settings.people.name))
        .child(if ne.error.is_some() { super::error_line(f, &line) } else { hint(f, &line) })
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let mut out = col().key("settings-people").w_full().gap(12.0).child(your_name(s, f));
    let spaces: Vec<_> = s.data.spaces.iter().filter(|b| !b.leaving).collect();
    if spaces.is_empty() && !s.settings.people.spaces_read {
        out = out.child(super::loading(f, "settings-people-loading", "Loading your spaces"));
    } else if spaces.is_empty() {
        out = out.child(muted_line(f, "No spaces yet. Make one on the Spaces page, then invite people to it."));
    } else {
        let mut list = col().key("settings-people-list").w_full().gap(6.0).role("list").label("People in your spaces");
        for b in spaces {
            let id = b.board_id.clone();
            let name = super::plain_line(&b.name);
            let people = b.people.iter().map(|p| super::plain_line(p)).collect::<Vec<_>>().join(", ");
            list = list.child(
                row()
                    .key(format!("settings-people-{}", b.board_id))
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .px(12.0)
                    .py(8.0)
                    .radius(8.0)
                    .border(1.0, k.c.border_base)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(
                                row()
                                    .gap(6.0)
                                    .center_y()
                                    .child(
                                        text(if name.is_empty() { "(unnamed)".into() } else { name })
                                            .font(k.ty.ui_medium)
                                            .color(k.c.text_base)
                                            .truncate(),
                                    )
                                    .child(
                                        text(format!("{}#", super::plain_line(&b.tag)))
                                            .font(k.ty.small.mono())
                                            .color(k.c.text_muted),
                                    ),
                            )
                            .child(
                                text(if people.is_empty() { "Just you".into() } else { people })
                                    .font(k.ty.small)
                                    .color(k.c.text_muted)
                                    .truncate()
                                    .w_full(),
                            ),
                    )
                    .child(
                        Button::new(if b.owner { "Invite…" } else { "Open" })
                            .variant(ButtonVariant::Outline)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut State| {
                                s.close_dialog();
                                s.go(Route::Board { space: id.clone(), board: None });
                            })
                            .build(k),
                    ),
            );
        }
        out = out.child(list);
    }
    // Join with an invite someone sent you, or the invite in progress
    // in its place (the spaces package's InvitePanel).
    let open = s.spaces.invite.as_ref().filter(|i| !i.state.is_empty());
    out.child(match open {
        Some(inv) => super::ruled(f, "settings-invite", crate::spaces::invite_panel(s, f, inv)),
        None => join_invite(s, f),
    })
}

/// JoinInvite: paste the irori-invite: link someone sent, then Join.
fn join_invite(s: &State, f: &Frame) -> El {
    let k = f.k;
    let j = &s.settings.people.join;
    let ok = !j.busy && !j.link.text.trim().is_empty();
    let mut c = col()
        .key("settings-join")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(
                    TextInput::new(&j.link)
                        .placeholder("irori-invite:…")
                        .label("The invite link someone sent you")
                        .width(None)
                        .text_style(rune::tok::TextStyle::new(13.0, 400.0).mono())
                        .on_edit(|s: &mut State| &mut s.settings.people.join.link)
                        .on_submit(|s: &mut State, _: &mut Tx| join_submit(s))
                        .build(k)
                        .key("settings-join-link")
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(Button::new("Join").disabled(!ok).on_press(join_submit).build(k).key("settings-join-go")),
        )
        .child(hint(f, "Paste the link, then give whoever sent it the five words you'll see."));
    if let Some(e) = &j.error {
        c = c.child(super::error_line(f, e));
    }
    super::ruled(f, "settings-join-box", c)
}

fn join_submit(s: &mut State) {
    let j = &mut s.settings.people.join;
    let link = j.link.text.trim().to_string();
    if j.busy || link.is_empty() {
        return;
    }
    j.busy = true;
    j.error = None;
    s.spawn(
        move |w| w.boards_join_invite(&link),
        |s, r| {
            let j = &mut s.settings.people.join;
            j.busy = false;
            match r {
                Ok(st) => {
                    j.link = TextEdit::default();
                    s.spaces.invite = Some(st);
                    s.spaces.words.clear();
                    s.spaces.words_error = None;
                }
                Err(e) => j.error = Some(crate::spaces::logic::error_text(&e)),
            }
        },
    );

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_shows_until_the_name_is_edited_again() {
        let mut ne = NameEdit::default();
        ne.set("Stephen".into());
        assert!(!ne.changed() && !ne.shows_saved());
        ne.edit = TextEdit::new("Stephen H");
        assert!(ne.changed());
        ne.set("Stephen H".into());
        ne.saved = true;
        assert!(ne.shows_saved(), "a focus or caret pass leaves it");
        ne.edit = TextEdit::new("Stephen Ho");
        assert!(!ne.shows_saved(), "typing again hides it");
        ne.edit = TextEdit::new("Stephen H ");
        assert!(ne.shows_saved(), "only a real change counts");
    }
}
