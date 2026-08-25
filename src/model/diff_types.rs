use ratatui::style::Style;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

use crate::hash::Fnv1aHasher;
use crate::model::comment::LineSide;
use crate::model::thread::AnchorContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

impl FileStatus {
    pub fn as_char(&self) -> char {
        match self {
            FileStatus::Added => 'A',
            FileStatus::Modified => 'M',
            FileStatus::Deleted => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Copied => 'C',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub origin: LineOrigin,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    /// Optional syntax-highlighted spans for this line
    /// If None, use the default diff coloring
    pub highlighted_spans: Option<Vec<(Style, String)>>,
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
    /// Starting line number in the old file (from @@ header)
    #[allow(dead_code)]
    pub old_start: u32,
    /// Number of lines from the old file in this hunk
    #[allow(dead_code)]
    pub old_count: u32,
    /// Starting line number in the new file (from @@ header)
    pub new_start: u32,
    /// Number of lines from the new file in this hunk
    pub new_count: u32,
}

#[derive(Debug, Clone)]
pub struct DiffFile {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub status: FileStatus,
    pub hunks: Vec<DiffHunk>,
    pub is_binary: bool,
    pub is_too_large: bool,
    pub is_commit_message: bool,
    pub content_hash: u64,
}

impl DiffHunk {
    fn review_content_hash(&self) -> u64 {
        let mut hasher = Fnv1aHasher::new();
        write_hunk_content_hash(&mut hasher, &self.lines);
        hasher.finish()
    }
}

impl DiffFile {
    /// Capture bounded, contiguous context for an inclusive anchor range on
    /// one exact side of this displayed diff.
    pub fn anchor_context(
        &self,
        side: LineSide,
        start: u32,
        end: u32,
        window: usize,
    ) -> Option<AnchorContext> {
        if start == 0 || end < start {
            return None;
        }

        let mut side_lines = HashMap::new();
        for line in self.hunks.iter().flat_map(|hunk| &hunk.lines) {
            let line_number = match side {
                LineSide::Old => line.old_lineno,
                LineSide::New => line.new_lineno,
            };
            if let Some(line_number) = line_number {
                side_lines
                    .entry(line_number)
                    .or_insert_with(|| line.content.clone());
            }
        }

        let selected = (start..=end)
            .map(|line| side_lines.get(&line).cloned())
            .collect::<Option<Vec<_>>>()?;

        let mut before = Vec::new();
        let mut line = start;
        for _ in 0..window {
            let Some(previous) = line.checked_sub(1).filter(|line| *line > 0) else {
                break;
            };
            let Some(content) = side_lines.get(&previous) else {
                break;
            };
            before.push(content.clone());
            line = previous;
        }
        before.reverse();

        let mut after = Vec::new();
        let mut line = end;
        for _ in 0..window {
            let Some(next) = line.checked_add(1) else {
                break;
            };
            let Some(content) = side_lines.get(&next) else {
                break;
            };
            after.push(content.clone());
            line = next;
        }

        Some(AnchorContext {
            before,
            selected,
            after,
        })
    }

    /// Materialize the displayed lines for one side at their absolute line
    /// numbers. Missing hunk gaps use a sentinel that cannot match displayed
    /// content, so relocation may conservatively become stale but can never
    /// invent adjacency across unrelated hunks.
    pub fn side_content_for_relocation(&self, side: LineSide) -> Vec<String> {
        let numbered: Vec<(u32, &str)> = self
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter_map(|line| {
                let line_number = match side {
                    LineSide::Old => line.old_lineno,
                    LineSide::New => line.new_lineno,
                }?;
                Some((line_number, line.content.as_str()))
            })
            .collect();
        let Some(max_line) = numbered.iter().map(|(line, _)| *line).max() else {
            return Vec::new();
        };
        let mut sentinel = "\0tuicr-unavailable-context\0".to_string();
        while numbered.iter().any(|(_, content)| *content == sentinel) {
            sentinel.push('\0');
        }
        let mut content = vec![sentinel; max_line as usize];
        for (line, value) in numbered {
            content[line as usize - 1] = value.to_string();
        }
        content
    }

    /// Stable key for a reviewed hunk.
    ///
    /// Unique hunk content ignores hunk header line numbers so unrelated
    /// edits above a hunk do not clear its reviewed state. Repeated identical
    /// hunks fall back to a line-aware key because a pure occurrence count can
    /// move reviewed state onto a different hunk when one duplicate changes.
    pub fn hunk_review_key(&self, hunk_idx: usize) -> Option<String> {
        let hash_counts = self.hunk_content_hash_counts();
        self.hunks
            .get(hunk_idx)
            .map(|hunk| self.hunk_review_key_with_counts(hunk, &hash_counts))
    }

    pub fn hunk_review_keys(&self) -> Vec<String> {
        let hash_counts = self.hunk_content_hash_counts();
        self.hunks
            .iter()
            .map(|hunk| self.hunk_review_key_with_counts(hunk, &hash_counts))
            .collect()
    }

    fn hunk_content_hash_counts(&self) -> HashMap<u64, usize> {
        let mut hash_counts = HashMap::new();
        for hunk in &self.hunks {
            *hash_counts.entry(hunk.review_content_hash()).or_insert(0) += 1;
        }
        hash_counts
    }

    fn hunk_review_key_with_counts(
        &self,
        hunk: &DiffHunk,
        hash_counts: &HashMap<u64, usize>,
    ) -> String {
        let hash = hunk.review_content_hash();
        if hash_counts.get(&hash).copied().unwrap_or_default() > 1 {
            format_hunk_review_span_key(hunk, hash)
        } else {
            format_hunk_review_content_key(hash)
        }
    }

    /// Computes a hash of the diff content (all hunk line contents) for change detection.
    pub fn compute_content_hash(hunks: &[DiffHunk]) -> u64 {
        let mut hasher = Fnv1aHasher::new();
        for hunk in hunks {
            write_hunk_content_hash(&mut hasher, &hunk.lines);
        }
        hasher.finish()
    }

    /// Highest line number reachable from hunk headers (old or new side).
    /// Used to size the gutter; expanded context beyond hunks is covered
    /// separately via `file_line_count_cache`.
    pub fn max_lineno(&self) -> u32 {
        self.hunks
            .iter()
            .map(|h| (h.old_start + h.old_count).max(h.new_start + h.new_count))
            .max()
            .unwrap_or(0)
    }

    pub fn display_path(&self) -> &PathBuf {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .expect("DiffFile must have at least one path")
    }

    /// First line number in display order that carries a value on `side`.
    ///
    /// On `LineSide::New`, returns the first context or addition line; on
    /// `LineSide::Old`, the first deletion line. Used by the submission
    /// mapper to anchor file-level comments per the spec (a file-level
    /// comment posts on the first valid visible line on the right side, or
    /// the first deleted line for pure-deletion files).
    ///
    /// Returns `None` for binary, too-large, or empty-hunk files, and for
    /// the requested side when the file has no lines on that side (e.g. a
    /// pure addition has no Old-side anchor).
    pub fn first_valid_line(&self, side: LineSide) -> Option<u32> {
        if self.is_binary || self.is_too_large {
            return None;
        }
        for hunk in &self.hunks {
            for line in &hunk.lines {
                let candidate = match side {
                    LineSide::New => match line.origin {
                        LineOrigin::Context | LineOrigin::Addition => line.new_lineno,
                        LineOrigin::Deletion => None,
                    },
                    LineSide::Old => match line.origin {
                        LineOrigin::Deletion => line.old_lineno,
                        _ => None,
                    },
                };
                if let Some(n) = candidate {
                    return Some(n);
                }
            }
        }
        None
    }

    /// Returns `(additions, deletions)` for this file.
    pub fn stat(&self) -> (usize, usize) {
        let mut additions = 0;
        let mut deletions = 0;
        for hunk in &self.hunks {
            for line in &hunk.lines {
                match line.origin {
                    LineOrigin::Addition => additions += 1,
                    LineOrigin::Deletion => deletions += 1,
                    LineOrigin::Context => {}
                }
            }
        }
        (additions, deletions)
    }
}

fn format_hunk_review_content_key(hash: u64) -> String {
    format!("hunk-content-v1:{hash:016x}:0")
}

fn format_hunk_review_span_key(hunk: &DiffHunk, hash: u64) -> String {
    format!(
        "hunk-span-v1:{hash:016x}:{}:{}:{}:{}",
        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
    )
}

fn write_hunk_content_hash(hasher: &mut Fnv1aHasher, lines: &[DiffLine]) {
    for line in lines {
        hasher.write(match line.origin {
            LineOrigin::Addition => b"+",
            LineOrigin::Deletion => b"-",
            LineOrigin::Context => b" ",
        });
        hasher.write(line.content.as_bytes());
        hasher.write(b"\n");
    }
}

#[cfg(test)]
mod anchor_context_tests {
    use super::*;

    fn line(
        origin: LineOrigin,
        content: &str,
        old_lineno: Option<u32>,
        new_lineno: Option<u32>,
    ) -> DiffLine {
        DiffLine {
            origin,
            content: content.to_string(),
            old_lineno,
            new_lineno,
            highlighted_spans: None,
        }
    }

    fn changed_file() -> DiffFile {
        let lines = vec![
            line(LineOrigin::Context, "before", Some(9), Some(9)),
            line(LineOrigin::Deletion, "old selected", Some(10), None),
            line(LineOrigin::Addition, "new selected", None, Some(10)),
            line(LineOrigin::Context, "after", Some(11), Some(11)),
            line(LineOrigin::Context, "range end", Some(12), Some(12)),
        ];
        let hunks = vec![DiffHunk {
            header: "@@ -9,4 +9,4 @@".to_string(),
            lines,
            old_start: 9,
            old_count: 4,
            new_start: 9,
            new_count: 4,
        }];
        DiffFile {
            old_path: Some(PathBuf::from("old.rs")),
            new_path: Some(PathBuf::from("new.rs")),
            status: FileStatus::Modified,
            content_hash: DiffFile::compute_content_hash(&hunks),
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
        }
    }

    #[test]
    fn should_capture_context_from_the_exact_requested_side() {
        let file = changed_file();

        let old = file.anchor_context(LineSide::Old, 10, 10, 2).unwrap();
        let new = file.anchor_context(LineSide::New, 10, 10, 2).unwrap();

        assert_eq!(old.selected, vec!["old selected"]);
        assert_eq!(new.selected, vec!["new selected"]);
        assert_eq!(old.before, vec!["before"]);
        assert_eq!(new.after, vec!["after", "range end"]);
    }

    #[test]
    fn should_capture_complete_contiguous_range_or_return_none() {
        let file = changed_file();

        let context = file.anchor_context(LineSide::New, 10, 12, 2).unwrap();

        assert_eq!(context.selected, vec!["new selected", "after", "range end"]);
        assert!(file.anchor_context(LineSide::New, 10, 13, 2).is_none());
    }

    #[test]
    fn should_preserve_absolute_side_lines_without_joining_hunk_gaps() {
        let file = changed_file();

        let old = file.side_content_for_relocation(LineSide::Old);
        let new = file.side_content_for_relocation(LineSide::New);

        assert_eq!(old[9], "old selected");
        assert_eq!(new[9], "new selected");
        assert_ne!(old[7], "before");
        assert_ne!(new[7], "before");
    }
}
