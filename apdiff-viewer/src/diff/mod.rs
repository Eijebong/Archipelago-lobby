use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use strsim::levenshtein;
use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter, Style as SyntectStyle,
};
use syntect::parsing::{ParseState, ScopeStack};

use crate::{get_syntax_set, get_theme};

mod word_diff;

use word_diff::highlight_word_diff_structured;

/// Represents a syntax highlighting token with style information
#[derive(Debug, Clone)]
pub struct SyntaxToken {
    pub text: String,
    pub style: SyntectStyle,
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Represents a word diff change segment
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum WordChangeType {
    Insert,
    Delete,
    Equal,
}

#[derive(Debug, Clone)]
pub struct WordChangeSegment {
    pub text: String,
    pub change_type: WordChangeType,
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Merge syntax highlighting tokens with word diff changes
fn merge_syntax_and_word_highlighting(
    syntax_tokens: &[SyntaxToken],
    word_changes: &[WordChangeSegment],
) -> String {
    word_changes
        .iter()
        .map(|change| render_word_change_with_syntax(change, syntax_tokens))
        .collect::<Vec<_>>()
        .join("")
}

/// Render a single word change with syntax highlighting
fn render_word_change_with_syntax(
    change: &WordChangeSegment,
    syntax_tokens: &[SyntaxToken],
) -> String {
    let overlapping_tokens: Vec<_> = syntax_tokens
        .iter()
        .filter(|token| {
            token.end_offset > change.start_offset && token.start_offset < change.end_offset
        })
        .collect();

    let content = match overlapping_tokens.len() {
        0 => html_escape::encode_text(&change.text).to_string(),
        1 => render_single_token_segment(&change.text, overlapping_tokens[0]),
        _ => render_multi_token_segment(change, &overlapping_tokens),
    };

    wrap_with_change_tags(&content, change.change_type)
}

/// Render content with a single syntax token
fn render_single_token_segment(text: &str, token: &SyntaxToken) -> String {
    let escaped_text = html_escape::encode_text(text);
    let css_style = syntect_style_to_css(token.style);

    if css_style.is_empty() {
        escaped_text.to_string()
    } else {
        format!("<span {css_style}>{escaped_text}</span>")
    }
}

/// Render content with multiple overlapping syntax tokens
fn render_multi_token_segment(
    change: &WordChangeSegment,
    overlapping_tokens: &[&SyntaxToken],
) -> String {
    let mut result = String::new();
    let mut pos = change.start_offset;

    // Process each overlapping token in sequence
    for token in overlapping_tokens {
        let segment_start = pos.max(token.start_offset);
        let segment_end = change.end_offset.min(token.end_offset);

        if segment_start < segment_end {
            let text_range = (
                segment_start.saturating_sub(change.start_offset),
                segment_end.saturating_sub(change.start_offset),
            );

            if let Some(segment_text) = change.text.get(text_range.0..text_range.1) {
                let escaped_segment = html_escape::encode_text(segment_text);
                let css_style = syntect_style_to_css(token.style);

                if css_style.is_empty() {
                    result.push_str(&escaped_segment);
                } else {
                    result.push_str(&format!("<span {css_style}>{escaped_segment}</span>"));
                }
            }

            pos = segment_end;
        }
    }

    // Handle any remaining text not covered by syntax tokens
    if pos < change.end_offset {
        let remaining_start = pos.saturating_sub(change.start_offset);
        if let Some(remaining_text) = change.text.get(remaining_start..) {
            result.push_str(&html_escape::encode_text(remaining_text));
        }
    }

    result
}

/// Wrap content with appropriate change tags
fn wrap_with_change_tags(content: &str, change_type: WordChangeType) -> String {
    match change_type {
        WordChangeType::Insert => format!("<ins>{content}</ins>"),
        WordChangeType::Delete => format!("<del>{content}</del>"),
        WordChangeType::Equal => content.to_string(),
    }
}

/// Process style-text pairs into HTML with span tags
fn process_style_text_pairs(
    style_text_pairs: Vec<(SyntectStyle, &str)>,
    initial_capacity: usize,
) -> String {
    #[derive(Default)]
    struct StyleAccumulator {
        html: String,
        last_style: String,
        accumulated_text: String,
    }

    impl StyleAccumulator {
        fn new(capacity: usize) -> Self {
            Self {
                html: String::with_capacity(capacity),
                ..Default::default()
            }
        }

        fn flush(&mut self) {
            if !self.accumulated_text.is_empty() {
                if self.last_style.is_empty() {
                    self.html.push_str(&self.accumulated_text);
                } else {
                    self.html.push_str(&format!(
                        "<span {}>{}</span>",
                        self.last_style, self.accumulated_text
                    ));
                }
                self.accumulated_text.clear();
            }
        }

        fn process_style_text(&mut self, style: SyntectStyle, text: &str) {
            let css_style = syntect_style_to_css(style);

            if css_style == self.last_style {
                self.accumulated_text
                    .push_str(&html_escape::encode_text(text));
            } else {
                self.flush();
                self.last_style = css_style;
                self.accumulated_text = html_escape::encode_text(text).to_string();
            }
        }
    }

    let mut accumulator = style_text_pairs.into_iter().fold(
        StyleAccumulator::new(initial_capacity),
        |mut acc, (style, text)| {
            acc.process_style_text(style, text);
            acc
        },
    );

    accumulator.flush();
    accumulator.html
}

const MAX_WORD_DIFF_LINE_LENGTH: usize = 10000;
const MAX_LINE_SIMILARITY_LENGTH: usize = 1000;
const LINE_SIMILARITY_THRESHOLD: f64 = 0.25;
const MAX_BLOCK_SIZE_FOR_MATCHING: usize = 200;
const LENGTH_SIMILARITY_RATIO: f64 = 0.5;

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
    pub annotations: Vec<TemplateAnnotation>, // annotations for this specific line
    pub raw_content: String,                  // original content without highlighting
    pub syntax_tokens: Vec<SyntaxToken>,      // parsed syntax highlighting tokens
    pub word_changes: Option<Vec<WordChangeSegment>>, // word-level diff changes
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

    /// Generate HTML content by merging syntax highlighting with word diff changes
    pub fn html_content(&self) -> String {
        match self.line_type {
            LineType::Hunk => {
                format!(
                    "<span class='hunk-header'>{}</span>",
                    html_escape::encode_text(&self.raw_content)
                )
            }
            LineType::HunkSeparator => String::new(),
            _ => {
                if let Some(ref word_changes) = self.word_changes {
                    // Use word changes with merged syntax highlighting
                    merge_syntax_and_word_highlighting(&self.syntax_tokens, word_changes)
                } else {
                    // Use only syntax highlighting
                    let style_text_pairs: Vec<(SyntectStyle, &str)> = self
                        .syntax_tokens
                        .iter()
                        .map(|token| (token.style, token.text.as_str()))
                        .collect();
                    let highlighted =
                        process_style_text_pairs(style_text_pairs, self.raw_content.len() * 2);
                    apply_diff_styling(&highlighted, self.line_type)
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct FileDiff {
    pub filename_before: String,
    pub filename_after: String,
    pub is_binary: bool,
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
    let style_parts: Vec<String> = std::iter::empty()
        .chain((style.foreground.a > 0).then(|| {
            format!(
                "color:#{:02x}{:02x}{:02x}",
                style.foreground.r, style.foreground.g, style.foreground.b
            )
        }))
        .collect();

    let class_parts: Vec<&str> = [
        (style.font_style.contains(FontStyle::BOLD), "b"),
        (style.font_style.contains(FontStyle::ITALIC), "i"),
        (style.font_style.contains(FontStyle::UNDERLINE), "u"),
    ]
    .iter()
    .filter_map(|(condition, class)| condition.then_some(*class))
    .collect();

    let class_attr =
        (!class_parts.is_empty()).then(|| format!("class=\"{}\"", class_parts.join(" ")));

    let style_attr =
        (!style_parts.is_empty()).then(|| format!("style=\"{}\"", style_parts.join(";")));

    [class_attr, style_attr]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn highlight_hunk_lines(
    hunk_lines: &[(String, LineType, (i32, i32))],
    filename: &str,
) -> Vec<Vec<SyntaxToken>> {
    let syntax_set = get_syntax_set();
    let syntax = syntax_set
        .find_syntax_for_file(filename)
        .unwrap_or(None)
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

    let mut syntax_highlighter = SyntaxHighlighter::new(syntax);

    hunk_lines
        .iter()
        .map(|(content, _, _)| syntax_highlighter.highlight_line(content))
        .collect()
}

/// Encapsulates syntax highlighting state for processing multiple lines
struct SyntaxHighlighter {
    parse_state: ParseState,
    highlight_state: HighlightState,
    highlighter: Highlighter<'static>,
}

impl SyntaxHighlighter {
    fn new(syntax: &'static syntect::parsing::SyntaxReference) -> Self {
        let theme = get_theme();
        let highlighter = Highlighter::new(theme);

        Self {
            parse_state: ParseState::new(syntax),
            highlight_state: HighlightState::new(&highlighter, ScopeStack::new()),
            highlighter,
        }
    }

    fn highlight_line(&mut self, content: &str) -> Vec<SyntaxToken> {
        let previous_parse_state = self.parse_state.clone();

        // Parse the line to get scope operations
        let ops = match self.parse_state.parse_line(content, get_syntax_set()) {
            Ok(ops) => ops,
            Err(_) => return fallback_syntax_tokens(content),
        };

        // Check for invalid scopes
        if is_invalid_scope(&self.highlight_state.path) {
            return fallback_syntax_tokens(content);
        }

        let scope_stack_before = self.highlight_state.path.clone();
        let highlight_iter = HighlightIterator::new(
            &mut self.highlight_state,
            &ops[..],
            content,
            &self.highlighter,
        );

        let tokens = highlight_iter
            .scan(0usize, |offset, (style, text)| {
                let start_offset = *offset;
                *offset += text.len();
                Some(SyntaxToken {
                    text: text.to_string(),
                    style,
                    start_offset,
                    end_offset: *offset,
                })
            })
            .collect();

        // Reset state if needed for single-line constructs
        if should_reset_parser_state(&self.highlight_state.path) {
            self.parse_state = previous_parse_state;
            self.highlight_state = HighlightState::new(&self.highlighter, scope_stack_before);
        }

        tokens
    }
}

/// Create fallback syntax tokens when parsing fails
fn fallback_syntax_tokens(content: &str) -> Vec<SyntaxToken> {
    vec![SyntaxToken {
        text: content.to_string(),
        style: SyntectStyle::default(),
        start_offset: 0,
        end_offset: content.len(),
    }]
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
fn should_reset_parser_state(current_scopes: &ScopeStack) -> bool {
    // Check if we're currently in a single-line comment scope
    current_scopes.scopes.iter().any(|scope| {
        let scope_str = scope.build_string();
        is_single_line_comment(&scope_str)
    })
}

/// Find annotations for a specific line number
pub fn find_line_annotations(
    line_number: i32,
    all_annotations: &[TemplateAnnotation],
) -> Vec<TemplateAnnotation> {
    match line_number {
        n if n <= 0 => Vec::new(),
        n => all_annotations
            .iter()
            .filter(|ann| ann.line == n as u64)
            .cloned()
            .collect(),
    }
}

/// Check if a scope string represents invalid syntax
fn is_invalid_scope_str(scope_str: &str) -> bool {
    scope_str.starts_with("invalid.")
}

/// Check if a scope represents an error or invalid syntax
fn is_invalid_scope(scope_stack: &ScopeStack) -> bool {
    // In TextMate/Sublime Text scope conventions, invalid syntax is marked with scopes starting with "invalid."
    scope_stack.scopes.iter().any(|scope| {
        let scope_str = scope.build_string();
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

/// Apply word-level highlighting to pairs of add/delete lines
fn apply_word_highlighting(diff_lines: &mut [DiffLine]) {
    let change_blocks = find_change_blocks(diff_lines);

    // Process each change block independently
    for (delete_indices, add_indices) in change_blocks {
        if !delete_indices.is_empty() && !add_indices.is_empty() {
            let matches = find_best_line_matches(diff_lines, &delete_indices, &add_indices);

            matches.into_iter().for_each(|(del_idx, add_idx)| {
                let word_diff = highlight_word_diff_structured(
                    &diff_lines[del_idx].raw_content,
                    &diff_lines[add_idx].raw_content,
                    MAX_WORD_DIFF_LINE_LENGTH,
                );

                // Store structured word changes for later HTML generation
                diff_lines[del_idx].word_changes = Some(word_diff.old_changes);
                diff_lines[add_idx].word_changes = Some(word_diff.new_changes);
            });
        }
    }
}

/// Find consecutive blocks of delete+add lines
fn find_change_blocks(diff_lines: &[DiffLine]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < diff_lines.len() {
        if diff_lines[i].line_type == LineType::Delete {
            // Collect consecutive delete lines
            let delete_start = i;
            while i < diff_lines.len() && diff_lines[i].line_type == LineType::Delete {
                i += 1;
            }
            let delete_indices: Vec<usize> = (delete_start..i).collect();

            // Collect consecutive add lines that follow
            let add_start = i;
            while i < diff_lines.len() && diff_lines[i].line_type == LineType::Add {
                i += 1;
            }
            let add_indices: Vec<usize> = (add_start..i).collect();

            blocks.push((delete_indices, add_indices));
        } else {
            i += 1;
        }
    }

    blocks
}

fn find_windowed_matches(
    diff_lines: &[DiffLine],
    delete_indices: &[usize],
    add_indices: &[usize],
) -> Vec<(usize, usize)> {
    const WINDOW_SIZE: usize = 50;

    let mut candidates: Vec<_> = delete_indices
        .iter()
        .enumerate()
        .flat_map(|(del_i, &del_idx)| {
            let del_line = &diff_lines[del_idx].raw_content;
            let del_len = del_line.len();

            let window_start = del_i.saturating_sub(WINDOW_SIZE);
            let window_end = (del_i + WINDOW_SIZE).min(add_indices.len());

            (window_start..window_end).filter_map(move |add_i| {
                let add_idx = add_indices[add_i];
                let add_line = &diff_lines[add_idx].raw_content;
                let add_len = add_line.len();

                if !are_lengths_similar(del_len, add_len) {
                    return None;
                }

                let similarity = calculate_line_similarity(del_line, add_line);

                if similarity < LINE_SIMILARITY_THRESHOLD {
                    return None;
                }

                Some((del_i, add_i, del_idx, add_idx, similarity))
            })
        })
        .collect();

    candidates.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));

    let (matches, _, _) = candidates.into_iter().fold(
        (
            Vec::new(),
            vec![false; delete_indices.len()],
            vec![false; add_indices.len()],
        ),
        |(mut matches, mut used_deletes, mut used_adds),
         (del_i, add_i, del_idx, add_idx, similarity)| {
            if !used_deletes[del_i] && !used_adds[add_i] && similarity > LINE_SIMILARITY_THRESHOLD {
                matches.push((del_idx, add_idx));
                used_deletes[del_i] = true;
                used_adds[add_i] = true;
            }
            (matches, used_deletes, used_adds)
        },
    );

    matches
}

fn find_best_line_matches(
    diff_lines: &[DiffLine],
    delete_indices: &[usize],
    add_indices: &[usize],
) -> Vec<(usize, usize)> {
    if delete_indices.len() > MAX_BLOCK_SIZE_FOR_MATCHING
        || add_indices.len() > MAX_BLOCK_SIZE_FOR_MATCHING
    {
        return find_windowed_matches(diff_lines, delete_indices, add_indices);
    }

    let mut candidates: Vec<_> = delete_indices
        .iter()
        .enumerate()
        .flat_map(|(i, &del_idx)| {
            let del_line = &diff_lines[del_idx].raw_content;
            let del_len = del_line.len();

            add_indices
                .iter()
                .enumerate()
                .filter_map(move |(j, &add_idx)| {
                    let add_line = &diff_lines[add_idx].raw_content;
                    let add_len = add_line.len();

                    if !are_lengths_similar(del_len, add_len) {
                        return None;
                    }

                    let similarity = calculate_line_similarity(del_line, add_line);

                    if similarity < LINE_SIMILARITY_THRESHOLD {
                        return None;
                    }

                    Some((i, j, del_idx, add_idx, similarity))
                })
        })
        .collect();

    candidates.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));

    let (matches, _, _) = candidates.into_iter().fold(
        (
            Vec::new(),
            vec![false; delete_indices.len()],
            vec![false; add_indices.len()],
        ),
        |(mut matches, mut used_deletes, mut used_adds),
         (del_i, add_i, del_idx, add_idx, similarity)| {
            if !used_deletes[del_i] && !used_adds[add_i] && similarity > LINE_SIMILARITY_THRESHOLD {
                matches.push((del_idx, add_idx));
                used_deletes[del_i] = true;
                used_adds[add_i] = true;
            }
            (matches, used_deletes, used_adds)
        },
    );

    matches
}

fn are_lengths_similar(len1: usize, len2: usize) -> bool {
    if len1 == 0 && len2 == 0 {
        return true;
    }
    let min_len = len1.min(len2) as f64;
    let max_len = len1.max(len2) as f64;
    (min_len / max_len) >= LENGTH_SIMILARITY_RATIO
}

fn calculate_line_similarity(line1: &str, line2: &str) -> f64 {
    if line1.is_empty() && line2.is_empty() {
        return 1.0;
    }

    if line1.len() > MAX_LINE_SIMILARITY_LENGTH || line2.len() > MAX_LINE_SIMILARITY_LENGTH {
        return 0.0;
    }

    let distance = levenshtein(line1, line2);
    let max_len = std::cmp::max(line1.len(), line2.len()) as f64;

    if max_len == 0.0 {
        1.0
    } else {
        1.0 - (distance as f64 / max_len)
    }
}

/// Parse hunk header
///
/// Parses lines like: @@ -old_start,old_count +new_start,new_count @@
pub fn parse_hunk_header(line: &str) -> Option<(i32, i32)> {
    let (old_start, new_start) = line
        .split_whitespace()
        .fold((None, None), |(old, new), part| match part {
            p if p.starts_with('-') && p.len() > 1 => {
                let start = p[1..].split(',').next().and_then(|s| s.parse::<i32>().ok());
                (start.or(old), new)
            }
            p if p.starts_with('+') && p.len() > 1 => {
                let start = p[1..].split(',').next().and_then(|s| s.parse::<i32>().ok());
                (old, start.or(new))
            }
            _ => (old, new),
        });

    match (old_start, new_start) {
        (Some(old), Some(new)) => Some((old, new)),
        _ => None,
    }
}

/// Process accumulated hunk lines and convert them to DiffLines with syntax highlighting
fn process_hunk_lines(
    hunk_lines: &mut Vec<(String, LineType, (i32, i32))>,
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

    let syntax_tokens_per_line = highlight_hunk_lines(hunk_lines, display_filename);

    for (i, (content, line_type, (old_num, new_num))) in hunk_lines.iter().enumerate() {
        let syntax_tokens = syntax_tokens_per_line
            .get(i)
            .cloned()
            .unwrap_or_else(|| fallback_syntax_tokens(content));

        let line_annotations = match line_type {
            LineType::Add | LineType::Context => {
                if *new_num > 0 {
                    find_line_annotations(*new_num, &file_annotations)
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };

        diff_lines.push(DiffLine {
            line_type: *line_type,
            old_line_number: Some(*old_num),
            new_line_number: Some(*new_num),
            annotations: line_annotations,
            raw_content: content.clone(),
            syntax_tokens,
            word_changes: None, // Will be populated by apply_word_highlighting
        });
    }
    hunk_lines.clear();
}

/// Parse git diff with separation of concerns
pub fn parse_git_diff(
    git_diff: &str,
    annotations: &BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
    version_id: &str,
) -> Vec<FileDiff> {
    git_diff
        .split("diff --git")
        .filter(|file_diff| !file_diff.trim().is_empty())
        .filter_map(|file_diff| parse_single_file_diff(file_diff, annotations, version_id))
        .collect()
}

/// Parse a single file diff section
fn parse_single_file_diff(
    file_diff: &str,
    annotations: &BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
    version_id: &str,
) -> Option<FileDiff> {
    let lines: Vec<&str> = file_diff.split('\n').collect();

    let file_metadata = parse_file_metadata(&lines)?;

    // Process diff content
    let mut diff_lines = process_diff_content(
        &lines,
        &file_metadata.display_filename,
        annotations,
        version_id,
    );

    // Apply word-level highlighting
    apply_word_highlighting(&mut diff_lines);

    Some(FileDiff {
        filename_before: file_metadata.filename_before,
        filename_after: file_metadata.filename_after,
        is_binary: file_metadata.is_binary,
        lines: diff_lines,
    })
}

/// File metadata extracted from diff headers
#[derive(Debug)]
struct FileMetadata {
    filename_before: String,
    filename_after: String,
    is_binary: bool,
    display_filename: String,
}

/// Parse file metadata from diff header lines
fn parse_file_metadata(lines: &[&str]) -> Option<FileMetadata> {
    let mut filename_before = String::new();
    let mut filename_after = String::new();
    let mut is_binary = false;

    for line in lines {
        match line {
            line if line.starts_with(" a/") && line.contains(" b/") => {
                if let Some((before, after)) = line.split_once(" b/") {
                    filename_before = before.trim_start_matches(" a/").to_string();
                    filename_after = after.to_string();
                }
            }
            line if line.starts_with("+++") => {
                filename_after = line[4..].trim_start_matches("b/").to_string();
            }
            line if line.starts_with("---") => {
                filename_before = line[4..].trim_start_matches("a/").to_string();
            }
            line if line.starts_with("deleted file mode") => {
                filename_after = "/dev/null".to_string();
            }
            line if line.starts_with("new file mode") => {
                filename_before = "/dev/null".to_string();
            }
            line if line.starts_with("Binary files") => {
                is_binary = true;
            }
            _ => {}
        }
    }

    // If neither filename was found, this isn't a valid file diff
    if filename_before.is_empty() && filename_after.is_empty() {
        return None;
    }

    let display_filename = if filename_after != "/dev/null" {
        filename_after.clone()
    } else {
        filename_before.clone()
    };

    Some(FileMetadata {
        filename_before,
        filename_after,
        is_binary,
        display_filename,
    })
}

/// Process diff content lines into structured diff lines
fn process_diff_content(
    lines: &[&str],
    display_filename: &str,
    annotations: &BTreeMap<String, BTreeMap<String, Vec<Annotations>>>,
    version_id: &str,
) -> Vec<DiffLine> {
    let mut diff_lines = Vec::new();
    let mut current_hunk_lines: Vec<(String, LineType, (i32, i32))> = Vec::new();
    let mut line_numbers = LineNumbers::new();
    let mut in_hunk = false;

    for line in lines {
        if line.trim() == "\\ No newline at end of file" {
            continue;
        }

        if line.starts_with("@@") {
            process_hunk_lines(
                &mut current_hunk_lines,
                &mut diff_lines,
                display_filename,
                annotations,
                version_id,
            );

            add_hunk_separator_if_needed(&mut diff_lines, in_hunk);

            if let Some((old_start, new_start)) = parse_hunk_header(line) {
                line_numbers.reset(old_start as u32, new_start as u32);
            } else {
                line_numbers.reset(1, 1);
            }

            diff_lines.push(create_hunk_line(line));
            in_hunk = true;
        } else if in_hunk {
            process_hunk_content_line(line, &mut current_hunk_lines, &mut line_numbers);
        }
    }

    // Process final hunk
    process_hunk_lines(
        &mut current_hunk_lines,
        &mut diff_lines,
        display_filename,
        annotations,
        version_id,
    );

    diff_lines
}

/// Track line numbers for old and new versions
#[derive(Debug, Default)]
struct LineNumbers {
    old: u32,
    new: u32,
}

impl LineNumbers {
    fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self, old_start: u32, new_start: u32) {
        self.old = old_start;
        self.new = new_start;
    }
}

/// Process a single content line within a hunk
fn process_hunk_content_line(
    line: &str,
    current_hunk_lines: &mut Vec<(String, LineType, (i32, i32))>,
    line_numbers: &mut LineNumbers,
) {
    match line {
        line if line.starts_with("+") && !line.starts_with("+++") => {
            current_hunk_lines.push((
                line[1..].to_string(),
                LineType::Add,
                (-1, line_numbers.new as i32),
            ));
            line_numbers.new += 1;
        }
        line if line.starts_with("-") && !line.starts_with("---") => {
            current_hunk_lines.push((
                line[1..].to_string(),
                LineType::Delete,
                (line_numbers.old as i32, -1),
            ));
            line_numbers.old += 1;
        }
        line if is_context_line(line) => {
            let content = line.strip_prefix(" ").unwrap_or(line);
            current_hunk_lines.push((
                content.to_string(),
                LineType::Context,
                (line_numbers.old as i32, line_numbers.new as i32),
            ));
            line_numbers.old += 1;
            line_numbers.new += 1;
        }
        _ => {}
    }
}

/// Check if a line is a context line (starts with space or is other valid content)
fn is_context_line(line: &str) -> bool {
    line.starts_with(" ")
        || (!line.starts_with("diff")
            && !line.starts_with("index")
            && !line.starts_with("+++")
            && !line.starts_with("---")
            && !line.is_empty())
}

/// Add hunk separator if needed
fn add_hunk_separator_if_needed(diff_lines: &mut Vec<DiffLine>, in_hunk: bool) {
    if in_hunk {
        diff_lines.push(DiffLine {
            line_type: LineType::HunkSeparator,
            old_line_number: None,
            new_line_number: None,
            annotations: Vec::new(),
            raw_content: String::new(),
            syntax_tokens: Vec::new(),
            word_changes: None,
        });
    }
}

/// Create a hunk header line
fn create_hunk_line(line: &str) -> DiffLine {
    DiffLine {
        line_type: LineType::Hunk,
        old_line_number: None,
        new_line_number: None,
        annotations: Vec::new(),
        raw_content: line.to_string(),
        syntax_tokens: fallback_syntax_tokens(line),
        word_changes: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_similarity() {
        assert_eq!(calculate_line_similarity("hello world", "hello world"), 1.0);

        let sim1 = calculate_line_similarity(
            "[Main Page](../../../../games/OpenRCT2/info/en)",
            "[Main Page](../info/en)",
        );
        let sim2 = calculate_line_similarity(
            "[Options Page](../../../../games/OpenRCT2/player-options)",
            "[Options Page](../player-options)",
        );

        assert!(sim1 > 0.4, "sim1 = {}", sim1);
        assert!(sim2 > 0.4, "sim2 = {}", sim2);

        let sim3 = calculate_line_similarity(
            "[Main Page](../../../../games/OpenRCT2/info/en)",
            "[Options Page](../player-options)",
        );
        assert!(sim3 < 0.7, "sim3 = {}", sim3);
        assert!(sim1 > sim3, "sim1 ({}) should be > sim3 ({})", sim1, sim3);
    }

    #[test]
    fn test_huge_line_similarity_performance() {
        let huge_line1 = "x".repeat(MAX_LINE_SIMILARITY_LENGTH + 1);
        let huge_line2 = "y".repeat(MAX_LINE_SIMILARITY_LENGTH + 1);
        let normal_line = "normal line";

        assert_eq!(calculate_line_similarity(&huge_line1, &huge_line2), 0.0);
        assert_eq!(calculate_line_similarity(&huge_line1, normal_line), 0.0);
        assert_eq!(calculate_line_similarity(normal_line, &huge_line2), 0.0);

        let sim = calculate_line_similarity("hello world", "hello rust");
        assert!(
            sim > 0.0 && sim < 1.0,
            "Normal lines should have reasonable similarity: {}",
            sim
        );

        let limit_line1 = "a".repeat(MAX_LINE_SIMILARITY_LENGTH);
        let limit_line2 = "b".repeat(MAX_LINE_SIMILARITY_LENGTH);
        let limit_sim = calculate_line_similarity(&limit_line1, &limit_line2);
        assert!(
            limit_sim >= 0.0 && limit_sim <= 1.0,
            "Lines at limit should compute similarity: {}",
            limit_sim
        );
    }
}
