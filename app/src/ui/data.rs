//! What many pages read, kept fresh from the backend's events: the cards,
//! the agents, the spaces, this person's devices and the counts the
//! sidebar shows. A package reads `s.data`; it re-reads its own page's
//! data in its `on_event`.

use irori_window::Window;
use irori_window::agents::AgentsPauseView;
use irori_window::boards::BoardSummary;
use irori_window::cards::CardSummary;
use irori_window::devices::DeviceView;
use irori_window::events::{
    EVENT_BOARD_CHANGED, EVENT_DEVICES_CHANGED, EVENT_INBOX_CHANGED, EVENT_ROSTER_CHANGED,
};
use irori_window::inbox::InboxCounts;
use irori_window::roster::RosterAgent;

use super::Event;

#[derive(Clone, Debug, Default)]
pub struct Data {
    /// Every card this person sees, newest change first.
    pub cards: Vec<CardSummary>,
    pub agents: Vec<RosterAgent>,
    pub spaces: Vec<BoardSummary>,
    pub devices: Vec<DeviceView>,
    pub inbox: InboxCounts,
    /// Cards waiting on the person (the sidebar's Home count).
    pub decide: usize,
    pub unread_notifications: u32,
    pub pause: AgentsPauseView,
    pub your_name: String,
    pub version: String,
    /// How many times the cards were re-read (a cheap change stamp for
    /// packages that cache derived lists).
    pub cards_rev: u64,
}

impl Data {
    pub fn refresh_all(&mut self, w: Option<&Window>) {
        let Some(w) = w else { return };
        self.refresh_cards(w);
        self.agents = w.agents();
        self.spaces = w.boards_list();
        self.devices = w.devices();
        self.inbox = w.inbox_counts();
        self.unread_notifications = w.unread_notifications();
        self.pause = w.agents_pause();
        self.your_name = w.get_your_name();
        self.version = w.version();
    }

    fn refresh_cards(&mut self, w: &Window) {
        let mut cards = w.cards();
        cards.sort_by_key(|c| std::cmp::Reverse(c.updated_at.unix_milli_value()));
        self.cards = cards;
        self.decide = w.decide_count();
        self.cards_rev += 1;
    }

    pub fn on_event(&mut self, w: Option<&Window>, ev: &Event) {
        let Some(w) = w else { return };
        match ev.name.as_str() {
            EVENT_BOARD_CHANGED => {
                self.refresh_cards(w);
                self.unread_notifications = w.unread_notifications();
            }
            EVENT_ROSTER_CHANGED => {
                self.agents = w.agents();
                self.refresh_cards(w);
            }
            EVENT_INBOX_CHANGED => {
                self.inbox = w.inbox_counts();
            }
            EVENT_DEVICES_CHANGED => {
                self.devices = w.devices();
                self.your_name = w.get_your_name();
            }
            irori_window::boards::EVENT_BOARDS_CHANGED => self.spaces = w.boards_list(),
            irori_window::notifications::EVENT_NOTIFICATIONS_CHANGED => {
                self.unread_notifications = w.unread_notifications();
            }
            irori_window::agents::EVENT_AGENTS_PAUSED => {
                self.pause = ev.parse().unwrap_or_else(|| w.agents_pause());
            }
            _ => {}
        }
    }

    pub fn card(&self, id: &str) -> Option<&CardSummary> {
        let id = id.trim_start_matches("task:");
        self.cards.iter().find(|c| c.id == id)
    }
}
