use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use strsim::levenshtein;
use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter, Style as SyntectStyle,
};
use syntect::parsing::{ParseState, ScopeStack};

use crate::{get_syntax_set, get_theme};

mod word_diff;

use word_diff::highlight_word_diff;

/// Helper function to process style-text pairs into HTML with span tags
fn process_style_text_pairs(
    style_text_pairs: Vec<(SyntectStyle, &str)>,
    initial_capacity: usize,
) -> String {
    let mut html = String::with_capacity(initial_capacity);
    let mut last_style = String::new();
    let mut accumulated_text = String::new();

    let flush_accumulated = |html: &mut String, accumulated_text: &mut String, style: &str| {
        if !accumulated_text.is_empty() {
            if style.is_empty() {
                html.push_str(accumulated_text);
            } else {
                html.push_str(&format!("<span {style}>{accumulated_text}</span>"));
            }
            accumulated_text.clear();
        }
    };

    for (style, text) in style_text_pairs {
        let css_style = syntect_style_to_css(style);

        if css_style == last_style {
            accumulated_text.push_str(&html_escape::encode_text(text));
        } else {
            flush_accumulated(&mut html, &mut accumulated_text, &last_style);
            last_style = css_style;
            accumulated_text = html_escape::encode_text(text).to_string();
        }
    }

    flush_accumulated(&mut html, &mut accumulated_text, &last_style);
    html
}

// Constants for diff processing
const MAX_WORD_DIFF_LINE_LENGTH: usize = 10000;
const WORD_SIMILARITY_THRESHOLD: f64 = 0.25;
const LINE_SIMILARITY_THRESHOLD: f64 = 0.25;
const MAX_FILE_SIZE_BYTES: usize = 100000;
const MAX_FILE_LINES: usize = 5000;

#[derive(Debug, Clone)]
pub struct TemplateAnnotation {
    pub desc: String,
    pub line: u64,
    pub col_start: u64,
    pub col_end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineType {
    Add,
    Delete,
    Context,
    Hunk,
    HunkSeparator,
}

impl LineType {
    pub fn as_str(&self) -> &'static str {
        match self {
            LineType::Add => "add",
            LineType::Delete => "del",
            LineType::Context => "ctx",
            LineType::Hunk => "hunk",
            LineType::HunkSeparator => "hunk-separator",
        }
    }
}

#[derive(Debug)]
pub struct DiffLine {
    pub line_type: LineType,
    pub old_line_number: Option<i32>,
    pub new_line_number: Option<i32>,
    pub html_content: String, // syntax highlighted HTML for display
    pub annotations: Vec<TemplateAnnotation>, // annotations for this specific line
    pub raw_content: String,  // original content without highlighting for word diff
}

impl DiffLine {
    pub fn line_type_str(&self) -> &'static str {
        self.line_type.as_str()
    }

    pub fn is_hunk(&self) -> bool {
        self.line_type == LineType::Hunk
    }

    pub fn is_add(&self) -> bool {
        self.line_type == LineType::Add
    }

    pub fn is_delete(&self) -> bool {
        self.line_type == LineType::Delete
    }
}

#[derive(Debug)]
pub struct FileDiff {
    pub filename_before: String,
    pub filename_after: String,
    pub is_binary: bool,
    pub is_large: bool,
    pub lines: Vec<DiffLine>,
}

#[derive(Deserialize, Debug, Serialize, Clone)]
pub struct Annotations {
    pub ty: u64,
    pub desc: String,
    pub severity: u64,
    pub line: Option<u64>,
    pub col_start: Option<u64>,
    pub col_end: Option<u64>,
    pub extra: Option<String>,
}

fn syntect_style_to_css(style: SyntectStyle) -> String {
    let fg = style.foreground;

    let mut style_parts = Vec::new();
    let mut class_parts: Vec<&str> = Vec::new();

    if style.foreground.a > 0 {
        style_parts.push(format!("color:#{:02x}{:02x}{:02x}", fg.r, fg.g, fg.b));
    }

    if style.font_style.contains(FontStyle::BOLD) {
        class_parts.push("b");
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        class_parts.push("i");
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        class_parts.push("u");
    }

    let mut result = String::new();

    if !class_parts.is_empty() {
        result.push_str(&format!("class=\"{}\"", class_parts.join(" ")));
    }

    if !style_parts.is_empty() {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&format!("style=\"{}\"", style_parts.join(";")));
    }

    result
}

fn highlight_code_safely(content: &str, filename: &str) -> String {
    let syntax_set = get_syntax_set();
    let theme = get_theme();

    let syntax = syntax_set
        .find_syntax_for_file(filename)
        .unwrap_or(None)
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

    let mut parse_state = ParseState::new(syntax);
    let highlighter = Highlighter::new(theme);
    let mut highlight_state = HighlightState::new(&highlighter, ScopeStack::new());

    let ops = match parse_state.parse_line(content, syntax_set) {
        Ok(ops) => ops,
        Err(e) => {
            eprintln!("Parse error for single line '{content}' in {filename}: {e}");
            return html_escape::encode_text(content).to_string();
        }
    };

    let highlight_iter =
        HighlightIterator::new(&mut highlight_state, &ops[..], content, &highlighter);
    let style_text_pairs: Vec<(SyntectStyle, &str)> = highlight_iter.collect();

    if is_invalid_scope(&highlight_state.path) {
        return html_escape::encode_text(content).to_string();
    }

    process_style_text_pairs(style_text_pairs, content.len() * 2)
}

pub fn highlight_hunk_lines(
    hunk_lines: &[(String, LineType, usize)],
    filename: &str,
) -> Vec<String> {
    if hunk_lines.is_empty() {
        return Vec::new();
    }

    let syntax_set = get_syntax_set();
    let theme = get_theme();

    let syntax = syntax_set
        .find_syntax_for_file(filename)
        .unwrap_or(None)
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

    let mut parse_state = ParseState::new(syntax);
    let highlighter = Highlighter::new(theme);
    let mut highlight_state = HighlightState::new(&highlighter, ScopeStack::new());
    let mut results = Vec::new();

    for (content, _, _) in hunk_lines {
        let previous_parse_state = parse_state.clone();

        // Parse the line to get scope operations
        let ops = match parse_state.parse_line(content, syntax_set) {
            Ok(ops) => ops,
            Err(e) => {
                eprintln!("Parse error for line '{content}' in {filename}: {e}");
                results.push(highlight_code_safely(content, filename));
                continue;
            }
        };

        // Check if we have invalid scopes, if so fall back for this line
        if is_invalid_scope(&highlight_state.path) {
            results.push(highlight_code_safely(content, filename));
            continue;
        }

        let scope_stack_before = highlight_state.path.clone();

        let highlight_iter =
            HighlightIterator::new(&mut highlight_state, &ops[..], content, &highlighter);
        let style_text_pairs: Vec<(SyntectStyle, &str)> = highlight_iter.collect();

        let html = process_style_text_pairs(style_text_pairs, content.len() * 2);
        results.push(html);
        // Check if we ended in a single-line comment scope - if so, reset state for next line
        if should_reset_parser_state(&highlight_state.path, &scope_stack_before) {
            parse_state = previous_parse_state;
            highlight_state = HighlightState::new(&highlighter, scope_stack_before);
        }
    }

    results
}

/// Check if a scope string represents a single-line comment
fn is_single_line_comment(scope_str: &str) -> bool {
    scope_str.contains("comment.line")
        || scope_str == "comment"
        || (scope_str.starts_with("comment.") && !scope_str.contains("block"))
}

/// Determine if parser state should be reset after this line
/// This helps prevent single-line constructs like # comments from bleeding into next lines
/// while allowing multi-line constructs like triple-quoted strings to persist
fn should_reset_parser_state(current_scopes: &ScopeStack, _previous_scopes: &ScopeStack) -> bool {
    use syntect::parsing::SCOPE_REPO;
    let repo = SCOPE_REPO.lock().unwrap();

    // Check if we're currently in a single-line comment scope
    current_scopes.scopes.iter().any(|scope| {
        let scope_str = repo.to_string(*scope);
        is_single_line_comment(&scope_str)
    })
}

/// Find annotations for a specific line number
pub fn find_line_annotations(
    line_number: i32,
    all_annotations: &[TemplateAnnotation],
) -> Vec<TemplateAnnotation> {
    if line_number <= 0 {
        return Vec::new();
    }

    all_annotations
        .iter()
        .filter(|ann| ann.line == line_number as u64)
        .cloned()
        .collect()
}

/// Check if a scope string represents invalid syntax
fn is_invalid_scope_str(scope_str: &str) -> bool {
    scope_str.starts_with("invalid.")
}

/// Check if a scope represents an error or invalid syntax using TextMate conventions
fn is_invalid_scope(scope_stack: &ScopeStack) -> bool {
    // In TextMate/Sublime Text scope conventions, invalid syntax is marked with scopes starting with "invalid."
    use syntect::parsing::SCOPE_REPO;
    let repo = SCOPE_REPO.lock().unwrap();

    scope_stack.scopes.iter().any(|scope| {
        let scope_str = repo.to_string(*scope);
        is_invalid_scope_str(&scope_str)
    })
}

/// Apply diff styling to highlighted content
fn apply_diff_styling(highlighted_content: &str, line_type: LineType) -> String {
    match line_type {
        LineType::Add => format!("<span class='da'>{highlighted_content}</span>"),
        LineType::Delete => format!("<span class='dd'>{highlighted_content}</span>"),
        LineType::Hunk => format!("<span class='hunk-header'>{highlighted_content}</span>"),
        _ => highlighted_content.to_string(),
    }
}

/// Analyze file size and line count efficiently
fn analyze_file_size(content: &str) -> (bool, usize) {
    let byte_size = content.len();
    if byte_size > MAX_FILE_SIZE_BYTES {
        return (true, 0); // Don't bother counting lines if bytes already exceed limit
    }

    let line_count = content.lines().count();
    let is_large = line_count > MAX_FILE_LINES;

    (is_large, line_count)
}

/// Apply word-level highlighting to pairs of add/delete lines
fn apply_word_highlighting(diff_lines: &mut [DiffLine]) {
    let mut i = 0;

    while i < diff_lines.len() {
        // Find sequences of delete and add lines to match optimally
        if diff_lines[i].line_type == LineType::Delete {
            let mut delete_indices = Vec::new();
            let mut add_indices = Vec::new();

            // Collect consecutive delete lines
            let mut j = i;
            while j < diff_lines.len() && diff_lines[j].line_type == LineType::Delete {
                delete_indices.push(j);
                j += 1;
            }

            // Collect consecutive add lines that follow
            while j < diff_lines.len() && diff_lines[j].line_type == LineType::Add {
                add_indices.push(j);
                j += 1;
            }

            // If we have both deletes and adds, find optimal pairings
            if !delete_indices.is_empty() && !add_indices.is_empty() {
                let matches = find_best_line_matches(diff_lines, &delete_indices, &add_indices);

                // Apply word highlighting to matched pairs
                for (del_idx, add_idx) in matches {
                    let word_diff = highlight_word_diff(
                        &diff_lines[del_idx].raw_content,
                        &diff_lines[add_idx].raw_content,
                        MAX_WORD_DIFF_LINE_LENGTH,
                        WORD_SIMILARITY_THRESHOLD,
                    );

                    // Don't apply line-level styling wrapper for word-highlighted lines
                    // The <ins>/<del> tags provide sufficient visual indication
                    diff_lines[del_idx].html_content = word_diff.old_highlighted;
                    diff_lines[add_idx].html_content = word_diff.new_highlighted;
                }
            }

            i = j; // Skip past all processed lines
        } else {
            i += 1;
        }
    }
}

/// Find the best matches between delete and add lines using similarity scoring
fn find_best_line_matches(
    diff_lines: &[DiffLine],
    delete_indices: &[usize],
    add_indices: &[usize],
) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut used_deletes = vec![false; delete_indices.len()];
    let mut used_adds = vec![false; add_indices.len()];

    // Calculate similarity scores for all pairs
    let mut candidates = Vec::new();
    for (i, &del_idx) in delete_indices.iter().enumerate() {
        for (j, &add_idx) in add_indices.iter().enumerate() {
            let del_content = &diff_lines[del_idx].raw_content;
            let add_content = &diff_lines[add_idx].raw_content;

            let similarity = calculate_line_similarity(del_content, add_content);
            candidates.push((i, j, del_idx, add_idx, similarity));
        }
    }

    // Sort by similarity (higher is better)
    candidates.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));

    // Greedily select best matches
    for (del_i, add_i, del_idx, add_idx, similarity) in candidates {
        if !used_deletes[del_i] && !used_adds[add_i] && similarity > LINE_SIMILARITY_THRESHOLD {
            matches.push((del_idx, add_idx));
            used_deletes[del_i] = true;
            used_adds[add_i] = true;
        }
    }

    matches
}

/// Calculate similarity between two lines (0.0 = completely different, 1.0 = identical)
fn calculate_line_similarity(line1: &str, line2: &str) -> f64 {
    if line1.is_empty() && line2.is_empty() {
        return 1.0;
    }

    let distance = levenshtein(line1, line2);
    let max_len = std::cmp::max(line1.len(), line2.len()) as f64;

    if max_len == 0.0 {
        1.0
    } else {
        1.0 - (distance as f64 / max_len)
    }
}

/// Parse hunk header to extract starting line numbers
///
/// Parses lines like: @@ -old_start,old_count +new_start,new_count @@
/// Returns (old_start, new_start) or None if parsing fails
pub fn parse_hunk_header(line: &str) -> Option<(i32, i32)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let mut old_start = None;
    let mut new_start = None;

    for part in parts {
        if part.starts_with("-") && part.len() > 1 {
            let nums: Vec<&str> = part[1..].split(',').collect();
            if let Ok(start) = nums[0].parse::<i32>() {
                old_start = Some(start);
            }
        } else if part.starts_with("+") && part.len() > 1 {
            let nums: Vec<&str> = part[1..].split(',').collect();
            if let Ok(start) = nums[0].parse::<i32>() {
                new_start = Some(start);
            }
        }
    }

    // Only return if we successfully parsed both values
    match (old_start, new_start) {
        (Some(old), Some(new)) => Some((old, new)),
        _ => {
            eprintln!("Warning: Failed to parse hunk header: {line}");
            Some((1, 1)) // Fallback to line 1
        }
    }
}

/// Process accumulated hunk lines and convert them to DiffLines with syntax highlighting
fn process_hunk_lines(
    hunk_lines: &mut Vec<(String, LineType, usize)>,
    diff_lines: &mut Vec<DiffLine>,
    display_filename: &str,
    annotations: &BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
    version_id: &str,
) {
    // Get annotations for this specific file
    let file_annotations = annotations
        .get(version_id)
        .and_then(|version_annotations| {
            // Try to match filename from the diff
            version_annotations.get(display_filename)
        })
        .map(|anns| {
            anns.iter()
                .map(|a| TemplateAnnotation {
                    desc: a.desc.clone(),
                    line: a.line.unwrap_or(0),
                    col_start: a.col_start.unwrap_or(0),
                    col_end: a.col_end.unwrap_or(0),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if !hunk_lines.is_empty() {
        let highlighted_lines = highlight_hunk_lines(hunk_lines, display_filename);

        for (i, (content, line_type, line_num)) in hunk_lines.iter().enumerate() {
            let highlighted_content = highlighted_lines
                .get(i)
                .map(|h| h.as_str())
                .unwrap_or_else(|| content.as_str());

            let html_content = apply_diff_styling(highlighted_content, *line_type);

            let (old_num, new_num) = match line_type {
                LineType::Add => (-1, *line_num as i32),
                LineType::Delete => (*line_num as i32, -1),
                LineType::Context => (*line_num as i32, *line_num as i32),
                _ => (-1, -1),
            };

            // Show annotations for added lines and context lines
            let line_annotations = if (*line_type == LineType::Add
                || *line_type == LineType::Context)
                && new_num > 0
            {
                find_line_annotations(new_num, &file_annotations)
            } else {
                Vec::new()
            };

            diff_lines.push(DiffLine {
                line_type: *line_type,
                old_line_number: Some(old_num),
                new_line_number: Some(new_num),
                html_content,
                annotations: line_annotations,
                raw_content: content.clone(),
            });
        }
        hunk_lines.clear();
    }
}

/// Robust manual git diff parser for unified diff format
pub fn parse_git_diff(
    git_diff: &str,
    annotations: &BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
    version_id: &str,
) -> Vec<FileDiff> {
    let mut result = Vec::new();
    let file_diffs: Vec<&str> = git_diff.split("diff --git").collect();

    for file_diff in file_diffs {
        if file_diff.trim().is_empty() {
            continue;
        }

        let lines: Vec<&str> = file_diff.split('\n').collect();
        let mut filename_before = String::new();
        let mut filename_after = String::new();
        let mut is_binary = false;

        // Parse file headers
        for line in &lines {
            if line.starts_with(" a/") && line.contains(" b/") {
                let parts: Vec<&str> = line.split(" b/").collect();
                if parts.len() == 2 {
                    filename_before = parts[0].trim_start_matches(" a/").to_string();
                    filename_after = parts[1].to_string();
                }
            }
            if line.starts_with("+++") {
                filename_after = line[4..].trim_start_matches("b/").to_string();
            }
            if line.starts_with("---") {
                filename_before = line[4..].trim_start_matches("a/").to_string();
            }
            if line.starts_with("deleted file mode") {
                filename_after = "/dev/null".to_string();
            }
            if line.starts_with("new file mode") {
                filename_before = "/dev/null".to_string();
            }
            if line.starts_with("Binary files") {
                is_binary = true;
            }
        }

        if filename_before.is_empty() && filename_after.is_empty() {
            continue;
        }

        let display_filename = if filename_after != "/dev/null" {
            &filename_after
        } else {
            &filename_before
        };

        let (is_large, _line_count) = analyze_file_size(file_diff);

        let mut diff_lines = Vec::new();
        let mut current_hunk_lines: Vec<(String, LineType, usize)> = Vec::new(); // (content, line_type, line_number)
        let mut old_line = 0;
        let mut new_line = 0;
        let mut in_hunk = false;

        for line in &lines {
            if line.trim() == "\\ No newline at end of file" {
                continue;
            }

            if line.starts_with("@@") {
                // Process any accumulated hunk lines before starting a new hunk
                process_hunk_lines(
                    &mut current_hunk_lines,
                    &mut diff_lines,
                    display_filename,
                    annotations,
                    version_id,
                );

                // Add hunk separator if this isn't the first hunk
                if in_hunk {
                    diff_lines.push(DiffLine {
                        line_type: LineType::HunkSeparator,
                        old_line_number: None,
                        new_line_number: None,
                        html_content: String::new(),
                        annotations: Vec::new(),
                        raw_content: String::new(),
                    });
                }

                if let Some((old_start, new_start)) = parse_hunk_header(line) {
                    old_line = old_start as u32;
                    new_line = new_start as u32;
                }

                diff_lines.push(DiffLine {
                    line_type: LineType::Hunk,
                    old_line_number: None,
                    new_line_number: None,
                    html_content: format!(
                        "<span class='hunk-header'>{}</span>",
                        html_escape::encode_text(line)
                    ),
                    annotations: Vec::new(),
                    raw_content: line.to_string(),
                });
                in_hunk = true;
            } else if in_hunk {
                if line.starts_with("+") && !line.starts_with("+++") {
                    current_hunk_lines.push((
                        line[1..].to_string(),
                        LineType::Add,
                        new_line as usize,
                    ));
                    new_line += 1;
                } else if line.starts_with("-") && !line.starts_with("---") {
                    current_hunk_lines.push((
                        line[1..].to_string(),
                        LineType::Delete,
                        old_line as usize,
                    ));
                    old_line += 1;
                } else if line.starts_with(" ")
                    || (!line.starts_with("diff")
                        && !line.starts_with("index")
                        && !line.starts_with("+++")
                        && !line.starts_with("---")
                        && !line.is_empty())
                {
                    let content = line.strip_prefix(" ").unwrap_or(line);
                    current_hunk_lines.push((
                        content.to_string(),
                        LineType::Context,
                        old_line as usize,
                    ));
                    old_line += 1;
                    new_line += 1;
                }
            }
        }

        // Process the final hunk
        process_hunk_lines(
            &mut current_hunk_lines,
            &mut diff_lines,
            display_filename,
            annotations,
            version_id,
        );

        // Apply word-level highlighting to add/delete line pairs
        apply_word_highlighting(&mut diff_lines);

        result.push(FileDiff {
            filename_before,
            filename_after,
            is_binary,
            is_large,
            lines: diff_lines,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_similarity() {
        // Identical lines should have similarity 1.0
        assert_eq!(calculate_line_similarity("hello world", "hello world"), 1.0);

        // Similar lines should have high similarity
        let sim1 = calculate_line_similarity(
            "[Main Page](../../../../games/OpenRCT2/info/en)",
            "[Main Page](../info/en)",
        );
        let sim2 = calculate_line_similarity(
            "[Options Page](../../../../games/OpenRCT2/player-options)",
            "[Options Page](../player-options)",
        );

        // Both should be similar (around 0.4-0.6)
        assert!(sim1 > 0.4, "sim1 = {}", sim1);
        assert!(sim2 > 0.4, "sim2 = {}", sim2);

        // Different lines should have low similarity
        let sim3 = calculate_line_similarity(
            "[Main Page](../../../../games/OpenRCT2/info/en)",
            "[Options Page](../player-options)",
        );
        assert!(sim3 < 0.7, "sim3 = {}", sim3);

        // The similar lines should be more similar than different ones
        assert!(sim1 > sim3, "sim1 ({}) should be > sim3 ({})", sim1, sim3);
    }
}
