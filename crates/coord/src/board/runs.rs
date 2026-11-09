//! How attempts on cards end (card #966): every request for a card's agent
//! records the weight, model and effort it ran on and whether the advisor
//! was on. The person's answer on the done check that attempt raised says
//! how it ended. Worked out when asked, from what the fold already holds,
//! so it gives one answer whatever order the rows came in.

use std::collections::BTreeMap;

use serde::Serialize;

use super::cards::last_answer;
use super::fold::Fold;
use super::types::WEIGHTS;

/// What one attempt ran on.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Run {
    pub weight: String,
    pub model: String,
    pub effort: String,
    pub advisor: bool,
}

/// One row of the history: attempts the person has answered, on one weight,
/// model and effort.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTally {
    pub weight: String,
    pub model: String,
    pub effort: String,
    /// Answered attempts.
    pub runs: u32,
    /// The person said Done, and the card never crashed, moved or came back.
    pub first_try: u32,
    /// The person said Not done.
    pub sent_back: u32,
    /// How many ran with the advisor on.
    pub advised: u32,
}

impl Fold {
    pub(crate) fn run_tally(&self) -> Vec<RunTally> {
        let mut rows: BTreeMap<(usize, String, String), RunTally> = BTreeMap::new();
        for t in self.tasks.values() {
            let Some(runs) = self.runs.get(&t.id) else {
                continue;
            };
            for b in t.blockers.iter().filter(|b| b.done_check) {
                let Some(run) = self
                    .spawned_for(&b.raised_by)
                    .filter(|m| m.task_id == t.id)
                    .and_then(|m| runs.get(&m.generation))
                else {
                    continue;
                };
                let done = b.resolved.as_ref().is_some_and(|r| r.option >= 0);
                let back = !done && last_answer(self, t, &b.id).is_some_and(|c| c.pick <= 0);
                if !done && !back {
                    continue;
                }
                let rank = WEIGHTS.iter().position(|w| *w == run.weight).unwrap_or(1);
                let row = rows
                    .entry((rank, run.model.clone(), run.effort.clone()))
                    .or_insert_with(|| RunTally {
                        weight: run.weight.clone(),
                        model: run.model.clone(),
                        effort: run.effort.clone(),
                        ..RunTally::default()
                    });
                row.runs += 1;
                row.first_try += u32::from(done && t.retries.is_empty());
                row.sent_back += u32::from(back);
                row.advised += u32::from(run.advisor);
            }
        }
        rows.into_values().collect()
    }
}
