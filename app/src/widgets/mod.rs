//! Card and agent pieces several pages share (F0c: the React app's
//! cards/bits.tsx, faces.tsx, agent-hover.tsx, agent-strip.tsx,
//! spawned-actions.tsx, done-rounds.tsx).

use rune::El;

use crate::ui::{Frame, State};

/// The sidebar's agents strip: who is running now. Not ported yet.
pub fn agent_strip(s: &State, f: &mut Frame) -> El {
    let _ = (s, f);
    rune::el().key("agent-strip").h(0.0)
}
