//! Fuzzy matching for the command palette (the React app's lib/fuzzy.ts,
//! card #213). Every query word has to match the item (its title, or else
//! one of its other texts); a word scores best as the start of the title,
//! then the start of a word in it, then anywhere, then as letters in order
//! ("nwcd" finds "New card"), and less again when only a keyword matches.

fn boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '/' | '#' | '.' | ':')
}

/// How well one lower-cased word matches one lower-cased text, 0 for no
/// match. `loose`: letters in order count too.
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
        return if text[..at].chars().last().is_some_and(boundary) { 80.0 } else { 60.0 };
    }
    let w: Vec<char> = word.chars().collect();
    if !loose || w.len() < 2 {
        return 0.0;
    }
    let t: Vec<char> = text.chars().collect();
    let (mut i, mut bonus, mut last) = (0usize, 0i64, -2i64);
    for (j, &c) in t.iter().enumerate() {
        if i >= w.len() {
            break;
        }
        if c != w[i] {
            continue;
        }
        if j == 0 || boundary(t[j - 1]) {
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
    (20 + bonus * 2).min(50) as f64
}

/// The query's words, lower-cased.
pub fn query_parts(query: &str) -> Vec<String> {
    query.to_lowercase().split_whitespace().map(str::to_string).collect()
}

/// How well every word of `query` matches `title`, with `others` as
/// whole-substring fallbacks at a lower weight; 0 when a word matches
/// nothing.
pub fn score(query: &str, title: &str, others: &[&str]) -> f64 {
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
pub fn rank<'a, T>(items: &'a [T], query: &str, texts: impl Fn(&T) -> (String, Vec<String>)) -> Vec<(&'a T, f64)> {
    let mut out: Vec<(usize, &T, f64)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let (title, others) = texts(item);
        let o: Vec<&str> = others.iter().map(String::as_str).collect();
        let s = score(query, &title, &o);
        if s > 0.0 {
            out.push((i, item, s));
        }
    }
    out.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    out.into_iter().map(|(_, t, s)| (t, s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_beats_word_start_beats_inside_beats_letters() {
        let prefix = word_score("new", "new card", true);
        let word_start = word_score("card", "new card", true);
        let inside = word_score("ard", "new card", true);
        let letters = word_score("nwcd", "new card", true);
        assert!(prefix > word_start && word_start > inside && inside > letters && letters > 0.0);
    }

    #[test]
    fn no_match_is_zero() {
        assert_eq!(word_score("xyz", "new card", true), 0.0);
        assert_eq!(word_score("dc", "new card", true), 0.0);
    }

    #[test]
    fn every_word_has_to_match() {
        assert!(score("new card", "New card", &[]) > 0.0);
        assert_eq!(score("new zebra", "New card", &[]), 0.0);
    }

    #[test]
    fn keywords_count_below_the_title() {
        assert!(score("preferences", "Open settings", &["preferences"]) > 0.0);
        assert!(score("settings", "Settings", &[]) > score("settings", "Devices", &["settings"]));
    }

    #[test]
    fn a_card_ref_is_found_by_its_ref_text() {
        assert!(score("#212", "Command palette", &["#212", "irori#212"]) > 0.0);
        assert!(score("wind#51", "Boss relay", &["#51", "wind#51"]) > 0.0);
    }

    #[test]
    fn rank_best_first_misses_dropped() {
        let items = ["Pin this board", "Open settings", "Settings: Devices", "Home"];
        let got: Vec<&str> = rank(&items, "settings", |t| (t.to_string(), vec![]))
            .into_iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(got, ["Settings: Devices", "Open settings"]);
    }
}
