use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter, Style as SyntectStyle,
};
use syntect::parsing::{ParseState, ScopeStack};

use crate::{get_syntax_set, get_theme};

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
    let mut class_parts = Vec::new();

    if style.foreground.a > 0 {
        style_parts.push(format!("color:#{:02x}{:02x}{:02x}", fg.r, fg.g, fg.b));
    }

    if style.font_style.contains(FontStyle::BOLD) {
        class_parts.push("b".to_string());
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        class_parts.push("i".to_string());
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        class_parts.push("u".to_string());
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
        Err(_) => {
            return html_escape::encode_text(content).to_string();
        }
    };

    let highlight_iter =
        HighlightIterator::new(&mut highlight_state, &ops[..], content, &highlighter);
    let mut html = String::new();
    let mut last_style = String::new();
    let mut accumulated_text = String::new();

    let style_text_pairs: Vec<(SyntectStyle, &str)> = highlight_iter.collect();

    if is_invalid_scope(&highlight_state.path) {
        return html_escape::encode_text(content).to_string();
    }

    for (style, text) in style_text_pairs {
        let css_style = syntect_style_to_css(style);

        if css_style == last_style {
            accumulated_text.push_str(&html_escape::encode_text(text));
        } else {
            if !accumulated_text.is_empty() {
                if last_style.is_empty() {
                    html.push_str(&accumulated_text);
                } else {
                    html.push_str(&format!("<span {last_style}>{accumulated_text}</span>"));
                }
            }

            last_style = css_style;
            accumulated_text = html_escape::encode_text(text).to_string();
        }
    }

    if !accumulated_text.is_empty() {
        if last_style.is_empty() {
            html.push_str(&accumulated_text);
        } else {
            html.push_str(&format!("<span {last_style}>{accumulated_text}</span>"));
        }
    }

    html
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
            Err(_) => {
                // If parsing fails, fall back to individual line parsing for this line
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
        let mut html = String::new();
        let mut last_style = String::new();
        let mut accumulated_text = String::new();

        let style_text_pairs: Vec<(SyntectStyle, &str)> = highlight_iter.collect();

        for (style, text) in style_text_pairs {
            let css_style = syntect_style_to_css(style);

            if css_style == last_style {
                // Same style as previous, accumulate text
                accumulated_text.push_str(&html_escape::encode_text(text));
            } else {
                // Different style, flush accumulated text
                if !accumulated_text.is_empty() {
                    if last_style.is_empty() {
                        html.push_str(&accumulated_text);
                    } else {
                        html.push_str(&format!("<span {last_style}>{accumulated_text}</span>"));
                    }
                }

                // Start new accumulation
                last_style = css_style;
                accumulated_text = html_escape::encode_text(text).to_string();
            }
        }

        // Flush remaining accumulated text
        if !accumulated_text.is_empty() {
            if last_style.is_empty() {
                html.push_str(&accumulated_text);
            } else {
                html.push_str(&format!("<span {last_style}>{accumulated_text}</span>"));
            }
        }

        results.push(html);
        // Check if we ended in a single-line comment scope - if so, reset state for next line
        if should_reset_parser_state(&highlight_state.path, &scope_stack_before) {
            parse_state = previous_parse_state;
            highlight_state = HighlightState::new(&highlighter, scope_stack_before);
        }
    }

    results
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
        // Reset after single-line comments (but not multi-line comments)
        scope_str.contains("comment.line")
            || scope_str == "comment"
            || scope_str.starts_with("comment.") && !scope_str.contains("block")
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

/// Check if a scope represents an error or invalid syntax using TextMate conventions
fn is_invalid_scope(scope_stack: &ScopeStack) -> bool {
    // In TextMate/Sublime Text scope conventions, invalid syntax is marked with scopes starting with "invalid."
    use syntect::parsing::SCOPE_REPO;
    let repo = SCOPE_REPO.lock().unwrap();

    scope_stack.scopes.iter().any(|scope| {
        let scope_str = repo.to_string(*scope);
        scope_str.starts_with("invalid.")
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

        // Helper function to process accumulated hunk lines
        let process_hunk = |hunk_lines: &mut Vec<(String, LineType, usize)>,
                            diff_lines: &mut Vec<DiffLine>,
                            display_filename: &str| {
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
                    });
                }
                hunk_lines.clear();
            }
        };

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

        let line_count = file_diff.lines().count();
        let is_large = file_diff.len() > 100000 || line_count > 5000;

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
                process_hunk(&mut current_hunk_lines, &mut diff_lines, display_filename);

                // Add hunk separator if this isn't the first hunk
                if in_hunk {
                    diff_lines.push(DiffLine {
                        line_type: LineType::HunkSeparator,
                        old_line_number: None,
                        new_line_number: None,
                        html_content: String::new(),
                        annotations: Vec::new(),
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
        process_hunk(&mut current_hunk_lines, &mut diff_lines, display_filename);

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
