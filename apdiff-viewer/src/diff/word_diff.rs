use similar::{ChangeTag, TextDiff};

use super::{WordChangeSegment, WordChangeType};

pub struct StructuredWordDiffResult {
    pub old_changes: Vec<WordChangeSegment>,
    pub new_changes: Vec<WordChangeSegment>,
}

fn split_into_words(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    // Find boundaries where character type changes (word ↔ whitespace)
    let boundaries = std::iter::once(0)
        .chain(
            text.char_indices()
                .zip(text.char_indices().skip(1))
                .filter_map(|((_, c1), (pos2, c2))| {
                    if c1.is_whitespace() != c2.is_whitespace() {
                        Some(pos2)
                    } else {
                        None
                    }
                }),
        )
        .chain(std::iter::once(text.len()));

    // Split text at boundaries
    boundaries
        .collect::<Vec<_>>()
        .windows(2)
        .map(|window| &text[window[0]..window[1]])
        .collect()
}

pub fn highlight_word_diff_structured(
    old_line: &str,
    new_line: &str,
    max_line_length: usize,
) -> StructuredWordDiffResult {
    if old_line.len() > max_line_length || new_line.len() > max_line_length {
        return StructuredWordDiffResult {
            old_changes: vec![create_equal_segment(old_line, 0)],
            new_changes: vec![create_equal_segment(new_line, 0)],
        };
    }

    // Split into words but preserve word boundaries with spaces
    let old_words = split_into_words(old_line);
    let new_words = split_into_words(new_line);

    create_word_diff_segments(&old_words, &new_words)
}

/// Helper function to create a word change segment
fn create_segment(
    content: &str,
    change_type: WordChangeType,
    start_offset: usize,
) -> WordChangeSegment {
    WordChangeSegment {
        text: content.to_string(),
        change_type,
        start_offset,
        end_offset: start_offset + content.len(),
    }
}

/// Helper function to create an equal segment
fn create_equal_segment(content: &str, start_offset: usize) -> WordChangeSegment {
    create_segment(content, WordChangeType::Equal, start_offset)
}

/// Create word diff segments with proper space handling
///
/// This function performs word-level diffing while preserving spaces within change blocks.
/// Performs word-level diff with slice comparison,
/// then consolidate consecutive changes of the same type to group words with their spaces.
fn create_word_diff_segments(old_words: &[&str], new_words: &[&str]) -> StructuredWordDiffResult {
    // Use word-level diffing with string references
    let diff = TextDiff::from_slices(old_words, new_words);

    let (old_changes, new_changes) = diff
        .iter_all_changes()
        .scan((0, 0), |(old_offset, new_offset), change| {
            let content = change.value();
            let change_type = match change.tag() {
                ChangeTag::Equal => WordChangeType::Equal,
                ChangeTag::Delete => WordChangeType::Delete,
                ChangeTag::Insert => WordChangeType::Insert,
            };

            let old_segment = match change.tag() {
                ChangeTag::Equal | ChangeTag::Delete => {
                    let segment = Some(create_segment(content, change_type, *old_offset));
                    *old_offset += content.len();
                    segment
                }
                ChangeTag::Insert => None,
            };

            let new_segment = match change.tag() {
                ChangeTag::Equal | ChangeTag::Insert => {
                    let segment = Some(create_segment(content, change_type, *new_offset));
                    *new_offset += content.len();
                    segment
                }
                ChangeTag::Delete => None,
            };

            Some((old_segment, new_segment))
        })
        .fold(
            (Vec::new(), Vec::new()),
            |(mut old_acc, mut new_acc), (old_opt, new_opt)| {
                if let Some(segment) = old_opt {
                    old_acc.push(segment);
                }
                if let Some(segment) = new_opt {
                    new_acc.push(segment);
                }
                (old_acc, new_acc)
            },
        );

    // The key improvement: consolidate consecutive changes of the same type
    // This will merge fragments like ["word1", " ", "word2"] into ["word1 word2"]
    // when they all have the same change type (e.g., all Delete or all Insert)
    let consolidated_old_changes = consolidate_consecutive_changes(&old_changes);
    let consolidated_new_changes = consolidate_consecutive_changes(&new_changes);

    StructuredWordDiffResult {
        old_changes: consolidated_old_changes,
        new_changes: consolidated_new_changes,
    }
}

/// Consolidate consecutive changes of the same type to prevent space fragmentation
///
/// This function merges segments with the same change type, even when separated
/// by whitespace-only Equal segments. This eliminates the fragmentation issue where
/// spaces between changed words were creating separate segments like:
/// Delete("word1") -> Equal(" ") -> Delete("word2") becomes Delete("word1 word2")
fn consolidate_consecutive_changes(changes: &[WordChangeSegment]) -> Vec<WordChangeSegment> {
    changes
        .iter()
        .enumerate()
        .scan(None::<usize>, |skip_until, (i, segment)| {
            // Skip segments that were already consolidated
            if skip_until.is_some_and(|skip| i < skip) {
                return Some(None);
            }

            if matches!(
                segment.change_type,
                WordChangeType::Delete | WordChangeType::Insert
            ) {
                // Find the range of segments to consolidate
                let consolidation_end = find_consolidation_end(changes, i);
                *skip_until = Some(consolidation_end);

                // Build consolidated segment from the range
                let segments_to_merge = &changes[i..consolidation_end];
                let consolidated_text = segments_to_merge
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<String>();

                Some(Some(WordChangeSegment {
                    text: consolidated_text,
                    change_type: segment.change_type,
                    start_offset: segment.start_offset,
                    end_offset: segment.start_offset
                        + segments_to_merge
                            .iter()
                            .map(|s| s.text.len())
                            .sum::<usize>(),
                }))
            } else {
                // Equal segment, keep as-is
                Some(Some(segment.clone()))
            }
        })
        .flatten()
        .collect()
}

/// Find the end index for consolidation of segments starting at the given index
fn find_consolidation_end(changes: &[WordChangeSegment], start_idx: usize) -> usize {
    let target_type = changes[start_idx].change_type;

    changes[start_idx..]
        .iter()
        .enumerate()
        .scan(true, |can_continue, (offset, segment)| {
            if segment.change_type == target_type {
                Some(Some(start_idx + offset + 1)) // Include this segment
            } else if segment.change_type == WordChangeType::Equal
                && is_whitespace_only(&segment.text)
                && *can_continue
            {
                // Check if next segment (if exists) is the target type
                let next_idx = start_idx + offset + 1;
                if next_idx < changes.len() && changes[next_idx].change_type == target_type {
                    Some(Some(start_idx + offset + 1)) // Include bridging whitespace
                } else {
                    *can_continue = false;
                    Some(None) // Stop here, don't include trailing whitespace
                }
            } else {
                *can_continue = false;
                Some(None) // Stop consolidation
            }
        })
        .flatten()
        .last()
        .unwrap_or(start_idx + 1)
}

/// Check if a string contains only whitespace characters
fn is_whitespace_only(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper function to reconstruct text from segments (used in multiple tests)
    fn reconstruct_text(segments: &[WordChangeSegment]) -> String {
        segments.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn test_split_into_words() {
        let text = "hello world  test";
        let words = split_into_words(text);
        assert_eq!(words, vec!["hello", " ", "world", "  ", "test"]);
    }

    #[test]
    fn test_word_diff_structured_simple() {
        let old_line = "hello world";
        let new_line = "hello rust";
        let result = highlight_word_diff_structured(old_line, new_line, 10000);

        // Check that we have the expected changes
        let old_has_delete = result.old_changes.iter().any(|c| {
            c.change_type == super::super::WordChangeType::Delete && c.text.contains("world")
        });
        let new_has_insert = result.new_changes.iter().any(|c| {
            c.change_type == super::super::WordChangeType::Insert && c.text.contains("rust")
        });

        assert!(old_has_delete);
        assert!(new_has_insert);
    }

    #[test]
    fn test_space_fragmentation_issue() {
        // This test demonstrates the space fragmentation problem
        let old_line = "No checks to send";
        let new_line = "Different message here";

        let result = highlight_word_diff_structured(old_line, new_line, 10000);

        // The key test: we should have consolidated delete segments, not fragmented ones
        // Instead of multiple small segments like ["No", " ", "checks", " ", "to", " ", "send"]
        // we should have fewer, consolidated segments

        // Verify that all text is preserved when we reconstruct from all segments
        let reconstructed_old = reconstruct_text(&result.old_changes);
        assert_eq!(
            reconstructed_old, old_line,
            "All old text should be preserved when reconstructed"
        );

        // Verify that we have reasonable consolidation of delete segments
        let delete_segments: Vec<_> = result
            .old_changes
            .iter()
            .filter(|c| c.change_type == super::super::WordChangeType::Delete)
            .collect();

        // After the fix, we should have consolidated segments
        // The exact number depends on implementation, but it should be reasonable
        assert!(
            delete_segments.len() <= 3,
            "Should have consolidated delete segments, got {} segments",
            delete_segments.len()
        );
    }

    #[test]
    fn test_multiple_word_changes_with_spaces() {
        let old_line = "The quick brown fox";
        let new_line = "A fast red dog";
        let result = highlight_word_diff_structured(old_line, new_line, 10000);

        // Verify that the complete text is preserved when reconstructed from all segments
        assert_eq!(reconstruct_text(&result.old_changes), old_line);
        assert_eq!(reconstruct_text(&result.new_changes), new_line);
    }

    #[test]
    fn test_partial_changes_preserve_spaces() {
        let old_line = "hello world test";
        let new_line = "hello universe test";
        let result = highlight_word_diff_structured(old_line, new_line, 10000);

        // Should have equal segments for "hello " and " test"
        let _equal_segments: Vec<_> = result
            .old_changes
            .iter()
            .filter(|c| c.change_type == super::super::WordChangeType::Equal)
            .collect();

        // Should have exactly one delete segment for "world"
        let delete_segments: Vec<_> = result
            .old_changes
            .iter()
            .filter(|c| c.change_type == super::super::WordChangeType::Delete)
            .collect();

        assert_eq!(delete_segments.len(), 1);
        assert_eq!(delete_segments[0].text, "world");
    }

    #[test]
    fn test_html_fragmentation_fix() {
        // This test verifies that the fix prevents the HTML fragmentation issue
        // described in the original bug report
        let old_line = "No checks to send at BK";
        let new_line = "Different message entirely";
        let result = highlight_word_diff_structured(old_line, new_line, 10000);

        // Count change segments - should be consolidated, not fragmented
        let old_delete_segments: Vec<_> = result
            .old_changes
            .iter()
            .filter(|c| c.change_type == super::super::WordChangeType::Delete)
            .collect();

        let new_insert_segments: Vec<_> = result
            .new_changes
            .iter()
            .filter(|c| c.change_type == super::super::WordChangeType::Insert)
            .collect();

        // The key success criteria: we should have consolidated segments
        // For a complete replacement like this, we expect exactly 1 delete and 1 insert segment
        assert_eq!(
            old_delete_segments.len(),
            1,
            "Should have exactly 1 consolidated delete segment"
        );
        assert_eq!(
            new_insert_segments.len(),
            1,
            "Should have exactly 1 consolidated insert segment"
        );

        // Verify the segments contain the complete text with spaces
        assert_eq!(old_delete_segments[0].text, old_line);
        assert_eq!(new_insert_segments[0].text, new_line);

        // Verify reconstruction preserves all text
        assert_eq!(reconstruct_text(&result.old_changes), old_line);
        assert_eq!(reconstruct_text(&result.new_changes), new_line);

        // Verify that spaces are preserved in the consolidated segments
        assert!(
            old_delete_segments[0].text.contains(' '),
            "Delete segment should contain spaces: '{}'",
            old_delete_segments[0].text
        );
        assert!(
            new_insert_segments[0].text.contains(' '),
            "Insert segment should contain spaces: '{}'",
            new_insert_segments[0].text
        );
    }

    #[test]
    fn test_is_whitespace_only() {
        assert!(is_whitespace_only(" "));
        assert!(is_whitespace_only("  "));
        assert!(is_whitespace_only("\t"));
        assert!(is_whitespace_only(" \t "));
        assert!(is_whitespace_only("\n"));
        assert!(!is_whitespace_only("a"));
        assert!(!is_whitespace_only(" a "));
        assert!(!is_whitespace_only(""));
    }

    #[test]
    fn test_consolidation_with_mixed_segments() {
        // Test consolidation when we have mixed segment types
        let old_line = "keep this but change that";
        let new_line = "keep this but modify something";
        let result = highlight_word_diff_structured(old_line, new_line, 10000);

        // Should have some equal segments and some change segments
        let equal_count = result
            .old_changes
            .iter()
            .filter(|c| c.change_type == super::super::WordChangeType::Equal)
            .count();
        let delete_count = result
            .old_changes
            .iter()
            .filter(|c| c.change_type == super::super::WordChangeType::Delete)
            .count();

        // Should preserve equal parts and consolidate changes
        assert!(equal_count > 0, "Should have some equal segments");
        assert!(delete_count > 0, "Should have some delete segments");

        // Verify complete reconstruction
        assert_eq!(reconstruct_text(&result.old_changes), old_line);
        assert_eq!(reconstruct_text(&result.new_changes), new_line);
    }

    #[test]
    fn test_original_fragmentation_problem() {
        // This test demonstrates the specific problem mentioned in the request:
        // fragmented sequences like Add("word"), Equal(" "), Add("other")
        // should become Add("word other")
        let old_line = "original text here";
        let new_line = "new different content";
        let result = highlight_word_diff_structured(old_line, new_line, 10000);

        // The key test: ensure we don't have fragmented <ins>/<del> tags
        // by checking that consecutive change operations are consolidated

        // Count total segments vs change segments -
        // fragmented output would have many more total segments due to spaces
        let total_old_segments = result.old_changes.len();
        let change_old_segments = result
            .old_changes
            .iter()
            .filter(|c| c.change_type != super::super::WordChangeType::Equal)
            .count();

        let total_new_segments = result.new_changes.len();
        let change_new_segments = result
            .new_changes
            .iter()
            .filter(|c| c.change_type != super::super::WordChangeType::Equal)
            .count();

        // With proper consolidation, the ratio of total to change segments should be reasonable
        // (not like 10 total segments for 1 actual change due to space fragmentation)

        // For completely different strings, we expect minimal segmentation
        assert!(
            total_old_segments <= 2,
            "Should have minimal old segments, got {}",
            total_old_segments
        );
        assert!(
            total_new_segments <= 2,
            "Should have minimal new segments, got {}",
            total_new_segments
        );
        assert_eq!(
            change_old_segments, 1,
            "Should have exactly 1 delete segment"
        );
        assert_eq!(
            change_new_segments, 1,
            "Should have exactly 1 insert segment"
        );

        // Verify the change segments contain spaces (proving consolidation worked)
        let delete_segment = result
            .old_changes
            .iter()
            .find(|c| c.change_type == super::super::WordChangeType::Delete)
            .unwrap();
        let insert_segment = result
            .new_changes
            .iter()
            .find(|c| c.change_type == super::super::WordChangeType::Insert)
            .unwrap();

        assert!(
            delete_segment.text.contains(' '),
            "Delete segment should contain consolidated spaces"
        );
        assert!(
            insert_segment.text.contains(' '),
            "Insert segment should contain consolidated spaces"
        );
        assert_eq!(
            delete_segment.text, old_line,
            "Delete segment should be the complete old line"
        );
        assert_eq!(
            insert_segment.text, new_line,
            "Insert segment should be the complete new line"
        );
    }
}
