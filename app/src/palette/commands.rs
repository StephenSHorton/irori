//! The command registry (card #213), the React app's lib/commands.ts and
//! the commands App.tsx, the sidebar, a board's page and the settings menu
//! registered there. React registered them while a component was mounted
//! and filtered them by `when()`; here [`commands`] builds the list from
//! the state as the palette opens, and a command that doesn't apply now
//! is left out. A package adds its own commands by adding them here.

use std::rc::Rc;

use crate::ui::{Dialog, Route, State, theme};

/// What a command does.
pub type Run = Rc<dyn Fn(&mut State)>;

#[derive(Clone)]
pub struct Command {
    /// A stable id ("nav.home"): recents remember it.
    pub id: String,
    /// What the palette shows ("New card").
    pub title: String,
    /// Group heading in the palette (default "Commands").
    pub group: &'static str,
    /// Other words it is found by.
    pub keywords: Vec<&'static str>,
    /// The keys that also run it, as shown: "Mod+K" (Mod is Cmd on a Mac,
    /// Ctrl elsewhere). The palette only shows it.
    pub shortcut: Option<&'static str>,
    /// Where it sits in its group when the query is empty (lower first).
    pub order: i32,
    /// Listed only for a typed query (the Settings sections).
    pub quiet: bool,
    pub run: Run,
}

impl Command {
    fn new(id: &str, title: impl Into<String>, run: impl Fn(&mut State) + 'static) -> Command {
        Command {
            id: id.to_string(),
            title: title.into(),
            group: "Commands",
            keywords: Vec::new(),
            shortcut: None,
            order: 100,
            quiet: false,
            run: Rc::new(run),
        }
    }
    fn go_to(mut self, order: i32) -> Command {
        self.group = "Go to";
        self.order = order;
        self
    }
    fn order(mut self, order: i32) -> Command {
        self.order = order;
        self
    }
    fn words(mut self, k: &[&'static str]) -> Command {
        self.keywords = k.to_vec();
        self
    }
    fn keys(mut self, s: &'static str) -> Command {
        self.shortcut = Some(s);
        self
    }
    fn quiet(mut self) -> Command {
        self.quiet = true;
        self
    }
}

/// Settings' sections (settings-dialog.tsx settingsSections).
pub const SETTINGS_SECTIONS: &[(&str, &str)] = &[
    ("notifications", "Notifications"),
    ("appearance", "Appearance"),
    ("devices", "Devices and connections"),
    ("people", "People and invites"),
    ("agents", "Agents"),
    ("desks", "Merge desks"),
    ("maintenance", "Maintenance"),
    ("app", "App"),
];

/// Goes to a section from anywhere (a card's page included).
fn go_to(s: &mut State, r: Route) {
    s.close_card();
    s.go(r);
}

/// Every command that applies now. `dark` is the theme showing.
pub fn commands(s: &State, dark: bool) -> Vec<Command> {
    let mut out = vec![
        Command::new("nav.home", "Home", |s| go_to(s, Route::Home)).go_to(1).words(&["my cards", "start"]),
        Command::new("nav.talk", "Talk", |s| go_to(s, Route::Talk(None)))
            .go_to(2)
            .words(&["threads", "asks", "conversations", "messages"]),
        Command::new("nav.desk", "Merge desks", |s| go_to(s, Route::Desk(None)))
            .go_to(2)
            .words(&["prs", "pull requests", "review", "github"]),
        Command::new("nav.notes", "Notes", |s| go_to(s, Route::Notes))
            .go_to(2)
            .words(&["rules", "gotchas", "decisions", "knowledge", "memory"]),
        Command::new("nav.boards", "Spaces", |s| go_to(s, Route::Spaces))
            .go_to(2)
            .words(&["shared", "projects", "boards"]),
        Command::new("nav.notifications", "Notifications", |s| go_to(s, Route::Notifications))
            .go_to(3)
            .words(&["mentions", "history", "alerts"]),
        Command::new("nav.held", "Held messages", |s| s.open_dialog(Dialog::Held { from: None }))
            .go_to(3)
            .words(&["inbox", "waiting", "history"]),
        Command::new("nav.profile", "Profile", |s| go_to(s, Route::Profile))
            .go_to(3)
            .words(&["name", "avatar", "colour", "color", "you"]),
    ];

    // New card: on the board or space that is open.
    let on_board = match (&s.nav.route, &s.nav.card) {
        (Route::Board { space, board }, None) => Some((space.clone(), board.clone())),
        _ => None,
    };
    let title = match &on_board {
        Some((_, Some(_))) => "New card on this board",
        Some((_, None)) => "New card in this space",
        None => "New card",
    };
    let target = on_board.as_ref().map(|(space, board)| board.clone().unwrap_or_else(|| space.clone()));
    out.push(
        Command::new("card.new", title, move |s| s.open_dialog(Dialog::NewCard { board: target.clone() }))
            .order(1)
            .words(&["create", "add", "task"]),
    );
    out.push(
        // Goes to Spaces with New space open (App.tsx setNewBoard).
        Command::new("board.new", "New space", crate::spaces::open_new_space)
            .order(2)
            .words(&["create", "add", "join", "invite", "board"]),
    );
    // The open space's own commands (board-page.tsx).
    if let Some((space, _)) = &on_board
        && let Some(b) = s.data.spaces.iter().find(|b| &b.board_id == space)
    {
        let (id, pinned) = (b.board_id.clone(), b.pinned);
        out.push(
            Command::new("board.pin", if pinned { "Unpin this space" } else { "Pin this space" }, move |s| {
                let id = id.clone();
                s.act(
                    "Pin this space",
                    move |w| w.boards_pin(&id, !pinned).map(|_| w.boards_list()),
                    |s, list| s.data.spaces = list,
                );
            })
            .words(&["sidebar", "favourite", "favorite"]),
        );
        // Owners rename the space from its own page.
        if b.owner && on_board.as_ref().is_some_and(|(_, list)| list.is_none()) {
            let name = b.name.clone();
            out.push(Command::new("board.rename", "Rename this space", move |s| {
                crate::spaces::start_space_rename(s, &name)
            }));
        }
    }
    // The cards' page (Home, a board): its List/Canvas switch and search.
    let on_list = s.nav.card.is_none() && matches!(s.nav.route, Route::Home | Route::Board { .. });
    if on_list {
        if crate::canvas::wants_canvas(s) {
            out.push(
                Command::new("view.list", "Switch to List", |s| crate::canvas::set_view(s, "list"))
                    .words(&["grid", "view"]),
            );
        } else {
            out.push(
                Command::new("view.canvas", "Switch to Canvas", |s| crate::canvas::set_view(s, "canvas"))
                    .words(&["flow", "chart", "view"]),
            );
        }
        out.push(
            Command::new("search.cards", "Search cards on this page", crate::board::search::focus)
                .keys("/")
                .words(&["find", "filter", "search"]),
        );
    }
    if s.nav.card.is_some() {
        out.push(Command::new("card.back", "Back to the cards", |s| s.back()).keys("Esc").words(&["close", "leave"]));
    }
    out.push(
        Command::new("agents.wind-down", "Wind down all agents", |s| s.open_dialog(Dialog::WindDown))
            .order(3)
            .words(&["stop", "pause", "save", "end", "shut down", "quit"]),
    );
    let paused = s.data.pause.on;
    out.push(
        Command::new("agents.pause", if paused { "Resume agents" } else { "Pause agents" }, move |s| {
            s.act("Pause agents", move |w| w.agents_pause_set(!paused), |s, v| s.data.pause = v);
        })
        .order(3)
        .words(&["wakes", "stop", "hold", "resume"]),
    );
    out.push(
        Command::new("appearance.theme", if dark { "Switch to light theme" } else { "Switch to dark theme" }, move |s| {
            s.prefs.set(theme::MODE, if dark { "light" } else { "dark" });
        })
        .words(&["theme", "appearance", "dark mode", "light mode", "colours", "colors"]),
    );
    out.push(
        Command::new("settings.open", "Settings", |s| s.open_dialog(Dialog::Settings(String::new())))
            .go_to(4)
            .words(&["preferences", "options", "open settings"]),
    );
    for (id, label) in SETTINGS_SECTIONS {
        out.push(
            Command::new(&format!("settings.{id}"), format!("Settings: {label}"), move |s| {
                s.open_dialog(Dialog::Settings(id.to_string()))
            })
            .go_to(100)
            .quiet()
            .words(&["settings", "preferences"]),
        );
    }
    out
}

/// The keys to show for a shortcut: "Mod+K" is ["⌘", "K"] on a Mac and
/// ["Ctrl", "K"] elsewhere.
pub fn shortcut_keys(s: &str, mac: bool) -> Vec<String> {
    s.split('+')
        .map(|k| match k {
            "Mod" => if mac { "⌘" } else { "Ctrl" }.to_string(),
            "Shift" => if mac { "⇧" } else { "Shift" }.to_string(),
            "Alt" => if mac { "⌥" } else { "Alt" }.to_string(),
            k => k.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_keys_per_platform() {
        assert_eq!(shortcut_keys("Mod+K", true), ["⌘", "K"]);
        assert_eq!(shortcut_keys("Mod+K", false), ["Ctrl", "K"]);
    }

    #[test]
    fn what_applies_is_what_is_listed() {
        // React's register/when()/unregister test: a command that doesn't
        // apply now isn't built.
        let mut s = State::default();
        let ids = |s: &State| commands(s, true).into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert!(ids(&s).contains(&"nav.home".to_string()));
        assert!(!ids(&s).contains(&"card.back".to_string()));
        s.nav.open_card("c".into());
        assert!(ids(&s).contains(&"card.back".to_string()));
        let theme = commands(&s, true).into_iter().find(|c| c.id == "appearance.theme").unwrap();
        assert_eq!(theme.title, "Switch to light theme");
        (theme.run)(&mut s);
        assert_eq!(s.prefs.get::<String>(theme::MODE).as_deref(), Some("light"));
    }

    #[test]
    fn the_cards_page_offers_its_view_and_search() {
        let mut s = State::default();
        s.go(Route::Home);
        let ids = |s: &State| commands(s, true).into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert!(ids(&s).contains(&"view.canvas".to_string()));
        assert!(!ids(&s).contains(&"view.list".to_string()));
        let search = commands(&s, true).into_iter().find(|c| c.id == "search.cards").unwrap();
        assert_eq!(search.shortcut, Some("/"));
        (search.run)(&mut s);
        assert!(s.status.focus_search);
        // Not on Notes, nor over a card.
        s.go(Route::Notes);
        assert!(!ids(&s).contains(&"search.cards".to_string()));
        s.go(Route::Home);
        s.nav.open_card("c".into());
        assert!(!ids(&s).contains(&"view.canvas".to_string()));
    }

    #[test]
    fn new_space_opens_its_dialog() {
        let mut s = State::default();
        let c = commands(&s, true).into_iter().find(|c| c.id == "board.new").unwrap();
        (c.run)(&mut s);
        assert_eq!(s.nav.route, Route::Spaces);
        assert!(s.spaces.new_space.is_some());
    }
}
