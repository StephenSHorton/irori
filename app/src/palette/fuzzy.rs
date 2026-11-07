//! Fuzzy matching for the command palette (card #213), the React app's
//! lib/fuzzy.ts. A query is split into words; every word has to match the
//! item (in its title, or else one of its other texts), and the item's
//! score is the sum. A word scores best as the start of the title, then the
//! start of a word in it, then anywhere in it, then as letters in order
//! ("nwcd" finds "New card"), and less again when it only matches a keyword.

fn word_break(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '/' | '#' | '.' | ':')
}

/// How well one lower-cased query word matches one lower-cased text, or 0
/// for no match. `loose`: letters in order count too.
pub fn word_score(word: &str, text: &str, loose: bool) -> f64 {
    if word.is_empty() {
        return 1.0;
    }
    if text.is_empty() {
        return 0.0;
    }
    if text == word {
        return 120.0;
    }
    if text.starts_with(word) {
        return 100.0;
    }
    if let Some(at) = text.find(word) {
        // The start of a word in the text ("card" in "new card").
        return if text[..at].chars().next_back().is_some_and(word_break) { 80.0 } else { 60.0 };
    }
    let w: Vec<char> = word.chars().collect();
    if !loose || w.len() < 2 {
        return 0.0;
    }
    // Letters in order, each preferring the start of a word.
    let t: Vec<char> = text.chars().collect();
    let (mut i, mut bonus, mut last) = (0usize, 0i32, -2i64);
    for j in 0..t.len() {
        if i >= w.len() {
            break;
        }
        if t[j] != w[i] {
            continue;
        }
        if j == 0 || word_break(t[j - 1]) {
            bonus += 2;
        }
        if j as i64 == last + 1 {
            bonus += 1;
        }
        last = j as i64;
        i += 1;
    }
    if i < w.len() {
        return 0.0;
    }
    f64::from((20 + bonus * 2).min(50))
}

/// The query's words, lower-cased.
pub fn query_parts(query: &str) -> Vec<String> {
    query.to_lowercase().split_whitespace().map(str::to_string).collect()
}

/// How well every word of `query` matches `title`, with `others` (ref,
/// keywords, names) as fallbacks at a lower weight and only as whole
/// substrings, or 0 when one word matches nothing.
pub fn score(query: &str, title: &str, others: &[String]) -> f64 {
    let words = query_parts(query);
    if words.is_empty() {
        return 1.0;
    }
    let t = title.to_lowercase();
    let o: Vec<String> = others.iter().filter(|x| !x.is_empty()).map(|x| x.to_lowercase()).collect();
    let mut total = 0.0;
    for w in &words {
        let mut best = word_score(w, &t, true);
        for x in &o {
            best = best.max(word_score(w, x, false) * 0.8);
        }
        if best == 0.0 {
            return 0.0;
        }
        total += best;
    }
    // A shorter title is a closer match for the same words.
    total - (t.chars().count().min(80) as f64) * 0.05
}

/// The items that match `query`, best first; ties keep their order.
pub fn rank<'a, T>(items: &'a [T], query: &str, texts: impl Fn(&T) -> (&str, &[String])) -> Vec<(&'a T, f64)> {
    let mut out: Vec<(usize, &T, f64)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let (title, others) = texts(item);
            let s = score(query, title, others);
            (s > 0.0).then_some((i, item, s))
        })
        .collect();
    out.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    out.into_iter().map(|(_, item, s)| (item, s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_beats_a_word_start_beats_inside_beats_letters_in_order() {
        let prefix = word_score("new", "new card", true);
        let word_start = word_score("card", "new card", true);
        let inside = word_score("ard", "new card", true);
        let letters = word_score("nwcd", "new card", true);
        assert!(prefix > word_start);
        assert!(word_start > inside);
        assert!(inside > letters);
        assert!(letters > 0.0);
    }

    #[test]
    fn no_match_is_0() {
        assert_eq!(word_score("xyz", "new card", true), 0.0);
        assert_eq!(word_score("dc", "new card", true), 0.0);
    }

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn every_word_has_to_match() {
        assert!(score("new card", "New card", &[]) > 0.0);
        assert_eq!(score("new zebra", "New card", &[]), 0.0);
    }

    #[test]
    fn keywords_count_below_the_title() {
        assert!(score("preferences", "Open settings", &v(&["preferences"])) > 0.0);
        assert!(score("settings", "Settings", &[]) > score("settings", "Devices", &v(&["settings"])));
    }

    #[test]
    fn a_card_ref_is_found_by_its_ref_text() {
        assert!(score("#212", "Command palette", &v(&["#212", "irori#212"])) > 0.0);
        assert!(score("wind#51", "Boss relay", &v(&["#51", "wind#51"])) > 0.0);
    }

    #[test]
    fn best_first_misses_dropped() {
        let items = ["Pin this board", "Open settings", "Settings: Devices", "Home"];
        let none: Vec<String> = Vec::new();
        let got: Vec<&str> = rank(&items, "settings", |t| (*t, none.as_slice())).into_iter().map(|(t, _)| *t).collect();
        assert_eq!(got, ["Settings: Devices", "Open settings"]);
    }
}
