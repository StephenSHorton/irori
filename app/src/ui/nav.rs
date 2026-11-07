//! Where the window is (the React app's lib/nav.ts `Section` and its
//! history): a [`Route`] with Back and Forward, the open card's page over
//! it, and the open dialog.

use super::prefs::Prefs;

/// A page in the window (the sidebar's sections).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Route {
    /// Your board: Needs you, then the cards.
    #[default]
    Home,
    /// Talk, with a conversation open (a channel or DM id).
    Talk(Option<String>),
    Notes,
    /// Merge desks, for one repo or all.
    Desk(Option<String>),
    /// The spaces page.
    Spaces,
    /// A space's page, or one board's card list in it.
    Board { space: String, board: Option<String> },
    Profile,
    Notifications,
}

impl Route {
    /// The sidebar section it lights.
    pub fn section(&self) -> &'static str {
        match self {
            Route::Home => "home",
            Route::Talk(_) => "talk",
            Route::Notes => "notes",
            Route::Desk(_) => "desk",
            Route::Spaces | Route::Board { .. } => "spaces",
            Route::Profile => "profile",
            Route::Notifications => "notifications",
        }
    }

    fn to_pref(&self) -> String {
        match self {
            Route::Talk(Some(c)) => format!("talk/{c}"),
            Route::Desk(Some(r)) => format!("desk/{r}"),
            Route::Board { space, board } => match board {
                Some(b) => format!("board/{space}/{b}"),
                None => format!("board/{space}"),
            },
            r => r.section().to_string(),
        }
    }

    fn from_pref(s: &str) -> Route {
        let mut p = s.splitn(3, '/');
        match (p.next(), p.next(), p.next()) {
            (Some("talk"), c, _) => Route::Talk(c.map(str::to_string)),
            (Some("notes"), ..) => Route::Notes,
            (Some("desk"), r, _) => Route::Desk(r.map(str::to_string)),
            (Some("spaces"), ..) => Route::Spaces,
            (Some("board"), Some(space), b) => Route::Board {
                space: space.to_string(),
                board: b.map(str::to_string),
            },
            (Some("profile"), ..) => Route::Profile,
            (Some("notifications"), ..) => Route::Notifications,
            _ => Route::Home,
        }
    }
}

/// A dialog over the window. Its content belongs to the package named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// card: New card, optionally on a board.
    NewCard { board: Option<String> },
    /// card: Share a card to a space.
    Share { card: String },
    /// inbox: held messages, optionally from one sender.
    Held { from: Option<String> },
    /// inbox: one message (a delivery id).
    Message { id: String },
    /// palette: the command palette.
    Palette,
    /// home: Wind down all agents.
    WindDown,
    /// settings: the settings dialog on a pane ("" for the first).
    Settings(String),
}

#[derive(Clone, Debug, Default)]
pub struct Nav {
    pub route: Route,
    /// The open card's id: its page shows over the route's.
    pub card: Option<String>,
    back: Vec<(Route, Option<String>)>,
    forward: Vec<(Route, Option<String>)>,
}

const PREF: &str = "nav.route";

impl Nav {
    pub fn restore(p: &Prefs) -> Nav {
        Nav {
            route: p.get::<String>(PREF).map(|s| Route::from_pref(&s)).unwrap_or_default(),
            ..Nav::default()
        }
    }

    pub fn save(&self, p: &mut Prefs) {
        p.set(PREF, self.route.to_pref());
    }

    pub fn go(&mut self, r: Route) {
        if self.route == r && self.card.is_none() {
            return;
        }
        self.back.push((self.route.clone(), self.card.take()));
        self.forward.clear();
        self.route = r;
    }

    pub fn open_card(&mut self, id: String) {
        if self.card.as_deref() == Some(id.as_str()) {
            return;
        }
        self.back.push((self.route.clone(), self.card.take()));
        self.forward.clear();
        self.card = Some(id);
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn back(&mut self) {
        if let Some((r, c)) = self.back.pop() {
            self.forward.push((std::mem::replace(&mut self.route, r), std::mem::replace(&mut self.card, c)));
        } else {
            self.card = None;
        }
    }

    pub fn forward(&mut self) {
        if let Some((r, c)) = self.forward.pop() {
            self.back.push((std::mem::replace(&mut self.route, r), std::mem::replace(&mut self.card, c)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_prefs_round_trip() {
        for r in [
            Route::Home,
            Route::Talk(None),
            Route::Talk(Some("c1".into())),
            Route::Notes,
            Route::Desk(Some("irori".into())),
            Route::Spaces,
            Route::Board { space: "s".into(), board: None },
            Route::Board { space: "s".into(), board: Some("b".into()) },
            Route::Profile,
            Route::Notifications,
        ] {
            assert_eq!(Route::from_pref(&r.to_pref()), r);
        }
    }

    #[test]
    fn back_returns_to_the_page_under_a_card() {
        let mut n = Nav::default();
        n.go(Route::Talk(None));
        n.open_card("c".into());
        assert_eq!(n.card.as_deref(), Some("c"));
        n.back();
        assert_eq!(n.card, None);
        assert_eq!(n.route, Route::Talk(None));
        n.back();
        assert_eq!(n.route, Route::Home);
        n.forward();
        assert_eq!(n.route, Route::Talk(None));
    }
}
