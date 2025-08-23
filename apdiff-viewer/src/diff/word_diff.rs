use similar::{ChangeTag, TextDiff};
use std::cmp;
use strsim::levenshtein;

pub struct WordDiffResult {
    pub old_highlighted: String,
    pub new_highlighted: String,
}

/// Split text into words while preserving whitespace boundaries
/// Similar to jsDiff.diffWordsWithSpace
fn split_into_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let mut in_word = false;

    for (i, ch) in text.char_indices() {
        let is_whitespace = ch.is_whitespace();

        if in_word && is_whitespace {
            // End of word
            words.push(&text[start..i]);
            start = i;
            in_word = false;
        } else if !in_word && !is_whitespace {
            // Start of word, but first push any accumulated whitespace
            if i > start {
                words.push(&text[start..i]);
            }
            start = i;
            in_word = true;
        }
    }

    // Push the final segment
    if start < text.len() {
        words.push(&text[start..]);
    }

    words
}

/// Calculate normalized levenshtein distance between two strings
/// Returns a score between 0.0 and 1.0 where 0.0 is identical
fn normalized_distance(a: &str, b: &str) -> f64 {
    let a_trimmed = a.trim();
    let b_trimmed = b.trim();
    let distance = levenshtein(a_trimmed, b_trimmed);
    let max_len = cmp::max(a_trimmed.len(), b_trimmed.len()) as f64;

    if max_len == 0.0 {
        0.0
    } else {
        distance as f64 / max_len
    }
}

/// Find best matches between added and removed words using distance metric
/// Similar to the matcher function in diff2html
fn find_enhanced_matches(
    added_words: &[&str],
    removed_words: &[&str],
    threshold: f64,
) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut used_added = vec![false; added_words.len()];
    let mut used_removed = vec![false; removed_words.len()];

    // Find all potential matches below threshold
    let mut candidates = Vec::new();
    for (i, &added) in added_words.iter().enumerate() {
        for (j, &removed) in removed_words.iter().enumerate() {
            let dist = normalized_distance(added, removed);
            if dist < threshold {
                candidates.push((i, j, dist));
            }
        }
    }

    // Sort by distance and greedily select best matches
    candidates.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

    for (i, j, _dist) in candidates {
        if !used_added[i] && !used_removed[j] {
            matches.push((i, j));
            used_added[i] = true;
            used_removed[j] = true;
        }
    }

    matches
}

/// Skip word-level diffing for very long lines to avoid performance issues
fn skip_word_diff(old_line: &str, new_line: &str) -> WordDiffResult {
    WordDiffResult {
        old_highlighted: html_escape::encode_text(old_line).to_string(),
        new_highlighted: html_escape::encode_text(new_line).to_string(),
    }
}

/// Process inserted words with enhanced highlighting
fn process_inserted_words(
    content: &str,
    enhanced_added: &[bool],
    added_word_idx: &mut usize,
) -> String {
    let words = split_into_words(content);
    let mut word_html = String::with_capacity(content.len() + words.len() * 20); // Estimate for HTML tags

    for word in &words {
        let escaped_word = html_escape::encode_text(word);
        if word.trim().is_empty() {
            word_html.push_str(&escaped_word);
        } else {
            let is_enhanced =
                *added_word_idx < enhanced_added.len() && enhanced_added[*added_word_idx];

            if is_enhanced {
                word_html.push_str(&format!("<ins class=\"enhanced\">{escaped_word}</ins>"));
            } else {
                word_html.push_str(&format!("<ins>{escaped_word}</ins>"));
            }
            *added_word_idx += 1;
        }
    }
    word_html
}

/// Process deleted words with enhanced highlighting
fn process_deleted_words(
    content: &str,
    enhanced_removed: &[bool],
    removed_word_idx: &mut usize,
) -> String {
    let words = split_into_words(content);
    let mut word_html = String::with_capacity(content.len() + words.len() * 20); // Estimate for HTML tags

    for word in &words {
        let escaped_word = html_escape::encode_text(word);
        if word.trim().is_empty() {
            word_html.push_str(&escaped_word);
        } else {
            let is_enhanced =
                *removed_word_idx < enhanced_removed.len() && enhanced_removed[*removed_word_idx];

            if is_enhanced {
                word_html.push_str(&format!("<del class=\"enhanced\">{escaped_word}</del>"));
            } else {
                word_html.push_str(&format!("<del>{escaped_word}</del>"));
            }
            *removed_word_idx += 1;
        }
    }
    word_html
}

/// Perform word-level diff highlighting on two lines of text
pub fn highlight_word_diff(
    old_line: &str,
    new_line: &str,
    max_line_length: usize,
    match_threshold: f64,
) -> WordDiffResult {
    if old_line.len() > max_line_length || new_line.len() > max_line_length {
        return skip_word_diff(old_line, new_line);
    }

    let old_words = split_into_words(old_line);
    let new_words = split_into_words(new_line);

    // Use similar crate to find word-level diffs
    let old_text = old_words.join("");
    let new_text = new_words.join("");
    let diff = TextDiff::from_words(&old_text, &new_text);

    // Extract added and removed words for enhanced matching
    let mut added_words = Vec::new();
    let mut removed_words = Vec::new();
    let mut added_indices = Vec::new();
    let mut removed_indices = Vec::new();

    let mut added_idx = 0;
    let mut removed_idx = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => {
                let words = split_into_words(change.value());
                for word in &words {
                    if !word.trim().is_empty() {
                        added_words.push(*word);
                        added_indices.push(added_idx);
                    }
                }
                added_idx += words.len();
            }
            ChangeTag::Delete => {
                let words = split_into_words(change.value());
                for word in &words {
                    if !word.trim().is_empty() {
                        removed_words.push(*word);
                        removed_indices.push(removed_idx);
                    }
                }
                removed_idx += words.len();
            }
            ChangeTag::Equal => {
                let words = split_into_words(change.value());
                added_idx += words.len();
                removed_idx += words.len();
            }
        }
    }

    // Find enhanced matches between similar words
    let enhanced_matches = find_enhanced_matches(&added_words, &removed_words, match_threshold);
    let mut enhanced_added = vec![false; added_words.len()];
    let mut enhanced_removed = vec![false; removed_words.len()];

    for &(added_i, removed_i) in &enhanced_matches {
        enhanced_added[added_i] = true;
        enhanced_removed[removed_i] = true;
    }

    // Build highlighted strings
    let mut old_html = String::new();
    let mut new_html = String::new();

    let mut added_word_idx = 0;
    let mut removed_word_idx = 0;

    for change in diff.iter_all_changes() {
        let escaped_value = html_escape::encode_text(change.value());

        match change.tag() {
            ChangeTag::Insert => {
                let word_html =
                    process_inserted_words(change.value(), &enhanced_added, &mut added_word_idx);
                new_html.push_str(&word_html);
            }
            ChangeTag::Delete => {
                let word_html =
                    process_deleted_words(change.value(), &enhanced_removed, &mut removed_word_idx);
                old_html.push_str(&word_html);
            }
            ChangeTag::Equal => {
                old_html.push_str(&escaped_value);
                new_html.push_str(&escaped_value);
            }
        }
    }

    WordDiffResult {
        old_highlighted: old_html,
        new_highlighted: new_html,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_into_words() {
        let text = "hello world  test";
        let words = split_into_words(text);
        assert_eq!(words, vec!["hello", " ", "world", "  ", "test"]);
    }

    #[test]
    fn test_word_diff_simple() {
        let old_line = "hello world";
        let new_line = "hello rust";
        let result = highlight_word_diff(old_line, new_line, 10000, 0.25);

        assert!(result.old_highlighted.contains("<del>world</del>"));
        assert!(result.new_highlighted.contains("<ins>rust</ins>"));
        assert!(result.old_highlighted.contains("hello"));
        assert!(result.new_highlighted.contains("hello"));
    }

    #[test]
    fn test_normalized_distance() {
        assert_eq!(normalized_distance("hello", "hello"), 0.0);
        assert!(normalized_distance("hello", "hallo") < 0.5);
        assert!(normalized_distance("hello", "world") > 0.5);
    }
}
