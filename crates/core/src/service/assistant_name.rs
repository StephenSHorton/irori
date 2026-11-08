//! The person's Office Assistant's name (card #1045,
//! docs/design/call-agents.md §5): set in the app, kept on every device
//! through a private `assistant.name` row, and carried to other members on
//! the person's member.profile.

use super::*;

impl Core {
    /// The name the person chose for their assistant, "" when they haven't
    /// (the default then applies).
    pub fn assistant_name(&self) -> String {
        self.s.assistant_name.chosen()
    }

    /// The name shown for the person's own assistant: the chosen one, else
    /// "<name>'s assistant" from their current name. Nothing is written for
    /// the default, so it follows a rename.
    pub fn assistant_name_or_default(&self) -> String {
        let chosen = self.s.assistant_name.chosen();
        if chosen.is_empty() {
            coord::assistant_name::default_name(&self.s.ident.current().name)
        } else {
            chosen
        }
    }

    /// Records the name (`""` goes back to the default): 1 to 24 letters and
    /// spaces. Every shared space hears it on the person's member.profile.
    /// Only the window calls it, so no agent's tool can.
    pub fn set_assistant_name(&self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if !coord::assistant_name::valid(name) {
            return Err(format!(
                "a name is 1 to {} letters and spaces",
                coord::assistant_name::MAX_CHARS
            ));
        }
        self.s
            .store
            .append(
                "user",
                coord::assistant_name::KIND,
                &AssistantNameRow {
                    name,
                    by: &self.s.person_id(),
                    at: (self.s.now)().unix_milli_value(),
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
