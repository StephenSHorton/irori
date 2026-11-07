//! Settings > People and invites (settings-dialog.tsx PeoplePane and
//! YourName, lib/settings.ts useYourName): your one name (card #183),
//! everyone on your spaces, space by space, and joining with an invite.

use rune::{El, TextEdit, Tx, col, el, row, text};
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
}

#[derive(Default)]
pub struct Model {
    pub name: NameEdit,
}

pub fn load(s: &mut State) {
    load_name(s, |s| &mut s.settings.people.name);
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
        .on_edit(move |s: &mut State| {
            let ne = at(s);
            ne.saved = false;
            &mut ne.edit
        })
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
    let line = match (&ne.error, ne.saved) {
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
    if spaces.is_empty() {
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
    // Join with an invite someone sent you, and the invite in progress:
    // the spaces package's (P7, boards/parts.tsx JoinInvite, InvitePanel).
    // TODO(P7): `crate::spaces::join_invite(s, f)` here once it exists.
    out.child(super::ruled(
        f,
        "settings-join",
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(hint(f, "Got an invite? Join it from the Spaces page.").grow(1.0).min_w(0.0))
            .child(
                Button::new("Open Spaces")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut State| {
                        s.close_dialog();
                        s.go(Route::Spaces);
                    })
                    .build(k),
            )
            .child(el()),
    ))
}
