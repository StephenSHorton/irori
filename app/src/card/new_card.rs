//! New card (cards/new-card-dialog.tsx, card #116): the person makes a
//! card straight from the window, no ask. Title is all it needs; goal,
//! done-when and the repo say more. With nobody picked Irori starts a
//! fresh agent for it where spawning is on, else hands it to one free
//! agent; picking an agent hands it to that one, woken. Files go with it
//! (the clip, a drop, a pasted screenshot in the goal). From a space it
//! goes on that space, on the board picked (card #558).

use irori_window::agents::AgentsSettings;
use irori_window::board::AskRepo;
use irori_window::boards::{BoardSummary, SpaceBoard};
use irori_window::new_card::NewCardInput;
use rune::{El, TextEdit, Tx, col, row, text, text_input};
use rune_kit::{Button, ButtonVariant, ControlSize, Dialog};

use super::{err_line, spacer, ts};
use crate::logic::names::display_name;
use crate::logic::plain::plain_line;
use crate::logic::spaces::{PERSONAL, boards_of};
use crate::text::{self as tt, Composer, ComposerOpts};
use crate::ui::{Dialog as Dlg, Event, Frame, Route, State, icons};
use crate::widgets::choice::{Choice, choice_select};

/// The boards on offer and the one the card goes on (lib/new-card.ts).
#[derive(Clone, Debug, PartialEq)]
pub struct BoardTarget {
    pub choices: Vec<SpaceBoard>,
    pub on: Option<SpaceBoard>,
    /// What create_card gets: the space (None for Personal) and the board
    /// inside it.
    pub board: Option<String>,
    pub space_board: Option<String>,
}

/// The one picked, else the open list's, else the first. A pick that
/// isn't among the choices (another space's) is ignored.
pub fn board_target(space: Option<&BoardSummary>, personal: &[SpaceBoard], list: Option<&SpaceBoard>, picked: Option<&str>) -> BoardTarget {
    let choices = match space {
        Some(s) => boards_of(s),
        None => personal.to_vec(),
    };
    let find = |id: Option<&str>| id.and_then(|id| choices.iter().find(|x| x.board_id == id).cloned());
    let on = find(picked)
        .or_else(|| find(list.map(|l| l.board_id.as_str())))
        .or_else(|| choices.first().cloned())
        .or_else(|| list.cloned());
    BoardTarget {
        board: space.map(|s| s.board_id.clone()),
        space_board: on.as_ref().map(|o| o.board_id.clone()),
        choices,
        on,
    }
}

#[derive(Default)]
pub struct NewCard {
    /// The dialog this state is for ("<space>"), so a new opening starts
    /// afresh.
    pub opened_for: Option<String>,
    pub title: TextEdit,
    pub goal: Composer,
    pub done_when: TextEdit,
    pub repo_key: String,
    pub assign_to: String,
    pub picked: Option<String>,
    pub repos: Vec<AskRepo>,
    pub spawn: Option<AgentsSettings>,
    pub busy: bool,
    pub error: Option<String>,
}

fn goal(s: &State) -> &Composer {
    &s.card.new_card.goal
}
fn goal_mut(s: &mut State) -> &mut Composer {
    &mut s.card.new_card.goal
}

fn dialog_space(s: &State) -> Option<Option<String>> {
    match &s.dialog {
        Some(Dlg::NewCard { board }) => Some(board.clone()),
        _ => None,
    }
}

/// The space and the board list the dialog was opened from.
fn context(s: &State) -> (Option<&BoardSummary>, Option<SpaceBoard>) {
    let space_id = dialog_space(s).flatten();
    let space = space_id.as_deref().and_then(|id| s.data.spaces.iter().find(|b| b.board_id == id));
    let list = match &s.nav.route {
        Route::Board { space: sp, board: Some(b) } => {
            if let Some(sb) = space {
                if *sp == sb.board_id { boards_of(sb).into_iter().find(|x| x.board_id == *b) } else { None }
            } else if sp == PERSONAL {
                s.card.personal.iter().find(|x| x.board_id == *b).cloned()
            } else {
                None
            }
        }
        _ => None,
    };
    (space, list)
}

/// The dialog opened: start afresh and read the repos, the space's agent
/// settings and Personal's boards.
pub fn opened(s: &mut State) {
    let space = dialog_space(s).flatten().unwrap_or_default();
    s.card.new_card = NewCard {
        opened_for: Some(space.clone()),
        ..NewCard::default()
    };
    // The title takes the keyboard as the dialog opens (autoFocus).
    s.card.new_card.title.focused = true;
    // A Talk message's Make a card fills the form (card #390).
    if let Some((title, objective)) = crate::talk::new_card_from(s) {
        s.card.new_card.title.replace_text(title);
        s.card.new_card.goal.edit.replace_text(objective);
    }
    s.spawn(
        move |w| (w.ask_repos(), w.agents_settings(&space), w.boards_personal().unwrap_or_default()),
        |s, (repos, spawn, personal)| {
            s.card.new_card.repos = repos;
            s.card.new_card.spawn = Some(spawn);
            s.card.personal = personal;
        },
    );
}

/// Notices the dialog opening (the sidebar opens it with no call here).
pub fn watch(s: &mut State) -> bool {
    match dialog_space(s) {
        Some(sp) if s.card.new_card.opened_for.as_deref() != Some(sp.as_deref().unwrap_or("")) => {
            opened(s);
            true
        }
        None if s.card.new_card.opened_for.is_some() => {
            s.card.new_card.opened_for = None;
            false
        }
        _ => false,
    }
}

pub fn on_event(s: &mut State, ev: &Event) {
    if ev.is(irori_window::boards::EVENT_BOARDS_CHANGED) {
        let space = dialog_space(s).flatten().unwrap_or_default();
        s.spawn(
            move |w| (w.agents_settings(&space), w.boards_personal().unwrap_or_default()),
            |s, (spawn, personal)| {
                s.card.new_card.spawn = Some(spawn);
                s.card.personal = personal;
            },
        );
    }
}

fn label(f: &Frame, t: &str) -> El {
    text(t.to_string()).font(f.k.ty.small_medium).color(f.k.c.text_base)
}

fn field(f: &Frame, edit: &TextEdit, key: &str, placeholder: &str, get: fn(&mut State) -> &mut TextEdit) -> El {
    let k = f.k;
    text_input(edit, k.ty.ui)
        .key(key.to_string())
        .w_full()
        .px(10.0)
        .py(8.0)
        .radius(6.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_02)
        .color(k.c.text_base)
        .placeholder(placeholder.to_string(), k.c.text_faint)
        .on_edit(rune::edit_access(get))
        .on_submit(|s: &mut State, _tx: &mut Tx| create(s))
}

fn model_label(v: &str) -> String {
    match v {
        "sonnet" => "Sonnet".into(),
        "opus" => "Opus".into(),
        "haiku" => "Haiku".into(),
        _ => v.to_string(),
    }
}

fn effort_label(v: &str) -> String {
    match v {
        "low" => "Low".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra high".into(),
        "max" => "Max".into(),
        _ => v.to_string(),
    }
}

fn create(s: &mut State) {
    let n = &s.card.new_card;
    if n.busy {
        return;
    }
    if n.title.text.trim().is_empty() {
        s.card.new_card.error = Some("Give the card a title.".into());
        return;
    }
    let (space, list) = context(s);
    let t = board_target(space, &s.card.personal, list.as_ref(), n.picked.as_deref());
    let input = NewCardInput {
        title: n.title.text.clone(),
        objective: n.goal.edit.text.clone(),
        done_when: n.done_when.text.clone(),
        repo_key: n.repo_key.clone(),
        assign_to: n.assign_to.clone(),
        board: t.board.unwrap_or_default(),
        space_board: t.space_board.unwrap_or_default(),
        files: n.goal.files.iter().map(|p| p.upload()).collect(),
    };
    s.card.new_card.busy = true;
    s.card.new_card.error = None;
    s.spawn(
        move |w| w.create_card(input),
        |s, r| {
            s.card.new_card.busy = false;
            match r {
                Ok(res) => {
                    // Made from a Talk message, the message links to it
                    // (onMade) and the conversation stays open.
                    let talk = crate::talk::card_made(s, &res.task.id);
                    tt::sent(s, goal_mut);
                    s.card.new_card = NewCard::default();
                    s.close_dialog();
                    if !talk {
                        s.open_card(&res.task.id);
                    }
                }
                Err(e) => s.card.new_card.error = Some(e),
            }
        },
    );
}

/// The dialog, while `Dialog::NewCard` is open.
pub fn new_card_dialog(s: &State, f: &mut Frame) {
    if dialog_space(s).is_none() {
        return;
    }
    let k = f.k;
    let n = &s.card.new_card;
    let (space, list) = context(s);
    let t = board_target(space, &s.card.personal, list.as_ref(), n.picked.as_deref());
    let on = t.on.clone();
    // The space, showing the board it goes on.
    let board_name = match (space, &on) {
        (Some(_), Some(o)) => Some((o.name.clone(), o.tag.clone())),
        (Some(sp), None) => Some((sp.name.clone(), sp.tag.clone())),
        _ => None,
    };
    let fresh = n.spawn.as_ref().is_some_and(|x| x.on);
    let giving = n.assign_to.starts_with("person:");
    let members: Vec<(String, String)> = space
        .map(|sp| sp.members.iter().filter(|m| !m.me).map(|m| (m.person_id.clone(), plain_line(&m.name))).collect())
        .unwrap_or_default();
    let spawned: std::collections::HashSet<&str> = s.data.agents.iter().filter(|a| a.spawned).map(|a| a.card.key.as_str()).collect();
    let agents: Vec<&crate::text::mentions::Candidate> = s
        .text
        .cands
        .iter()
        .filter(|c| c.who.live && c.board.is_empty() && c.card.is_empty() && !spawned.contains(c.who.key.as_str()))
        .collect();

    let title = match &board_name {
        Some((name, _)) => format!("New card in {}", plain_line(name)),
        None => "New card".into(),
    };
    let mut desc = match (&board_name, space) {
        (Some((_, tag)), Some(sp)) => format!(
            "Goes on {}#, visible to {}.{}",
            plain_line(tag),
            sp.people.iter().map(|p| plain_line(p)).collect::<Vec<_>>().join(", "),
            if giving {
                ""
            } else if fresh {
                " Irori starts your agent for it, unless you give it to someone."
            } else {
                " Nobody picked: it waits there for someone in the space to take it up."
            }
        ),
        _ if fresh => "Work for an agent. Irori starts a fresh one for it, unless you give it to one of yours.".into(),
        _ => "Work for an agent to pick up. Nobody picked: Irori hands it to an idle agent with Offer it cards on.".into(),
    };
    if space.is_none()
        && let Some(o) = &on
        && (list.is_some() || t.choices.len() > 1)
    {
        desc.push_str(&format!(" Goes on {}.", plain_line(&o.name)));
    }

    let mut body = col()
        .key("card/new-body")
        .w_full()
        .gap(12.0)
        .px(16.0)
        .pb(16.0)
        .child(text(desc).font(k.ty.ui).color(k.c.text_muted).w_full().wrap())
        .child(col().w_full().gap(6.0).child(label(f, "Title")).child(field(f, &n.title, "card/new-title", "What needs doing", |s| {
            &mut s.card.new_card.title
        })))
        .child(
            col().w_full().gap(6.0).child(label(f, "Goal")).child(tt::composer(
                s,
                f,
                "card/new-goal",
                goal,
                goal_mut,
                &ComposerOpts {
                    placeholder: "What and why (optional)".into(),
                    label: "Goal".into(),
                    no_mentions: Some(String::new()),
                    disabled: n.busy,
                    rows: Some((2, 8)),
                    popup_below: true,
                    on_send: None,
                    ..ComposerOpts::default()
                },
            )),
        )
        .child(col().w_full().gap(6.0).child(label(f, "Done when")).child(field(
            f,
            &n.done_when,
            "card/new-done",
            "How to tell it's done (optional)",
            |s| &mut s.card.new_card.done_when,
        )));
    if t.choices.len() > 1
        && let Some(o) = &on
    {
        let choices: Vec<Choice> = t
            .choices
            .iter()
            .map(|x| {
                let name = plain_line(&x.name);
                let name = if name.is_empty() { "(unnamed)".to_string() } else { name };
                Choice::new(
                    x.board_id.clone(),
                    if x.tag.is_empty() { name } else { format!("{name} · {}#", plain_line(&x.tag)) },
                )
            })
            .collect();
        body = body.child(col().w_full().gap(6.0).child(label(f, "Board")).child(choice_select(
            s,
            f,
            "card/new-board",
            "Board",
            &o.board_id,
            &choices,
            n.busy,
            460.0,
            |s, v| s.card.new_card.picked = Some(v.to_string()),
        )));
    }
    let mut repo_choices = vec![Choice::new("", "No repo")];
    repo_choices.extend(n.repos.iter().map(|r| Choice::new(r.repo_key.clone(), r.label.clone())));
    let nobody = if fresh {
        "A fresh agent"
    } else if space.is_some() {
        "Nobody yet"
    } else {
        "Any free agent"
    };
    let mut who_choices = vec![Choice::new("", nobody)];
    who_choices.extend(members.iter().map(|(id, name)| {
        Choice::new(format!("person:{id}"), if name.is_empty() { "A member".to_string() } else { name.clone() })
    }));
    who_choices.extend(agents.iter().map(|c| {
        Choice::new(
            c.who.key.clone(),
            if c.repo.is_empty() { display_name(&c.who) } else { format!("{} · {}", display_name(&c.who), c.repo) },
        )
    }));
    body = body.child(
        row()
            .w_full()
            .gap(12.0)
            .child(col().grow(1.0).min_w(0.0).gap(6.0).child(label(f, "Repo")).child(choice_select(
                s,
                f,
                "card/new-repo",
                "Repo",
                &n.repo_key,
                &repo_choices,
                n.busy,
                224.0,
                |s, v| s.card.new_card.repo_key = v.to_string(),
            )))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(6.0)
                    .child(label(f, if space.is_some() { "Give to" } else { "Agent" }))
                    .child(choice_select(
                        s,
                        f,
                        "card/new-agent",
                        if space.is_some() { "Give to" } else { "Agent" },
                        &n.assign_to,
                        &who_choices,
                        n.busy,
                        224.0,
                        |s, v| s.card.new_card.assign_to = v.to_string(),
                    )),
            ),
    );
    let info = |key: &str, child: El| {
        row()
            .key(key.to_string())
            .w_full()
            .gap(8.0)
            .items_start()
            .px(12.0)
            .py(8.0)
            .radius(6.0)
            .border(1.0, k.c.border_base)
            .fill(k.c.layer_02)
            .child(child)
    };
    let small = |t: &str| text(t.to_string()).font(ts(12.5, 400.0)).color(k.c.text_muted).w_full().wrap();
    let settings_link = |label: &str| {
        text(label.to_string())
            .font(ts(12.5, 500.0))
            .color(k.c.text_accent)
            .role("link")
            .cursor(rune::Cursor::Pointer)
            .on_press(|s: &mut State| s.open_dialog(Dlg::Settings("agents".into())))
    };
    if giving {
        body = body.child(info(
            "card/new-giving",
            small("It's theirs to pick up. Their Irori asks them before it starts an agent on it."),
        ));
    }
    if space.is_some() && n.spawn.as_ref().is_some_and(|x| !x.on) && n.assign_to.is_empty() {
        body = body.child(info(
            "card/new-off",
            col()
                .grow(1.0)
                .gap(2.0)
                .child(small("Irori won't start an agent for this card: your agents are off in this space."))
                .child(settings_link("Turn them on in Settings › Agents")),
        ));
    }
    if fresh
        && let Some(sp) = &n.spawn
        && n.assign_to.is_empty()
    {
        let what = format!("{} · {}", model_label(&sp.model), effort_label(&sp.effort).to_lowercase());
        let line = format!("A fresh agent starts for this card with {what} and ends when it's done.");
        let at = line.find(&what).unwrap_or(0);
        body = body.child(info(
            "card/new-fresh",
            row()
                .grow(1.0)
                .gap(8.0)
                .items_start()
                .child(rune::icon(&icons::ZAP, 14.0).color(k.c.icon_accent).mt(2.0))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            rune::rich(line.clone(), vec![rune::Span::new(at, at + what.len()).color(k.c.text_base)])
                                .font(ts(12.5, 400.0))
                                .color(k.c.text_muted)
                                .w_full()
                                .wrap(),
                        )
                        .child(settings_link("Change in Settings › Agents")),
                ),
        ));
    }
    if let Some(e) = &n.error {
        body = body.child(err_line(f, e.clone()));
    }
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .disabled(n.busy)
                    .on_press(|s: &mut State| s.close_dialog())
                    .build(k),
            )
            .child(
                Button::new(if n.busy { "Making it…" } else { "Make the card" })
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .disabled(n.busy)
                    .on_press(create)
                    .build(k),
            ),
    );
    Dialog::new("card/new", title)
        .open(true)
        .width(520.0)
        .fit()
        .child(body)
        .on_close(|s: &mut State| {
            if !s.card.new_card.busy {
                s.close_dialog();
            }
        })
        .build(k, f.layer);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb(id: &str, name: &str, tag: &str) -> SpaceBoard {
        SpaceBoard {
            board_id: id.into(),
            name: name.into(),
            tag: tag.into(),
        }
    }

    fn space() -> BoardSummary {
        BoardSummary {
            board_id: "s".into(),
            name: "MLG".into(),
            tag: "mlg".into(),
            boards: vec![sb("s", "", ""), sb("n", "Netcode", "net")],
            ..BoardSummary::default()
        }
    }

    fn personal() -> Vec<SpaceBoard> {
        vec![sb(PERSONAL, "Personal", ""), sb("p-home", "House", "home")]
    }

    fn ids(t: &BoardTarget) -> Vec<&str> {
        t.choices.iter().map(|x| x.board_id.as_str()).collect()
    }

    #[test]
    fn from_a_boards_list_that_board() {
        let sp = space();
        let t = board_target(Some(&sp), &personal(), Some(&sp.boards[1]), None);
        assert_eq!(ids(&t), ["s", "n"]);
        assert_eq!(t.on.as_ref().unwrap().board_id, "n");
        assert_eq!((t.board.as_deref(), t.space_board.as_deref()), (Some("s"), Some("n")));
    }

    #[test]
    fn from_the_spaces_page_its_first_board_named_as_the_space() {
        let sp = space();
        let t = board_target(Some(&sp), &personal(), None, None);
        assert_eq!(t.on, Some(sb("s", "MLG", "mlg")));
        assert_eq!((t.board.as_deref(), t.space_board.as_deref()), (Some("s"), Some("s")));
    }

    #[test]
    fn a_pick_wins_a_pick_from_elsewhere_is_ignored() {
        let sp = space();
        assert_eq!(board_target(Some(&sp), &personal(), None, Some("n")).space_board.as_deref(), Some("n"));
        assert_eq!(
            board_target(Some(&sp), &personal(), Some(&sp.boards[1]), Some("p-home")).space_board.as_deref(),
            Some("n")
        );
    }

    #[test]
    fn a_space_with_one_board_has_one_choice() {
        let one = BoardSummary {
            board_id: "o".into(),
            name: "Solo".into(),
            tag: "solo".into(),
            ..BoardSummary::default()
        };
        let t = board_target(Some(&one), &personal(), None, None);
        assert_eq!(t.choices.len(), 1);
        assert_eq!((t.board.as_deref(), t.space_board.as_deref()), (Some("o"), Some("o")));
    }

    #[test]
    fn no_space_personals_boards_the_card_private() {
        let p = personal();
        let t = board_target(None, &p, None, None);
        assert_eq!(ids(&t), [PERSONAL, "p-home"]);
        assert_eq!((t.board.as_deref(), t.space_board.as_deref()), (None, Some(PERSONAL)));
        assert_eq!(board_target(None, &p, Some(&p[1]), None).space_board.as_deref(), Some("p-home"));
        assert_eq!(board_target(None, &p, None, Some("p-home")).space_board.as_deref(), Some("p-home"));
    }

    #[test]
    fn personals_boards_not_loaded_yet_the_open_list_else_nothing() {
        let p = personal();
        assert_eq!(board_target(None, &[], Some(&p[1]), None).space_board.as_deref(), Some("p-home"));
        assert_eq!(board_target(None, &[], None, None).space_board, None);
    }
}
