//! Settings > Merge desks (desks-pane.tsx, lib/desk.ts's settings half;
//! card #417, #521, #625). A GitHub repo you or your space members open
//! PRs in has a desk by itself: on, polled from the device that saw it
//! last. Here the person turns a desk off, picks the device that polls
//! GitHub, and names the logins whose PRs also count as ours. Agents can't
//! write any of this.

use irori_window::desk::{DeskSettings, DeskView};
use irori_window::events::EVENT_BOARD_CHANGED;
use rune::{El, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Switch, TextInput};

use crate::ui::{Event, Frame, State, icons};

use super::{choice, error_line, hint, loading, muted_line};

const NONE: &str = "-";

#[derive(Default)]
pub struct Model {
    pub desks: Option<Vec<DeskView>>,
    /// The repo whose editor is open (a new one shows first).
    pub editing: String,
    /// The editor's working copy and its trusted-logins field.
    pub draft: Option<DeskSettings>,
    pub trusted: TextEdit,
    pub saving: bool,
    pub edit_err: String,
    /// "Add a desk": the repo field.
    pub adding: TextEdit,
    pub err: String,
}

/// "owner/name", a GitHub URL or a repo key as the key a desk is stored
/// under, or "" when it isn't a GitHub repo.
pub fn github_key(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_suffix(".git").unwrap_or(t);
    let t = t.strip_suffix('/').unwrap_or(t);
    let pair = |x: &str| {
        let mut p = x.split('/');
        match (p.next(), p.next(), p.next()) {
            (Some(a), Some(b), None)
                if !a.is_empty() && !b.is_empty() && !x.contains(char::is_whitespace) =>
            {
                Some(format!("{a}/{b}"))
            }
            _ => None,
        }
    };
    let rest = t
        .strip_prefix("remote:github.com/")
        .or_else(|| {
            let u = t
                .strip_prefix("https://")
                .or_else(|| t.strip_prefix("http://"))
                .unwrap_or(t);
            let u = u.strip_prefix("www.").unwrap_or(u);
            u.strip_prefix("github.com/")
        })
        .or_else(|| t.strip_prefix("git@github.com:"));
    let found = match rest {
        Some(r) => pair(r),
        None => pair(t).filter(|p| {
            p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-/".contains(c))
        }),
    };
    found
        .map(|p| format!("remote:github.com/{p}"))
        .unwrap_or_default()
}

/// "Owner/Name" from a repo key, or the key itself.
pub fn owner_name(key: &str) -> String {
    match key.find("github.com") {
        Some(i) => {
            let rest = key[i + "github.com".len()..].trim_start_matches(['/', ':']);
            let rest = rest.strip_suffix(".git").unwrap_or(rest);
            if rest.split('/').count() == 2 {
                rest.to_string()
            } else {
                key.to_string()
            }
        }
        None => key.to_string(),
    }
}

/// The logins typed, split on commas and spaces.
pub fn list(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

/// The settings a new desk starts from (coord new_desk).
pub fn new_desk(repo_key: &str, poll_device: &str) -> DeskSettings {
    DeskSettings {
        repo_key: repo_key.into(),
        enabled: true,
        takes_prs: true,
        poll_device: poll_device.into(),
        ..Default::default()
    }
}

pub fn load(s: &mut State) {
    s.spawn(|w| w.desks(), |s, d| s.settings.desks.desks = Some(d));
}

pub fn on_event(s: &mut State, ev: &Event) {
    if ev.is(EVENT_BOARD_CHANGED) && super::open_pane(s) == Some("desks") {
        load(s);
    }
}

fn here(s: &State) -> String {
    s.data
        .devices
        .iter()
        .find(|d| d.this)
        .map(|d| d.device_id.clone())
        .unwrap_or_default()
}

/// A poll device that follows where the repo was seen last (the
/// default's, or a saved blank one) is saved blank so it keeps following.
fn start_of(d: &DeskView) -> DeskSettings {
    let mut st = d.settings.clone();
    if d.is_default || d.follows_seen {
        st.poll_device.clear();
    }
    st
}

fn save(s: &mut State, st: DeskSettings, done: impl FnOnce(&mut State) + Send + 'static) {
    s.spawn(
        move |w| w.desk_set(st).map(|_| w.desks()),
        move |s, r| match r {
            Ok(d) => {
                s.settings.desks.desks = Some(d);
                done(s);
            }
            Err(e) => s.settings.desks.err = e,
        },
    );
}

fn open_editor(s: &mut State, st: DeskSettings) {
    let d = &mut s.settings.desks;
    d.editing = st.repo_key.clone();
    d.trusted = TextEdit::new(st.trusted.join(", "));
    d.draft = Some(st);
    d.edit_err.clear();
}

fn add(s: &mut State) {
    let key = github_key(&s.settings.desks.adding.text);
    if key.is_empty() {
        s.settings.desks.err = "Give a GitHub repo as owner/name or its URL.".into();
        return;
    }
    s.settings.desks.err.clear();
    s.settings.desks.adding = TextEdit::new("");
    let existing = s
        .settings
        .desks
        .desks
        .as_ref()
        .and_then(|ds| ds.iter().find(|d| d.settings.repo_key == key))
        .map(start_of);
    let here = here(s);
    open_editor(s, existing.unwrap_or_else(|| new_desk(&key, &here)));
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let m = &s.settings.desks;
    let mut out = col().key("settings-desks").w_full().gap(12.0).child(muted_line(
        f,
        "A GitHub repo you or your space members open PRs in has a desk: it lists your PRs, your people's and your agents' from GitHub, and shows while one is open, for review. The desk never merges: each agent asks you \u{201c}Can I merge PR #N?\u{201d} on its own card and merges on your yes. Only you change these: agents can't.",
    ));
    let devices: Vec<(String, String)> = s
        .data
        .devices
        .iter()
        .filter(|d| !d.revoked)
        .map(|d| {
            (
                d.device_id.clone(),
                if d.name.is_empty() {
                    d.device_id.clone()
                } else {
                    d.name.clone()
                },
            )
        })
        .collect();
    let device_name = |id: &str| {
        devices
            .iter()
            .find(|(v, _)| v == id)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| "another device".into())
    };
    match &m.desks {
        None => out = out.child(loading(f, "settings-desks-loading", "Loading merge desks")),
        Some(desks) => {
            let mut shown: Vec<DeskView> = desks.clone();
            if !m.editing.is_empty() && !shown.iter().any(|d| d.settings.repo_key == m.editing) {
                shown.insert(
                    0,
                    DeskView {
                        settings: m
                            .draft
                            .clone()
                            .unwrap_or_else(|| new_desk(&m.editing, &here(s))),
                        ..Default::default()
                    },
                );
            }
            for d in &shown {
                let st = &d.settings;
                let on = st.enabled && st.takes_prs;
                let open = m.editing == st.repo_key;
                let start = start_of(d);
                let follow = if d.seen_on.is_empty() {
                    String::new()
                } else {
                    device_name(&d.seen_on)
                };
                let line = if !st.takes_prs {
                    "No desk: this repo takes no PRs."
                } else if d.is_default {
                    "on by itself"
                } else {
                    "your settings"
                };
                let mut head = row()
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .child(icon(&icons::MERGE, 16.0).color(k.c.text_base))
                    .child(
                        text(owner_name(&st.repo_key))
                            .font(k.ty.ui.mono())
                            .color(k.c.text_base)
                            .truncate()
                            .shrink()
                            .min_w(0.0),
                    )
                    .child(text(line).font(k.ty.small).color(k.c.text_muted))
                    .child(el().grow(1.0));
                if !open {
                    let st2 = start.clone();
                    head = head.child(
                        Button::new("Edit")
                            .variant(ButtonVariant::Ghost)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut State| open_editor(s, st2.clone()))
                            .build(k),
                    );
                }
                let seen = !d.seen_on.is_empty();
                let st3 = start.clone();
                let enabled = st.enabled;
                head = head
                    .child(
                        text(format!("Desk {}", if st.enabled { "on" } else { "off" }))
                            .font(k.ty.ui)
                            .color(k.c.text_muted),
                    )
                    .child(
                        Switch::new(st.enabled)
                            .label(format!("Desk for {}", owner_name(&st.repo_key)))
                            .hide_label()
                            .disabled(!st.takes_prs && !st.enabled)
                            .on_toggle(move |s: &mut State| {
                                let mut x = st3.clone();
                                x.enabled = !enabled;
                                if x.poll_device.is_empty() && !seen {
                                    x.poll_device = here(s);
                                }
                                save(s, x, |_| {});
                            })
                            .build(k),
                    );
                let mut card = col()
                    .key(format!("settings-desk-{}", st.repo_key))
                    .w_full()
                    .gap(4.0)
                    .px(16.0)
                    .py(10.0)
                    .radius(12.0)
                    .border(1.0, k.c.border_base)
                    .child(head);
                card = if on {
                    card.fill(k.c.layer_02)
                } else {
                    card.border_dash(4.0, 3.0).opacity(0.85)
                };
                if open {
                    card = card.child(editor(s, f, &devices, &follow));
                }
                out = out.child(card);
            }
        }
    }
    out = out.child(
        row()
            .key("settings-desk-add")
            .gap(8.0)
            .center_y()
            .child(
                TextInput::new(&m.adding)
                    .label("Repo for a new desk")
                    .placeholder("owner/name or a GitHub URL")
                    .width(Some(320.0))
                    .on_edit(|s: &mut State| &mut s.settings.desks.adding)
                    .on_submit(|s: &mut State, _: &mut Tx| add(s))
                    .build(k),
            )
            .child(
                Button::new("Add a desk")
                    .variant(ButtonVariant::Outline)
                    .icon(&icons::PLUS)
                    .on_press(|s: &mut State| add(s))
                    .build(k),
            ),
    );
    if !m.err.is_empty() {
        out = out.child(error_line(f, &m.err));
    }
    out
}

fn field(f: &Frame, label: &str, hint_text: Option<&str>, control: El) -> El {
    let mut right = col().grow(1.0).min_w(0.0).gap(4.0).child(control);
    if let Some(h) = hint_text {
        right = right.child(hint(f, h));
    }
    row()
        .w_full()
        .gap(12.0)
        .py(6.0)
        .items_start()
        .child(
            text(label.to_string())
                .font(f.k.ty.ui)
                .color(f.k.c.text_muted)
                .w(130.0)
                .pt(6.0),
        )
        .child(right)
}

fn editor(s: &State, f: &mut Frame, devices: &[(String, String)], follow: &str) -> El {
    let m = &s.settings.desks;
    let Some(d) = &m.draft else { return el() };
    let k = f.k;
    let mut polls = vec![(
        NONE.to_string(),
        if follow.is_empty() {
            "Pick a device".to_string()
        } else {
            format!("Where agents last worked in it ({follow})")
        },
    )];
    polls.extend(devices.iter().cloned());
    let poll = choice(
        s,
        f,
        "settings-desk-poll",
        "Poll device",
        if d.poll_device.is_empty() {
            NONE
        } else {
            &d.poll_device
        },
        &polls,
        300.0,
        false,
        |s, v| {
            if let Some(d) = &mut s.settings.desks.draft {
                d.poll_device = if v == NONE {
                    String::new()
                } else {
                    v.to_string()
                };
            }
        },
    );
    let takes = d.takes_prs;
    let mut ed = col()
        .key("settings-desk-editor")
        .w_full()
        .gap(2.0)
        .pt(8.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base)
        .child(field(
            f,
            "Takes PRs",
            Some("Off for a repo whose work merges straight to main: it gets no desk."),
            row().child(
                Switch::new(takes)
                    .label("Takes PRs")
                    .hide_label()
                    .on_toggle(move |s: &mut State| {
                        if let Some(d) = &mut s.settings.desks.draft {
                            d.takes_prs = !takes;
                        }
                    })
                    .build(k),
            ),
        ))
        .child(field(f, "Polls GitHub from", Some("One device asks GitHub for the open PRs every 3 minutes."), poll))
        .child(field(
            f,
            "Trusted authors",
            Some("GitHub logins whose PRs show here too, beside yours, your space members' and your agents'. Anyone else's PR isn't listed."),
            TextInput::new(&m.trusted)
                .label("Trusted GitHub logins")
                .placeholder("your-login, a-collaborator")
                .on_edit(|s: &mut State| &mut s.settings.desks.trusted)
                .build(k),
        ));
    if !m.edit_err.is_empty() {
        ed = ed.child(error_line(f, &m.edit_err));
    }
    ed.child(
        row()
            .gap(8.0)
            .pt(4.0)
            .child(
                Button::new("Save")
                    .variant(ButtonVariant::Contrast)
                    .disabled(m.saving)
                    .on_press(|s: &mut State| {
                        let dm = &mut s.settings.desks;
                        let Some(mut st) = dm.draft.clone() else {
                            return;
                        };
                        st.trusted = list(&dm.trusted.text);
                        dm.saving = true;
                        dm.edit_err.clear();
                        s.spawn(
                            move |w| w.desk_set(st).map(|_| w.desks()),
                            |s, r| {
                                let dm = &mut s.settings.desks;
                                dm.saving = false;
                                match r {
                                    Ok(d) => {
                                        dm.desks = Some(d);
                                        dm.editing.clear();
                                        dm.draft = None;
                                    }
                                    Err(e) => dm.edit_err = e,
                                }
                            },
                        );
                    })
                    .build(k),
            )
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        s.settings.desks.editing.clear();
                        s.settings.desks.draft = None;
                    })
                    .build(k),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_keys_from_names_urls_and_keys() {
        let want = "remote:github.com/sample-org/irori";
        for s in [
            "sample-org/irori",
            "https://github.com/sample-org/irori",
            "github.com/sample-org/irori.git",
            "git@github.com:sample-org/irori.git",
            "remote:github.com/sample-org/irori",
            " https://www.github.com/sample-org/irori/ ",
        ] {
            assert_eq!(github_key(s), want, "{s}");
        }
        assert_eq!(github_key("just-a-word"), "");
        assert_eq!(github_key("https://gitlab.com/a/b"), "");
    }

    #[test]
    fn owner_names_and_login_lists() {
        assert_eq!(
            owner_name("remote:github.com/sample-org/irori"),
            "sample-org/irori"
        );
        assert_eq!(owner_name("root:abc"), "root:abc");
        assert_eq!(list("a, b  c,,d"), vec!["a", "b", "c", "d"]);
    }
}
