use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use rayon::iter::{ParallelBridge, ParallelIterator};
use similar::{ChangeTag, TextDiff};

use crate::apworld::{FileContent, FileTree};

use super::{
    apply_word_highlighting, fallback_syntax_tokens, find_line_annotations, highlight_hunk_lines,
    Annotations, DiffLine, FileDiff, LineType, TemplateAnnotation,
};

const CONTEXT_COLLAPSE_THRESHOLD: usize = 20;
const CONTEXT_VISIBLE_LINES: usize = 3;

pub fn compute_file_tree_diff(
    old_tree: &FileTree,
    new_tree: &FileTree,
    annotations: &BTreeMap<String, Vec<Annotations>>,
) -> Vec<FileDiff> {
    let all_files: BTreeSet<&PathBuf> = old_tree.keys().chain(new_tree.keys()).collect();

    let mut result: Vec<FileDiff> = all_files
        .into_iter()
        .par_bridge()
        .filter_map(|filepath| {
            let old = old_tree.get(filepath);
            let new = new_tree.get(filepath);
            let filename = filepath.to_string_lossy();

            let file_annotations: Vec<TemplateAnnotation> = annotations
                .get(filename.as_ref())
                .map(|anns| {
                    anns.iter()
                        .map(|a| TemplateAnnotation {
                            desc: a.desc.clone(),
                            line: a.line.unwrap_or(0),
                            col_start: a.col_start.unwrap_or(0),
                            col_end: a.col_end.unwrap_or(0),
                        })
                        .collect()
                })
                .unwrap_or_default();

            diff_single_file(&filename, old, new, &file_annotations)
        })
        .collect();

    result.sort_by(|a, b| a.filename_after.cmp(&b.filename_after));
    result
}

fn diff_single_file(
    filename: &str,
    old: Option<&FileContent>,
    new: Option<&FileContent>,
    annotations: &[TemplateAnnotation],
) -> Option<FileDiff> {
    let (filename_before, filename_after) = match (old, new) {
        (None, None) => return None,
        (None, Some(_)) => ("/dev/null".to_string(), filename.to_string()),
        (Some(_), None) => (filename.to_string(), "/dev/null".to_string()),
        (Some(_), Some(_)) => (filename.to_string(), filename.to_string()),
    };

    let is_binary =
        matches!(old, Some(FileContent::Binary(_))) || matches!(new, Some(FileContent::Binary(_)));

    if is_binary {
        if old == new {
            return None;
        }
        return Some(FileDiff {
            filename_before,
            filename_after,
            is_binary: true,
            lines: Vec::new(),
        });
    }

    let old_text = match old {
        Some(FileContent::Text(s)) => s.as_str(),
        _ => "",
    };
    let new_text = match new {
        Some(FileContent::Text(s)) => s.as_str(),
        _ => "",
    };

    if old_text == new_text {
        return None;
    }

    let mut lines = build_diff_lines(old_text, new_text, filename, annotations);
    apply_word_highlighting(&mut lines);
    collapse_context_regions(&mut lines);

    Some(FileDiff {
        filename_before,
        filename_after,
        is_binary: false,
        lines,
    })
}

fn build_diff_lines(
    old_text: &str,
    new_text: &str,
    filename: &str,
    annotations: &[TemplateAnnotation],
) -> Vec<DiffLine> {
    let text_diff = TextDiff::from_lines(old_text, new_text);

    let mut raw_lines: Vec<(String, LineType, (Option<i32>, Option<i32>))> = Vec::new();
    let mut old_num: i32 = 1;
    let mut new_num: i32 = 1;

    for change in text_diff.iter_all_changes() {
        let content = change.value().trim_end_matches('\n').to_string();
        match change.tag() {
            ChangeTag::Equal => {
                raw_lines.push((content, LineType::Context, (Some(old_num), Some(new_num))));
                old_num += 1;
                new_num += 1;
            }
            ChangeTag::Delete => {
                raw_lines.push((content, LineType::Delete, (Some(old_num), None)));
                old_num += 1;
            }
            ChangeTag::Insert => {
                raw_lines.push((content, LineType::Add, (None, Some(new_num))));
                new_num += 1;
            }
        }
    }

    let syntax_tokens = highlight_hunk_lines(
        &raw_lines
            .iter()
            .map(|(c, lt, _)| (c.clone(), *lt, (0, 0)))
            .collect::<Vec<_>>(),
        filename,
    );

    raw_lines
        .iter()
        .enumerate()
        .map(|(i, (content, line_type, (old_ln, new_ln)))| {
            let tokens = syntax_tokens
                .get(i)
                .cloned()
                .unwrap_or_else(|| fallback_syntax_tokens(content));

            let line_annotations = match (line_type, new_ln) {
                (LineType::Add | LineType::Context, Some(n)) if *n > 0 => {
                    find_line_annotations(*n, annotations)
                }
                _ => Vec::new(),
            };

            DiffLine {
                line_type: *line_type,
                old_line_number: *old_ln,
                new_line_number: *new_ln,
                annotations: line_annotations,
                raw_content: content.clone(),
                syntax_tokens: tokens,
                word_changes: None,
                collapsed: false,
                collapse_count: None,
            }
        })
        .collect()
}

fn collapse_context_regions(lines: &mut [DiffLine]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].line_type != LineType::Context {
            i += 1;
            continue;
        }

        let run_start = i;
        while i < lines.len() && lines[i].line_type == LineType::Context {
            i += 1;
        }
        let run_len = i - run_start;

        if run_len <= CONTEXT_COLLAPSE_THRESHOLD {
            continue;
        }

        let collapse_start = run_start + CONTEXT_VISIBLE_LINES;
        let collapse_end = i - CONTEXT_VISIBLE_LINES;

        if collapse_start >= collapse_end {
            continue;
        }

        let hidden_count = collapse_end - collapse_start;
        for line in &mut lines[collapse_start..collapse_end] {
            line.collapsed = true;
        }

        lines[collapse_start].collapse_count = Some(hidden_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> FileContent {
        FileContent::Text(s.to_string())
    }

    fn make_trees(old: &[(&str, &str)], new: &[(&str, &str)]) -> (FileTree, FileTree) {
        let old_tree: FileTree = old
            .iter()
            .map(|(k, v)| (PathBuf::from(k), text(v)))
            .collect();
        let new_tree: FileTree = new
            .iter()
            .map(|(k, v)| (PathBuf::from(k), text(v)))
            .collect();
        (old_tree, new_tree)
    }

    #[test]
    fn test_new_file() {
        let (old, new) = make_trees(&[], &[("new.py", "print('hello')\n")]);
        let diffs = compute_file_tree_diff(&old, &new, &BTreeMap::new());
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].filename_before, "/dev/null");
        assert_eq!(diffs[0].filename_after, "new.py");
        assert!(diffs[0].lines.iter().all(|l| l.line_type == LineType::Add));
    }

    #[test]
    fn test_removed_file() {
        let (old, new) = make_trees(&[("old.py", "print('bye')\n")], &[]);
        let diffs = compute_file_tree_diff(&old, &new, &BTreeMap::new());
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].filename_before, "old.py");
        assert_eq!(diffs[0].filename_after, "/dev/null");
        assert!(diffs[0]
            .lines
            .iter()
            .all(|l| l.line_type == LineType::Delete));
    }

    #[test]
    fn test_modified_file() {
        let (old, new) = make_trees(
            &[("file.py", "line1\nline2\nline3\n")],
            &[("file.py", "line1\nmodified\nline3\n")],
        );
        let diffs = compute_file_tree_diff(&old, &new, &BTreeMap::new());
        assert_eq!(diffs.len(), 1);

        let lines = &diffs[0].lines;
        assert!(lines.iter().any(|l| l.line_type == LineType::Context));
        assert!(lines.iter().any(|l| l.line_type == LineType::Delete));
        assert!(lines.iter().any(|l| l.line_type == LineType::Add));
    }

    #[test]
    fn test_identical_files_not_shown() {
        let (old, new) = make_trees(&[("same.py", "unchanged\n")], &[("same.py", "unchanged\n")]);
        let diffs = compute_file_tree_diff(&old, &new, &BTreeMap::new());
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_binary_file_unchanged() {
        let hash = [0u8; 32];
        let mut old_tree = FileTree::new();
        let mut new_tree = FileTree::new();
        old_tree.insert("img.png".into(), FileContent::Binary(hash));
        new_tree.insert("img.png".into(), FileContent::Binary(hash));

        let diffs = compute_file_tree_diff(&old_tree, &new_tree, &BTreeMap::new());
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_binary_file_changed() {
        let mut old_tree = FileTree::new();
        let mut new_tree = FileTree::new();
        old_tree.insert("img.png".into(), FileContent::Binary([0u8; 32]));
        new_tree.insert("img.png".into(), FileContent::Binary([1u8; 32]));

        let diffs = compute_file_tree_diff(&old_tree, &new_tree, &BTreeMap::new());
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_binary);
    }

    #[test]
    fn test_context_collapsing() {
        let mut old_lines = String::new();
        let mut new_lines = String::new();
        for i in 0..50 {
            old_lines.push_str(&format!("line {i}\n"));
            new_lines.push_str(&format!("line {i}\n"));
        }
        old_lines.push_str("old change\n");
        new_lines.push_str("new change\n");

        let (old, new) = make_trees(&[("big.py", &old_lines)], &[("big.py", &new_lines)]);
        let diffs = compute_file_tree_diff(&old, &new, &BTreeMap::new());
        assert_eq!(diffs.len(), 1);

        let collapsed_count = diffs[0].lines.iter().filter(|l| l.collapsed).count();
        assert!(collapsed_count > 0);

        let visible_context_before_change: Vec<_> = diffs[0]
            .lines
            .iter()
            .filter(|l| l.line_type == LineType::Context && !l.collapsed)
            .collect();
        assert!(visible_context_before_change.len() <= CONTEXT_VISIBLE_LINES * 2 + 2);
    }

    #[test]
    fn test_line_numbers() {
        let (old, new) = make_trees(&[("f.py", "a\nb\nc\n")], &[("f.py", "a\nx\nc\n")]);
        let diffs = compute_file_tree_diff(&old, &new, &BTreeMap::new());
        let lines = &diffs[0].lines;

        // First line: context "a" -> old=1, new=1
        assert_eq!(lines[0].old_line_number, Some(1));
        assert_eq!(lines[0].new_line_number, Some(1));
        assert_eq!(lines[0].line_type, LineType::Context);

        // Delete "b" -> old=2
        assert_eq!(lines[1].old_line_number, Some(2));
        assert_eq!(lines[1].line_type, LineType::Delete);

        // Add "x" -> new=2
        assert_eq!(lines[2].new_line_number, Some(2));
        assert_eq!(lines[2].line_type, LineType::Add);

        // Context "c" -> old=3, new=3
        assert_eq!(lines[3].old_line_number, Some(3));
        assert_eq!(lines[3].new_line_number, Some(3));
        assert_eq!(lines[3].line_type, LineType::Context);
    }

    #[test]
    fn test_annotations_on_new_lines() {
        let (old, new) = make_trees(&[], &[("f.py", "line1\nline2\nline3\n")]);

        let annotations = BTreeMap::from([(
            "f.py".to_string(),
            vec![Annotations {
                ty: 0,
                desc: "test annotation".into(),
                severity: 1,
                line: Some(2),
                col_start: Some(0),
                col_end: Some(5),
                extra: None,
            }],
        )]);

        let diffs = compute_file_tree_diff(&old, &new, &annotations);
        let annotated: Vec<_> = diffs[0]
            .lines
            .iter()
            .filter(|l| !l.annotations.is_empty())
            .collect();
        assert_eq!(annotated.len(), 1);
        assert_eq!(annotated[0].annotations[0].desc, "test annotation");
    }
}
