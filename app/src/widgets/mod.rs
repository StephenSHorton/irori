//! Card and agent pieces several pages share (F0c). Work in progress.

use rune::El;

use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {}

pub fn agent_strip(s: &State, f: &mut Frame) -> El {
    let _ = (s, f);
    rune::el().key("agent-strip").h(0.0)
}
pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}
pub fn overlay(s: &State, f: &mut Frame) {
    let _ = (s, f);
}
pub fn poll(s: &mut State, tx: &mut rune::Tx) -> bool {
    let _ = (s, tx);
    false
}
pub fn scene(s: &mut State) {
    let _ = s;
}
