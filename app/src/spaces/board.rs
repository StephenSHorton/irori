//! A space's page and one board's page (board-page.tsx `BoardPage`,
//! `BoardHeaderBar`, `BoardActionsMenu`, `BoardNotes`; space-boards.tsx
//! `SpaceBoards`, `ListHeaderBar`, `PersonalList`, `PersonalHeaderBar`,
//! `ListHeader`): `Route::Board { space, board }`.

use rune::{El, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant, Menu, MenuItem, Placement, TextInput};

use irori_window::boards::{BoardSummary, SpaceBoard};
use irori_window::cards::CardSummary;

use super::logic::{self, PERSONAL, boards_of, plain_line};
use super::page::{add_board, board_tile, clamp, tile_width};
use super::ui::{self as u, t12, t125};
use super::{Ask, Header, parts};
use crate::ui::{Dialog, Frame, Route, State, icons};

/// What a header's rename and delete act on.
#[derive(Clone, Debug)]
enum Target {
    /// The space itself (owners rename it).
    Space(String),
    /// One board in a shared space.
    List { space: String, board: String },
    /// One of Personal's boards.
    Personal(String),
}

/// The header state, when it belongs to the page now showing.
fn header<'a>(s: &'a State, list: bool) -> Option<&'a Header> {
    let h = if list { &s.spaces.list_header } else { &s.spaces.header };
    (h.at.as_ref() == Some(&s.nav.route)).then_some(h)
}

/// The header state for the page now showing, started clean when it was
/// another page's.
fn header_mut(s: &mut State, list: bool) -> &mut Header {
    let route = s.nav.route.clone();
    let h = if list { &mut s.spaces.list_header } else { &mut s.spaces.header };
    if h.at.as_ref() != Some(&route) {
        *h = Header {
            at: Some(route),
            ..Header::default()
        };
    }
    h
}

fn title_font() -> rune::tok::TextStyle {
    rune::tok::TextStyle::new(18.0, 600.0)
}

fn bar(key: &str) -> El {
    row()
        .key(key.to_string())
        .w_full()
        .h(48.0)
        .px(20.0)
        .gap(12.0)
        .center_y()
}

/// `Route::Board`: a space's page, one of its boards, or one of Personal's
/// boards.
pub fn board_page(s: &State, f: &mut Frame) -> El {
    let Route::Board { space, board } = &s.nav.route else {
        return el();
    };
    if space == PERSONAL {
        return personal_page(s, f, board.as_deref().unwrap_or(PERSONAL));
    }
    let k = f.k;
    let Some(b) = s.data.spaces.iter().find(|x| x.board_id == *space) else {
        return col()
            .key("spaces-board-page")
            .w_full()
            .h_full()
            .child(bar("spaces-board-header").child(text("Space").font(title_font()).color(k.c.text_base)))
            .child(
                col()
                    .px(24.0)
                    .gap(8.0)
                    .child(
                        Button::new("Spaces")
                            .variant(ButtonVariant::Ghost)
                            .icon(&icons::ARROW_LEFT)
                            .on_press(|s: &mut State| s.go(Route::Spaces))
                            .build(k)
                            .self_start(),
                    )
                    .child(u::line(
                        "This space isn't here any more: you may have left or forgotten it.",
                        rune::tok::TextStyle::new(14.0, 400.0),
                        k.c.text_muted,
                    )),
            );
    };
    let open = board
        .as_deref()
        .and_then(|l| boards_of(b).into_iter().find(|x| x.board_id == l));
    let head = match &open {
        Some(x) => {
            let first = boards_of(b).into_iter().next();
            let remove = first.filter(|fb| fb.board_id != x.board_id).map(|fb| fb.name);
            let space_name = plain_line(&b.name);
            list_header(
                s,
                f,
                if space_name.is_empty() { "(unnamed)".into() } else { space_name },
                Route::Board {
                    space: b.board_id.clone(),
                    board: None,
                },
                x,
                Target::List {
                    space: b.board_id.clone(),
                    board: x.board_id.clone(),
                },
                remove,
            )
        }
        None => space_header(s, f, b),
    };
    let mut top = col().key("spaces-board-notes").w_full().gap(8.0).px(24.0);
    let mut any = false;
    for n in board_notes(s, f, b) {
        top = top.child(n);
        any = true;
    }
    if let Some(off) = parts::agents_off(s, f, &b.board_id) {
        top = top.child(off);
        any = true;
    }
    // The same board list as Home (card #161), over this board's cards,
    // with who is on it beside them; a space's own page is plain: its
    // boards as tiles, and its Threads panel.
    let empty = match &open {
        Some(_) => muted(
            f,
            "No cards on this board yet. Make one with New card, or move one here from its page.",
        ),
        None => space_boards(s, f, b),
    };
    let cards = match &open {
        Some(x) => logic::cards_on_board(b, &s.data.cards, &x.board_id),
        None => vec![],
    };
    let ask = crate::home::ask::ask_box(s, f);
    let list = crate::board::board_view(
        s,
        f,
        crate::board::Opts {
            scope: b.board_id.clone(),
            cards,
            header: Some(ask),
            empty: Some(empty),
            nothing_decide: Some("Nothing on this board waits on you.".into()),
            threads: Some(b.board_id.clone()),
            plain: open.is_none(),
            ..Default::default()
        },
    );
    let new_on = Some(open.as_ref().map_or_else(|| b.board_id.clone(), |x| x.board_id.clone()));
    parts::settings_dialog(s, f, b);
    let mut page = col()
        .key("spaces-board-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(head.child(new_card(f, new_on)));
    if any {
        page = page.child(top.pb(4.0));
    }
    page.child(body(list))
}

/// The board list under a page's header, inset as the header is.
fn body(list: El) -> El {
    col()
        .key("spaces-board-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(20.0)
        .pt(8.0)
        .child(list.grow(1.0).min_h(0.0))
}

/// New card, on the open board (or the space's first board, or
/// Personal's default when `board` is None).
fn new_card(f: &Frame, board: Option<String>) -> El {
    Button::new("New card")
        .variant(ButtonVariant::Outline)
        .icon(&icons::PLUS)
        .on_press(move |s: &mut State| s.open_dialog(Dialog::NewCard { board: board.clone() }))
        .build(f.k)
        .key("spaces-new-card")
}

/// What an empty board says.
fn muted(f: &Frame, msg: &str) -> El {
    u::line(msg, t125(f.k), f.k.c.text_muted).key("spaces-list-empty")
}

/// What the page says about the space under its header: when it last
/// synced and with whom (card #580), a tag that clashes with another
/// board's, and the space's warnings.
fn board_notes(s: &State, f: &Frame, b: &BoardSummary) -> Vec<El> {
    let k = f.k;
    let mut v = Vec::new();
    if let Some(l) = u::fresh_line(s, k, &b.board_id) {
        v.push(l);
    }
    if b.tag_clash {
        v.push(u::line(
            format!(
                "Another board is also {}#: its refs need the card's id.",
                plain_line(&b.tag)
            ),
            t12(k),
            u::danger(k),
        ));
    }
    for w in &b.warnings {
        v.push(u::line(plain_line(w), t12(k), u::warning(k)));
    }
    v
}

/// What a space opens on: every board as a tile, and Add board.
fn space_boards(s: &State, f: &Frame, b: &BoardSummary) -> El {
    let k = f.k;
    let mut head = row().gap(8.0).center_y().child(u::heading(k, "Boards"));
    if b.ready {
        head = head.child(add_board(s, f, &b.board_id));
    }
    let w = tile_width(f, "spaces-space-tiles", 260.0, 12.0);
    let mut tiles = row().key("spaces-space-tiles").w_full().gap(12.0).flex_wrap();
    for (i, x) in boards_of(b).iter().enumerate() {
        let cards = logic::cards_on_board(b, &s.data.cards, &x.board_id);
        tiles = tiles.child(board_tile(
            f,
            x,
            i == 0,
            &cards,
            w,
            Route::Board {
                space: b.board_id.clone(),
                board: Some(x.board_id.clone()),
            },
        ));
    }
    col()
        .key("spaces-space-boards")
        .w_full()
        .gap(8.0)
        .pt(4.0)
        .label("Boards in this space")
        .child(head)
        .child(tiles)
}

/// One of Personal's boards open: the same list a space's board has, over
/// the person's own cards on it.
fn personal_page(s: &State, f: &mut Frame, list: &str) -> El {
    let k = f.k;
    let x = s.spaces.personal.iter().find(|b| b.board_id == list).cloned();
    let head = match &x {
        None => bar("spaces-board-header").child(text("Personal").font(title_font()).color(k.c.text_base)),
        Some(x) => {
            let remove = (x.board_id != PERSONAL).then(|| {
                s.spaces
                    .personal
                    .first()
                    .map_or_else(|| "Personal".to_string(), |d| d.name.clone())
            });
            list_header(
                s,
                f,
                "Personal".into(),
                Route::Spaces,
                x,
                Target::Personal(x.board_id.clone()),
                remove,
            )
        }
    };
    let cards: Vec<&CardSummary> = logic::cards_on_personal(&s.data.cards, list);
    let empty = muted(f, "No cards on this board yet. Make one with New card.");
    let lst = crate::board::board_view(
        s,
        f,
        crate::board::Opts {
            scope: crate::board::SCOPE_OWN.into(),
            cards,
            empty: Some(empty),
            ..Default::default()
        },
    );
    let new_on = (list != PERSONAL).then(|| list.to_string());
    col()
        .key("spaces-board-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(head.child(new_card(f, new_on)))
        .child(body(lst))
}

// ---- renaming -----------------------------------------------------------------

fn start_rename(s: &mut State, list: bool, name: &str) {
    let h = header_mut(s, list);
    h.error = None;
    h.menu = false;
    h.renaming = true;
    h.name = rune::TextEdit::new(name);
    h.name.focused = true;
}

/// The palette's "Rename this space", on the space's page.
pub fn start_space_rename(s: &mut State, name: &str) {
    start_rename(s, false, name);
}

fn save_rename(s: &mut State, list: bool, target: Target) {
    let name = header_mut(s, list).name.text.clone();
    if name.trim().is_empty() {
        return;
    }
    header_mut(s, list).error = None;
    let at = s.nav.route.clone();
    s.spawn(
        move |w| match &target {
            Target::Space(id) => w.boards_rename(id, &name).map(|_| ()),
            Target::List { space, board } => w.boards_rename_in_space(space, board, &name).map(|_| ()),
            Target::Personal(board) => w.boards_rename_in_personal(board, &name).map(|_| ()),
        },
        move |s, r| {
            if s.nav.route != at {
                return;
            }
            match r {
                Ok(()) => {
                    header_mut(s, list).renaming = false;
                    super::reload_spaces(s);
                    super::reload_personal(s);
                }
                Err(e) => header_mut(s, list).error = Some(logic::error_text(&e)),
            }
        },
    );
}

/// The rename form: the name, Save and Cancel.
fn rename_form(s: &State, f: &Frame, list: bool, label: &str, target: Target) -> El {
    let k = f.k;
    let Some(h) = header(s, list) else { return el() };
    let t2 = target.clone();
    row()
        .key("spaces-rename")
        .gap(6.0)
        .center_y()
        .child(
            TextInput::new(&h.name)
                .label(label.to_string())
                .width(Some(256.0))
                .on_edit(move |s: &mut State| &mut header_mut(s, list).name)
                .on_change(move |s: &mut State, _| clamp(&mut header_mut(s, list).name, logic::MAX_BOARD_NAME))
                .on_submit(move |s: &mut State, _| save_rename(s, list, t2.clone()))
                .build(k)
                .key("spaces-rename-name"),
        )
        .child(
            Button::new("Save")
                .disabled(h.name.text.trim().is_empty())
                .on_press(move |s: &mut State| save_rename(s, list, target.clone()))
                .build(k),
        )
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(move |s: &mut State| header_mut(s, list).renaming = false)
                .build(k),
        )
}

fn error_text(f: &Frame, e: &str) -> El {
    text(plain_line(e))
        .font(t12(f.k))
        .color(u::danger(f.k))
        .max_lines(1)
        .shrink()
        .min_w(0.0)
}

// ---- a space's header -----------------------------------------------------------

/// A space's page header (card #135): its state dot, name, its boards and
/// people, and the actions on the space: Pin, the settings cog, Rename
/// (owners), and Leave and Forget in its ⋯ menu.
fn space_header(s: &State, f: &mut Frame, b: &BoardSummary) -> El {
    let k = f.k;
    let h = header(s, false);
    let renaming = h.is_some_and(|h| h.renaming);
    let ok = b.ready && b.warnings.is_empty();
    let name = plain_line(&b.name);
    let name = if name.is_empty() { "(unnamed)".to_string() } else { name };
    let id = b.board_id.clone();
    let mut r = bar("spaces-board-header");
    r = if renaming {
        r.child(rename_form(s, f, false, "Space name", Target::Space(id.clone())))
    } else {
        r.child(
            row()
                .gap(8.0)
                .center_y()
                .min_w(0.0)
                .shrink()
                .child(u::dot(k, ok))
                .child(text(name.clone()).font(title_font()).color(k.c.text_base).max_lines(1).shrink()),
        )
    };
    let n = boards_of(b).len();
    r = r.child(
        text(if b.ready {
            format!(
                "{} · {}",
                if n == 1 { "1 board".to_string() } else { format!("{n} boards") },
                b.people.iter().map(|p| plain_line(p)).collect::<Vec<_>>().join(", ")
            )
        } else {
            "Its cards are still arriving from your other device".into()
        })
        .font(t12(k))
        .color(k.c.text_muted)
        .max_lines(1)
        .shrink()
        .min_w(0.0),
    );
    if let Some(e) = h.and_then(|h| h.error.as_ref()) {
        r = r.child(error_text(f, e));
    }
    r = r.child(el().grow(1.0));
    let pinned = b.pinned;
    let pin_id = id.clone();
    let mut acts = row().gap(6.0).center_y().child(
        Button::new(if pinned { "Unpin" } else { "Pin" })
            .variant(ButtonVariant::Ghost)
            .icon(if pinned { &icons::PIN_OFF } else { &icons::PIN })
            .help(if pinned {
                "Take it out of the sidebar"
            } else {
                "Keep it in the sidebar, on all your devices"
            })
            .on_press(move |s: &mut State| {
                let id = pin_id.clone();
                header_mut(s, false).error = None;
                let at = s.nav.route.clone();
                s.spawn(
                    move |w| w.boards_pin(&id, !pinned),
                    move |s, r| match r {
                        Ok(()) => super::reload_spaces(s),
                        Err(e) if s.nav.route == at => header_mut(s, false).error = Some(logic::error_text(&e)),
                        Err(_) => {}
                    },
                );
            })
            .build(k)
            .checked(pinned),
    );
    if b.ready {
        let sid = id.clone();
        acts = acts.child(
            IconButton::new(&icons::SETTINGS)
                .variant(IconButtonVariant::Ghost)
                .label("Space settings")
                .help("Space settings")
                .on_press(move |s: &mut State| parts::open_settings(s, &sid))
                .build(k)
                .key("spaces-settings-button"),
        );
    }
    if b.owner && !renaming {
        let n = b.name.clone();
        acts = acts.child(
            Button::new("Rename")
                .variant(ButtonVariant::Ghost)
                .on_press(move |s: &mut State| start_rename(s, false, &n))
                .build(k),
        );
    }
    acts = acts.child(space_menu(f, b, h));
    r.child(acts)
}

/// The header's ⋯ menu: the actions that take the space away, each behind
/// its confirm step.
fn space_menu(f: &mut Frame, b: &BoardSummary, h: Option<&Header>) -> El {
    let k = f.k;
    let open = h.is_some_and(|h| h.menu);
    let trigger = IconButton::new(&icons::MORE)
        .variant(IconButtonVariant::Ghost)
        .label("Space actions")
        .help("Space actions")
        .on_press(|s: &mut State| {
            let h = header_mut(s, false);
            h.menu = !h.menu;
        })
        .build(k);
    let menu = Menu::new("spaces-space-menu")
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomEnd)
        .on_close(|s: &mut State| header_mut(s, false).menu = false)
        .item(MenuItem::new("Leave the space").danger(true).on_select(|s: &mut State| {
            let h = header_mut(s, false);
            h.menu = false;
            h.asking = Some(Ask::Leave);
        }))
        .item(MenuItem::new("Forget here").danger(true).on_select(|s: &mut State| {
            let h = header_mut(s, false);
            h.menu = false;
            h.asking = Some(Ask::ForgetHere);
        }))
        .build(k, f.layer);
    let name = plain_line(&b.name);
    let cancel = |s: &mut State| header_mut(s, false).asking = None;
    let id = b.board_id.clone();
    match h.and_then(|h| h.asking.clone()) {
        Some(Ask::Leave) => {
            let q = if b.people.len() > 1 {
                format!(
                    "Leave {name}? It goes from every one of your devices now, and nothing you write there counts after it. The others hear that you left when their Irori next reaches one of your devices. Coming back needs a new invite."
                )
            } else {
                format!(
                    "Leave {name}? It goes from every one of your devices, with its cards. Coming back needs a new space."
                )
            };
            u::confirm(f, "spaces-leave", "Leave the space", &q, true, move |s: &mut State| {
                header_mut(s, false).asking = None;
                let id = id.clone();
                s.spawn(
                    move |w| w.boards_leave(&id),
                    |s, r| match r {
                        Ok(()) => {
                            super::reload_spaces(s);
                            s.go(Route::Spaces);
                        }
                        Err(e) => header_mut(s, false).error = Some(logic::error_text(&e)),
                    },
                );
            }, cancel);
        }
        Some(Ask::ForgetHere) => {
            let q = format!(
                "Forget {name} on this device only? Its file goes from here; your other devices keep it."
            );
            u::confirm(f, "spaces-forget", "Forget on this device", &q, false, move |s: &mut State| {
                header_mut(s, false).asking = None;
                let id = id.clone();
                s.spawn(
                    move |w| w.boards_forget(&id),
                    |s, r| match r {
                        Ok(()) => {
                            super::reload_spaces(s);
                            s.go(Route::Spaces);
                        }
                        Err(e) => header_mut(s, false).error = Some(logic::error_text(&e)),
                    },
                );
            }, cancel);
        }
        _ => {}
    }
    menu
}

// ---- one board's header -----------------------------------------------------------

/// The header over one board's list: the space it's in (back to the
/// space's page), its name and tag, Rename (any member, card #558) and
/// Delete (card #563; not the default board: `remove` is the board its
/// cards would go to).
fn list_header(
    s: &State,
    f: &mut Frame,
    space: String,
    back: Route,
    x: &SpaceBoard,
    target: Target,
    remove: Option<String>,
) -> El {
    let k = f.k;
    let h = header(s, true);
    let renaming = h.is_some_and(|h| h.renaming);
    let mut r = bar("spaces-board-header").gap(6.0).child(
        Button::new(space)
            .variant(ButtonVariant::Ghost)
            .on_press(move |s: &mut State| s.go(back.clone()))
            .build(k),
    )
    .child(icon(&icons::CHEVRON_RIGHT, 16.0).color(k.c.text_muted));
    let name = plain_line(&x.name);
    let name = if name.is_empty() { "(unnamed)".to_string() } else { name };
    if renaming {
        r = r.child(rename_form(s, f, true, "Board name", target.clone()));
    } else {
        let mut t = row()
            .gap(8.0)
            .center_y()
            .min_w(0.0)
            .shrink()
            .child(text(name.clone()).font(title_font()).color(k.c.text_base).max_lines(1).shrink());
        if !x.tag.is_empty() {
            t = t.child(
                text(format!("{}#", plain_line(&x.tag)))
                    .font(rune::tok::TextStyle::new(14.0, 400.0).mono())
                    .color(k.c.text_muted),
            );
        }
        r = r.child(t);
    }
    if let Some(e) = h.and_then(|h| h.error.as_ref()) {
        r = r.child(error_text(f, e));
    }
    if renaming {
        return r;
    }
    let n = x.name.clone();
    let mut acts = row().ml(0.0).gap(6.0).center_y().child(
        Button::new("Rename")
            .variant(ButtonVariant::Ghost)
            .on_press(move |s: &mut State| start_rename(s, true, &n))
            .build(k),
    );
    if let Some(into) = remove {
        let open = h.is_some_and(|h| h.menu);
        let into2 = into.clone();
        let trigger = IconButton::new(&icons::MORE)
            .variant(IconButtonVariant::Ghost)
            .label("Board actions")
            .help("Board actions")
            .on_press(|s: &mut State| {
                let h = header_mut(s, true);
                h.menu = !h.menu;
            })
            .build(k);
        acts = acts.child(
            Menu::new("spaces-list-menu")
                .trigger(trigger)
                .open(open)
                .placement(Placement::BottomEnd)
                .on_close(|s: &mut State| header_mut(s, true).menu = false)
                .item(MenuItem::new("Delete this board").danger(true).on_select(move |s: &mut State| {
                    let h = header_mut(s, true);
                    h.menu = false;
                    h.asking = Some(Ask::DeleteBoard { into: into2.clone() });
                }))
                .build(k, f.layer),
        );
        if let Some(Ask::DeleteBoard { into }) = h.and_then(|h| h.asking.clone()) {
            let q = format!(
                "Delete {name}? Its cards move to {}, the default board, and take new numbers there. Their old numbers still find them.",
                plain_line(&into)
            );
            let t = target.clone();
            u::confirm(
                f,
                "spaces-delete-board",
                "Delete the board",
                &q,
                true,
                move |s: &mut State| delete_board(s, t.clone()),
                |s: &mut State| header_mut(s, true).asking = None,
            );
        }
    }
    r.child(el().grow(1.0)).child(acts)
}

fn delete_board(s: &mut State, target: Target) {
    header_mut(s, true).asking = None;
    let back = match &target {
        Target::List { space, .. } => Route::Board {
            space: space.clone(),
            board: None,
        },
        _ => Route::Spaces,
    };
    s.spawn(
        move |w| match &target {
            Target::List { space, board } => w.boards_delete_in_space(space, board).map(|_| ()),
            Target::Personal(board) => w.boards_delete_in_personal(board).map(|_| ()),
            Target::Space(_) => Ok(()),
        },
        move |s, r| match r {
            Ok(()) => {
                super::reload_spaces(s);
                super::reload_personal(s);
                s.go(back);
            }
            Err(e) => header_mut(s, true).error = Some(logic::error_text(&e)),
        },
    );
}
