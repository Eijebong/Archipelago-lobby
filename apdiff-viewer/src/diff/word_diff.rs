use similar::{ChangeTag, TextDiff};

pub struct WordDiffResult {
    pub old_highlighted: String,
    pub new_highlighted: String,
}

fn split_into_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let mut in_word = false;

    for (i, ch) in text.char_indices() {
        let is_whitespace = ch.is_whitespace();

        if in_word && is_whitespace {
            words.push(&text[start..i]);
            start = i;
            in_word = false;
        } else if !in_word && !is_whitespace {
            if i > start {
                words.push(&text[start..i]);
            }
            start = i;
            in_word = true;
        }
    }

    if start < text.len() {
        words.push(&text[start..]);
    }

    words
}

fn skip_word_diff(old_line: &str, new_line: &str) -> WordDiffResult {
    WordDiffResult {
        old_highlighted: html_escape::encode_text(old_line).to_string(),
        new_highlighted: html_escape::encode_text(new_line).to_string(),
    }
}

fn process_inserted_words(content: &str) -> String {
    let escaped_content = html_escape::encode_text(content);
    format!("<ins>{escaped_content}</ins>")
}

fn process_deleted_words(content: &str) -> String {
    let escaped_content = html_escape::encode_text(content);
    format!("<del>{escaped_content}</del>")
}

fn combine_adjacent_tags(html: &str) -> String {
    let patterns = [
        ("</ins><ins>", ""),
        ("</ins> <ins>", " "),
        ("</ins><ins class=\"enhanced\">", ""),
        ("</ins> <ins class=\"enhanced\">", " "),
        ("</del><del>", ""),
        ("</del> <del>", " "),
        ("</del><del class=\"enhanced\">", ""),
        ("</del> <del class=\"enhanced\">", " "),
    ];

    let mut result = html.to_string();
    let mut changed = true;

    while changed {
        let before_len = result.len();

        for (pattern, replacement) in &patterns {
            result = result.replace(pattern, replacement);
        }

        changed = result.len() != before_len;
    }

    result
}

pub fn highlight_word_diff(
    old_line: &str,
    new_line: &str,
    max_line_length: usize,
    _match_threshold: f64,
) -> WordDiffResult {
    if old_line.len() > max_line_length || new_line.len() > max_line_length {
        return skip_word_diff(old_line, new_line);
    }

    let old_words = split_into_words(old_line);
    let new_words = split_into_words(new_line);

    let old_text = old_words.join("");
    let new_text = new_words.join("");
    let diff = TextDiff::from_words(&old_text, &new_text);

    let grouped_changes: Vec<(String, ChangeTag)> = diff
        .iter_all_changes()
        .map(|change| (change.value().to_string(), change.tag()))
        .collect();

    let (old_html, new_html) = grouped_changes.into_iter().fold(
        (String::new(), String::new()),
        |(mut old_html, mut new_html), (content, tag)| {
            match tag {
                ChangeTag::Insert => {
                    let word_html = process_inserted_words(&content);
                    new_html.push_str(&word_html);
                }
                ChangeTag::Delete => {
                    let word_html = process_deleted_words(&content);
                    old_html.push_str(&word_html);
                }
                ChangeTag::Equal => {
                    let escaped_value = html_escape::encode_text(&content);
                    old_html.push_str(&escaped_value);
                    new_html.push_str(&escaped_value);
                }
            }
            (old_html, new_html)
        },
    );

    let old_combined = combine_adjacent_tags(&old_html);
    let new_combined = combine_adjacent_tags(&new_html);

    WordDiffResult {
        old_highlighted: old_combined,
        new_highlighted: new_combined,
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

    #[test]
    fn test_word_diff_combines_adjacent_words() {
        let old_line = "hello beautiful world";
        let new_line = "hello amazing fantastic world";
        let result = highlight_word_diff(old_line, new_line, 10000, 0.25);

        assert!(result
            .new_highlighted
            .contains("<ins>amazing fantastic</ins>"));
        assert!(result.old_highlighted.contains("<del>beautiful</del>"));
        assert!(result.old_highlighted.contains("hello "));
        assert!(result.old_highlighted.contains(" world"));
        assert!(result.new_highlighted.contains("hello "));
        assert!(result.new_highlighted.contains(" world"));
    }

    #[test]
    fn test_word_diff_combines_consecutive_changes() {
        let old_line = "missions that needs this module are now unlocked. (If";
        let new_line = "manual for this module is now unlocked in the Expert";
        let result = highlight_word_diff(old_line, new_line, 10000, 0.25);

        assert!(!result.new_highlighted.contains("</ins> <ins>"));
        assert!(!result.old_highlighted.contains("</del> <del>"));

        let ins_count = result.new_highlighted.matches("<ins>").count();
        let del_count = result.old_highlighted.matches("<del>").count();

        assert!(ins_count <= 4, "Too many <ins> tags: {}", ins_count);
        assert!(del_count <= 4, "Too many <del> tags: {}", del_count);
    }
}
