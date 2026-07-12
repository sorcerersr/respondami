//! File discovery for @-autocomplete.
//!
//! Walks the project directory (excluding `.git`, `node_modules`, `target`, etc.)
//! and collects file and directory entries. Respects `.gitignore`, skips binary
//! files and files > 1MB. Provides `fuzzy_match()` for query filtering.

use std::path::Path;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::commands::fuzzy_match_case_insensitive;

/// A single file or directory match in the autocomplete popup.
#[derive(Debug, Clone, PartialEq)]
pub struct FileMatch {
    pub path: String,
    pub is_directory: bool,
}

impl FileMatch {
    /// Emoji icon for the entry type.
    #[must_use]
    pub fn icon(&self) -> &'static str {
        if self.is_directory {
            "📁"
        } else {
            "📄"
        }
    }

    /// Display string: icon + space + path.
    #[must_use]
    pub fn display(&self) -> String {
        format!("{} {}", self.icon(), self.path)
    }

    /// Display string truncated from the front if it exceeds `max_width`.
    ///
    /// The icon prefix (e.g., `📄 `) is always preserved. If the path is too
    /// long, leading characters are replaced with a single ellipsis (`…`).
    #[must_use]
    pub fn display_truncated(&self, max_width: usize) -> String {
        let icon = self.icon();
        let icon_width = icon.width() + 1; // icon + space
        let path_width = self.path.width();
        let total_width = icon_width + path_width;

        if total_width <= max_width {
            return self.display();
        }

        // Budget for the path portion: max_width - icon_width
        let path_budget = max_width.saturating_sub(icon_width);
        // Need at least 1 char for ellipsis + 1 char for path
        let path_budget = path_budget.max(2);

        // Truncate from front: keep last N display-width chars
        let mut kept_width = 0;
        let mut keep_byte_idx = self.path.len();
        for ch in self.path.chars().rev() {
            let ch_width = ch.width().unwrap_or(1);
            if kept_width + ch_width > path_budget - 1 {
                // -1 for the ellipsis
                break;
            }
            kept_width += ch_width;
            keep_byte_idx -= ch.len_utf8();
        }

        format!("{} …{}", icon, &self.path[keep_byte_idx..])
    }

    /// Returns `true` if the path is a hidden file or directory.
    ///
    /// A path is hidden if any of its components starts with `.`.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.path.split('/').any(|component| component.starts_with('.'))
    }
}

/// File discovery for @-autocomplete.
#[derive(Debug)]
pub struct FileDiscovery;

impl FileDiscovery {
    /// Walk the project directory and collect file and directory entries.
    #[must_use]
    pub fn discover_entries(cwd: &Path) -> Vec<FileMatch> {
        let excluded_dirs = [".git", "node_modules", "target", ".respondami", "__pycache__"];
        let mut entries = Vec::new();

        // Build gitignore filter
        let gitignore = Self::build_gitignore(cwd);

        let walker = walkdir::WalkDir::new(cwd).into_iter().filter_entry(|e| {
            let file_name = e.file_name().to_string_lossy();
            !excluded_dirs.iter().any(|ex| file_name == *ex)
        });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let rel = match entry.path().strip_prefix(cwd) {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Skip gitignored paths
            if gitignore
                .as_ref()
                .is_some_and(|gi| gi.matched_path_or_any_parents(rel, entry.file_type().is_dir()).is_ignore())
            {
                continue;
            }

            if entry.file_type().is_dir() {
                entries.push(FileMatch {
                    path: rel.to_string_lossy().to_string(),
                    is_directory: true,
                });
            } else {
                // Skip files > 1MB
                if entry.metadata().ok().is_some_and(|m| m.len() > 1_000_000) {
                    continue;
                }
                // Skip binary files
                if Self::is_binary(entry.path()) {
                    continue;
                }
                entries.push(FileMatch {
                    path: rel.to_string_lossy().to_string(),
                    is_directory: false,
                });
            }
        }

        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    fn build_gitignore(cwd: &Path) -> Option<ignore::gitignore::Gitignore> {
        let gitignore_path = cwd.join(".gitignore");
        let Ok(content) = std::fs::read_to_string(&gitignore_path) else {
            return None;
        };

        let mut builder = ignore::gitignore::GitignoreBuilder::new(cwd);
        for line in content.lines() {
            builder.add_line(None, line).ok();
        }
        builder.build().ok()
    }

    fn is_binary(path: &std::path::Path) -> bool {
        std::fs::read(path)
            .ok()
            .is_some_and(|bytes| bytes.iter().take(512).any(|&b| b == 0))
    }

    /// Fuzzy match entries against a query.
    ///
    /// When `show_hidden` is `false`, entries with any path component
    /// starting with `.` are filtered out.
    pub fn fuzzy_match(entries: &[FileMatch], query: &str, max_results: usize, show_hidden: bool) -> Vec<FileMatch> {
        let filtered = if show_hidden {
            entries.to_vec()
        } else {
            entries.iter().filter(|e| !e.is_hidden()).cloned().collect()
        };

        if query.is_empty() {
            let mut results = filtered.iter().take(max_results).cloned().collect::<Vec<FileMatch>>();
            results.sort_by(|a, b| a.path.cmp(&b.path));
            return results;
        }

        let mut scored: Vec<(usize, FileMatch)> = Vec::new();
        for entry in filtered {
            if let Some(score) = fuzzy_match_case_insensitive(query, &entry.path) {
                scored.push((score, entry.clone()));
            }
        }
        scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        let num_scored = scored.len();
        let results: Vec<FileMatch> = scored.into_iter().take(max_results).map(|(_, e)| e).collect();
        tracing::debug!(
            query = ?query,
            entries = entries.len(),
            scored = num_scored,
            results = results.len(),
            "fuzzy_match"
        );
        results
    }
}
