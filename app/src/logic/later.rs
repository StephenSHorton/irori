//! Not yet and order (the React app's lib/later.ts, card #570): a held
//! card is out of the way until what brings it back comes; an order is a
//! run of cards each after the last.

use irori_window::later::{CardLater, CardOrder, OrderState, OrderStep};

/// A card's run, when it is in one of two or more.
pub fn chain_of(order: Option<&CardOrder>) -> Option<&[OrderStep]> {
    order.map(|o| o.steps.as_slice()).filter(|s| s.len() > 1)
}

/// The card a chain hangs under: its first not done.
pub fn head_of(ch: &[OrderStep]) -> Option<&OrderStep> {
    ch.iter().find(|s| s.state != OrderState::Done)
}

/// The cards that hang under card `id` in a list, in order.
pub fn follows_of<'a>(id: &str, order: Option<&'a CardOrder>) -> &'a [OrderStep] {
    let Some(ch) = chain_of(order) else {
        return &[];
    };
    if head_of(ch).map(|h| h.id.as_str()) != Some(id) {
        return &[];
    }
    match ch.iter().position(|s| s.id == id) {
        Some(i) => &ch[i + 1..],
        None => &[],
    }
}

/// Whether card `id` hangs under another card.
pub fn follows(id: &str, order: Option<&CardOrder>) -> bool {
    chain_of(order).is_some_and(|ch| head_of(ch).map(|h| h.id.as_str()) != Some(id))
}

/// A held card's line under its title: "Not yet: on 1 Dec. Not before
/// the release."
pub fn when_later(l: &CardLater) -> String {
    let mut ch = l.line.chars();
    let line = match ch.next() {
        Some(f) => f.to_lowercase().collect::<String>() + ch.as_str(),
        None => String::new(),
    };
    let why = l.why.trim().trim_end_matches(['.', '!', '?']);
    if why.is_empty() {
        format!("Not yet: {line}.")
    } else {
        format!("Not yet: {line}. {why}.")
    }
}

/// The card before `id` in its order and the one after it.
pub fn around<'a>(
    id: &str,
    order: Option<&'a CardOrder>,
) -> (Option<&'a OrderStep>, Option<&'a OrderStep>) {
    let Some(ch) = chain_of(order) else {
        return (None, None);
    };
    let Some(i) = ch.iter().position(|s| s.id == id) else {
        return (None, None);
    };
    (i.checked_sub(1).map(|j| &ch[j]), ch.get(i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::later::LaterBack;

    fn step(short: &str, state: OrderState) -> OrderStep {
        OrderStep {
            id: format!("c{short}"),
            short: short.into(),
            title: format!("Card {short}"),
            state,
            ..OrderStep::default()
        }
    }
    fn run() -> CardOrder {
        CardOrder {
            steps: vec![
                step("80", OrderState::Done),
                step("81", OrderState::Working),
                step("82", OrderState::Waiting),
                step("83", OrderState::Later),
            ],
            ..CardOrder::default()
        }
    }
    fn later(why: &str, line: &str) -> CardLater {
        CardLater {
            why: why.into(),
            back: LaterBack::Word,
            line: line.into(),
            ..CardLater::default()
        }
    }

    #[test]
    fn when_later_is_one_line() {
        assert_eq!(
            when_later(&later("Not before the release", "on 1 Dec")),
            "Not yet: on 1 Dec. Not before the release."
        );
        assert_eq!(
            when_later(&later("An idea.", "When you say")),
            "Not yet: when you say. An idea."
        );
        assert_eq!(
            when_later(&later("", "when you say")),
            "Not yet: when you say."
        );
    }

    #[test]
    fn chains() {
        let r = run();
        assert_eq!(chain_of(Some(&r)).map(|c| c.len()), Some(4));
        assert!(chain_of(None).is_none());
        let lone = CardOrder {
            steps: vec![step("1", OrderState::Waiting)],
            ..CardOrder::default()
        };
        assert!(chain_of(Some(&lone)).is_none());
        assert_eq!(head_of(&r.steps).map(|s| s.id.as_str()), Some("c81"));
        let ids = |s: &[OrderStep]| s.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(follows_of("c81", Some(&r))), ["c82", "c83"]);
        assert!(follows_of("c82", Some(&r)).is_empty());
        assert!(follows_of("lone", None).is_empty());
        assert!(!follows("c81", Some(&r)));
        assert!(follows("c82", Some(&r)));
        assert!(follows("c83", Some(&r)));
        assert!(!follows("lone", None));
    }

    #[test]
    fn around_is_before_and_after() {
        let r = run();
        let id = |s: Option<&OrderStep>| s.map(|x| x.id.clone());
        let (b, a) = around("c80", Some(&r));
        assert_eq!((id(b), id(a)), (None, Some("c81".into())));
        let (b, a) = around("c82", Some(&r));
        assert_eq!((id(b), id(a)), (Some("c81".into()), Some("c83".into())));
        let (b, a) = around("c83", Some(&r));
        assert_eq!((id(b), id(a)), (Some("c82".into()), None));
        assert_eq!(around("lone", None), (None, None));
    }
}
