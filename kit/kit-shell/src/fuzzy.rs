//! The fuzzy search of the command palette, the one that quick insertion uses too (notation 2.6): the letters of the
//! query, in order, in the text, case and accents of ASCII aside. Deterministic and offline: the same query on the same
//! list gives the same order, the best match first, then the alphabet.

/// How well `query` matches `text`: none when its letters are not all in `text`, in order. Higher is better: a match at
/// the start of a word, and letters that follow one another, count more; a shorter text wins a tie.
pub fn score(query: &str, text: &str) -> Option<i64> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().collect();
    let mut score: i64 = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for wanted in &query {
        let found = (at..text.len()).find(|&index| text.get(index).is_some_and(|c| c.to_lowercase().eq(std::iter::once(*wanted))))?;
        let starts_word = found == 0 || text.get(found - 1).is_some_and(|before| !before.is_alphanumeric()) || is_hump(&text, found);
        score += if starts_word { 8 } else { 1 };
        if previous.is_some_and(|previous| previous + 1 == found) {
            score += 4;
        }
        // A gap costs a little: `nd` is a better match in `node` than in `new_diagram`.
        score -= i64::try_from(found - at).unwrap_or(i64::MAX / 4).min(16);
        previous = Some(found);
        at = found + 1;
    }
    Some(score * 64 - i64::try_from(text.len()).unwrap_or(i64::MAX / 128).min(63))
}

/// A capital after a small letter: `setView`.
fn is_hump(text: &[char], at: usize) -> bool {
    at > 0 && text.get(at).is_some_and(|c| c.is_uppercase()) && text.get(at - 1).is_some_and(|c| c.is_lowercase())
}

/// The indices of the items that match `query`, best first: each item is searched by its texts (a label, an id…), the
/// best of them counting, then in the order of its first text, then in the order given.
pub fn rank<T>(query: &str, items: &[T], texts: impl Fn(&T) -> Vec<&str>) -> Vec<usize> {
    let mut found: Vec<(i64, String, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let texts = texts(item);
            let best = texts.iter().filter_map(|text| score(query, text)).max()?;
            Some((best, texts.first().map(|text| text.to_lowercase()).unwrap_or_default(), index))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));
    found.into_iter().map(|(_, _, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_letters_must_come_in_order() {
        assert!(score("nd", "node").is_some());
        assert!(score("dn", "node").is_none());
        assert!(score("", "anything").is_some());
        assert!(score("NODE", "node.add").is_some());
    }

    #[test]
    fn a_match_at_the_start_of_words_wins() {
        let words = score("na", "node.add");
        let inside = score("na", "banana");
        assert!(words > inside, "{words:?} {inside:?}");
    }

    #[test]
    fn letters_that_follow_one_another_win() {
        assert!(score("open", "Open folder") > score("open", "Other pen"));
    }

    #[test]
    fn the_shorter_text_then_the_alphabet_and_the_order_is_stable() {
        let items = ["Zoom", "Reset layout", "Redo", "Run", "Rim"];
        let ranked = rank("r", &items, |item| vec![*item]);
        assert_eq!(ranked.iter().map(|&index| items[index]).collect::<Vec<_>>(), ["Rim", "Run", "Redo", "Reset layout"]);
        assert_eq!(rank("r", &items, |item| vec![*item]), ranked);
    }

    #[test]
    fn every_text_of_an_item_is_searched() {
        let items = [("Undo", "history.undo"), ("Open folder", "project.open")];
        let ranked = rank("history", &items, |(label, id)| vec![*label, *id]);
        assert_eq!(ranked, [0]);
    }
}
