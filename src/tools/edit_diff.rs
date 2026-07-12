//! Edit diff algorithms — exact/fuzzy matching and line ending handling.
//!
//! Provides `apply_edits()` for multi-edit transactions, BOM stripping/restoration,
//! line ending detection and normalization, and fuzzy matching for near-miss `oldText`
//! searches. All operations preserve original file encoding characteristics.

use std::borrow::Cow;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Errors that can occur during edit operations.
#[derive(Debug, Error)]
pub enum EditError {
    #[error("oldText must not be empty in {path}")]
    EmptyOldText { path: String },

    #[error("oldText not found in {path}. The text must match exactly including all whitespace and newlines.\n\noldText was:\n{old_text_preview}\n\nFirst 20 lines of file:\n{file_preview}")]
    NotFound { path: String, old_text_preview: String, file_preview: String },

    #[error("Found {count} occurrences of the text in {path}. The text must be unique. Please provide more context (surrounding lines) to make it unique.")]
    NotUnique { path: String, count: usize },

    #[error("Two edits overlap in {path}. Merge them into one edit or target disjoint regions.")]
    Overlapping { path: String },

    #[error("Internal line count mismatch in {path}: original has {original_lines} lines, normalized has {normalized_lines} lines. This indicates a bug in the edit tool.")]
    InternalMismatch { path: String, original_lines: usize, normalized_lines: usize },

    #[error("No changes made to {path}. The replacement produced identical content.")]
    NoChange { path: String },
}

/// Result of applying edits to content.
#[derive(Debug)]
pub struct ApplyResult {
    /// Content before edits (for diff generation).
    pub base_content: String,
    /// Content after edits.
    pub new_content: String,
}

/// Result of a fuzzy text search.
#[derive(Debug)]
pub struct FuzzyMatchResult<'a> {
    /// Byte index of the match in `content_for_replacement`.
    pub index: usize,
    /// Byte length of the matched text.
    pub match_length: usize,
    /// Whether fuzzy normalization was required.
    pub used_fuzzy: bool,
    /// The content to use for replacement (original if exact, normalized if fuzzy).
    pub content_for_replacement: Cow<'a, str>,
}

/// Strip UTF-8 BOM if present. Returns `(had_bom, text_without_bom)`.
pub fn strip_bom(content: &str) -> (bool, &str) {
    if let Some(stripped) = content.strip_prefix('\u{FEFF}') {
        (true, stripped)
    } else {
        (false, content)
    }
}

/// Detect the dominant line ending in content.
/// Returns `"\r\n"` if CRLF count >= bare LF count (ties go to CRLF),
/// otherwise `"\n"`. Files with no newlines return `"\n"`.
pub fn detect_line_ending(content: &str) -> &'static str {
    let crlf_count = content.matches("\r\n").count();
    let bare_lf_count = content.matches('\n').count().saturating_sub(crlf_count);

    if crlf_count >= bare_lf_count && crlf_count > 0 {
        "\r\n"
    } else {
        "\n"
    }
}

/// Normalize all line endings to LF.
pub fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Restore line endings to the original style.
pub fn restore_line_endings(text: &str, ending: &str) -> String {
    if ending == "\r\n" {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

/// Normalize text for fuzzy matching. Applies progressive transformations:
/// - NFKC normalization
/// - Strip trailing whitespace per line
/// - Smart quotes → ASCII equivalents
/// - Unicode dashes/hyphens → ASCII hyphen
/// - Special Unicode spaces → regular space
pub fn normalize_for_fuzzy(text: &str) -> String {
    let nfkc: String = text.nfkc().collect();
    nfkc
        .split('\n')
        .map(str::trim_end)
        .collect::<Vec<&str>>()
        .join("\n")
        // Smart single quotes → '
        .replace(['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'], "'")
        // Smart double quotes → "
        .replace(['\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'], "\"")
        // Various dashes/hyphens → -
        .replace(
            [
                '\u{2010}', // hyphen
                '\u{2011}', // non-breaking hyphen
                '\u{2012}', // figure dash
                '\u{2013}', // en-dash
                '\u{2014}', // em-dash
                '\u{2015}', // horizontal bar
                '\u{2212}', // minus
            ],
            "-",
        )
        // Special spaces → regular space
        .replace(
            [
                '\u{00A0}', // NBSP
                '\u{2002}', // en space
                '\u{2003}', // em space
                '\u{2004}', // three-per-em space
                '\u{2005}', // four-per-em space
                '\u{2006}', // six-per-em space
                '\u{2007}', // figure space
                '\u{2008}', // punctuation space
                '\u{2009}', // thin space
                '\u{200A}', // hair space
                '\u{202F}', // narrow NBSP
                '\u{205F}', // medium math space
                '\u{3000}', // ideographic space
            ],
            " ",
        )
}

/// Find `old_text` in `content`, trying exact match first, then fuzzy match.
///
/// When fuzzy matching is used, the returned `content_for_replacement` is the
/// fuzzy-normalized version of the content.
pub fn fuzzy_find_text<'a>(content: &'a str, old_text: &'a str) -> Option<FuzzyMatchResult<'a>> {
    // Phase 1: exact match
    if let Some(idx) = content.find(old_text) {
        return Some(FuzzyMatchResult {
            index: idx,
            match_length: old_text.len(),
            used_fuzzy: false,
            content_for_replacement: Cow::Borrowed(content),
        });
    }

    // Phase 2: fuzzy match — work entirely in normalized space
    let fuzzy_content = normalize_for_fuzzy(content);
    let fuzzy_old_text = normalize_for_fuzzy(old_text);

    fuzzy_content.find(&fuzzy_old_text).map(|idx| FuzzyMatchResult {
        index: idx,
        match_length: fuzzy_old_text.len(),
        used_fuzzy: true,
        content_for_replacement: Cow::Owned(fuzzy_content),
    })
}

/// Count occurrences of `old_text` in `content` using fuzzy normalization.
/// Returns 0 if not found.
pub fn fuzzy_count_occurrences(content: &str, old_text: &str) -> usize {
    let fuzzy_content = normalize_for_fuzzy(content);
    let fuzzy_old_text = normalize_for_fuzzy(old_text);
    if fuzzy_old_text.is_empty() {
        return 0;
    }
    fuzzy_content.split(&fuzzy_old_text).count() - 1
}

/// Split content into lines, preserving line endings.
fn split_lines_with_endings(content: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;

    if content.is_empty() {
        return lines;
    }

    for (i, ch) in content.char_indices() {
        if ch == '\n' {
            lines.push(&content[start..=i]);
            start = i + 1;
        }
    }

    // Handle last line without newline
    if start < content.len() {
        lines.push(&content[start..]);
    }

    lines
}

/// Get byte spans for each line in content.
fn get_line_spans(content: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in split_lines_with_endings(content) {
        spans.push((offset, offset + line.len()));
        offset += line.len();
    }
    spans
}

/// Find which line range a replacement touches.
fn get_replacement_line_range(
    line_spans: &[(usize, usize)],
    match_index: usize,
    match_length: usize,
) -> Option<(usize, usize)> {
    let replacement_end = match_index + match_length;

    let mut start_line = None;
    for (i, &(span_start, span_end)) in line_spans.iter().enumerate() {
        if match_index >= span_start && match_index < span_end {
            start_line = Some(i);
            break;
        }
    }
    let start_line = start_line?;

    let mut end_line = start_line;
    while end_line < line_spans.len() && line_spans[end_line].1 < replacement_end {
        end_line += 1;
    }

    if end_line >= line_spans.len() {
        return None;
    }

    Some((start_line, end_line + 1))
}

/// Apply replacements in reverse order (to preserve offsets).
fn apply_replacements(content: &str, replacements: &[(usize, usize, &str)]) -> String {
    let mut result = content.to_string();
    for &(match_index, match_length, new_text) in replacements.iter().rev() {
        result.replace_range(match_index..match_index + match_length, new_text);
    }
    result
}

/// A group of replacements covering a contiguous line range.
struct ReplacementGroup<'a> {
    start_line: usize,
    end_line: usize,
    replacements: Vec<(usize, usize, &'a str)>,
}

/// Apply replacements matched against `base_content` to `original_content` while
/// preserving unchanged line blocks from the original.
///
/// This is used when fuzzy matching is active: the replacements are computed
/// against normalized content, but we want to keep untouched lines from the
/// original (preserving trailing whitespace, original quotes, etc.).
pub(crate) fn apply_replacements_preserving_unchanged_lines(
    original_content: &str,
    base_content: &str,
    replacements: &[(usize, usize, &str)],
    path: &str,
) -> Result<String, EditError> {
    let original_lines: Vec<&str> = split_lines_with_endings(original_content);
    let base_line_spans = get_line_spans(base_content);

    if original_lines.len() != base_line_spans.len() {
        return Err(EditError::InternalMismatch {
            path: path.to_string(),
            original_lines: original_lines.len(),
            normalized_lines: base_line_spans.len(),
        });
    }

    // Sort replacements by index
    let mut sorted_replacements: Vec<_> = replacements.to_vec();
    sorted_replacements.sort_by_key(|(idx, _, _)| *idx);

    // Group replacements by line ranges
    let mut groups: Vec<ReplacementGroup<'_>> = Vec::new();

    for (match_index, match_length, new_text) in &sorted_replacements {
        let range =
            get_replacement_line_range(&base_line_spans, *match_index, *match_length).ok_or(
                EditError::InternalMismatch {
                    path: path.to_string(),
                    original_lines: original_lines.len(),
                    normalized_lines: base_line_spans.len(),
                },
            )?;

        if let Some(last) = groups.last_mut().filter(|g| range.0 < g.end_line) {
            // Merge with previous group
            last.end_line = last.end_line.max(range.1);
            last.replacements.push((*match_index, *match_length, new_text));
            continue;
        }
        groups.push(ReplacementGroup {
            start_line: range.0,
            end_line: range.1,
            replacements: vec![(*match_index, *match_length, new_text)],
        });
    }

    let mut result = String::new();
    let mut original_line_index = 0;

    for group in &groups {
        // Copy unchanged lines before this group
        for line in &original_lines[original_line_index..group.start_line] {
            result.push_str(line);
        }

        // Apply replacements within the group's line range from base content
        let group_start_offset = base_line_spans[group.start_line].0;
        let group_end_offset = base_line_spans[group.end_line - 1].1;
        let group_base = &base_content[group_start_offset..group_end_offset];

        // Adjust replacement offsets relative to group base
        let adjusted_replacements: Vec<_> = group
            .replacements
            .iter()
            .map(|(idx, len, text)| (*idx - group_start_offset, *len, *text))
            .collect();

        result.push_str(&apply_replacements(group_base, &adjusted_replacements));
        original_line_index = group.end_line;
    }

    // Copy remaining unchanged lines
    for line in &original_lines[original_line_index..] {
        result.push_str(line);
    }

    Ok(result)
}

/// Apply one or more exact-text replacements to LF-normalized content.
///
/// All edits are matched against the same original content. Replacements are
/// then applied in reverse order so offsets remain stable. If any edit needs
/// fuzzy matching, the operation runs in fuzzy-normalized content space and then
/// overlays those line-level changes onto the original content so unchanged line
/// blocks keep their original bytes.
pub fn apply_edits(
    content: &str,
    edits: &[(String, String)],
    path: &str,
) -> Result<ApplyResult, EditError> {
    // Validate: no empty oldText
    for old_text in edits.iter().map(|(o, _)| o) {
        if old_text.is_empty() {
            return Err(EditError::EmptyOldText {
                path: path.to_string(),
            });
        }
    }

    // Find each edit (exact → fuzzy)
    let mut matches: Vec<_> = Vec::new();
    for (i, (old_text, _)) in edits.iter().enumerate() {
        if let Some(result) = fuzzy_find_text(content, old_text) { matches.push((i, result)) } else {
            // Build context: show the oldText being searched and first lines of the file
            let old_text_preview = old_text.lines().take(10).collect::<Vec<_>>().join("\n");
            let file_preview = content.lines().take(20).collect::<Vec<_>>().join("\n");
            return Err(EditError::NotFound {
                path: path.to_string(),
                old_text_preview,
                file_preview,
            });
        }
    }

    // Determine the replacement base content
    let used_fuzzy = matches.iter().any(|(_, m)| m.used_fuzzy);
    let base_content: Cow<str> = if used_fuzzy {
        // Use the normalized content from the first fuzzy match
        matches
            .iter()
            .find(|(_, m)| m.used_fuzzy)
            .map(|(_, m)| m.content_for_replacement.clone())
            .unwrap_or(Cow::Borrowed(content))
    } else {
        Cow::Borrowed(content)
    };

    // Check uniqueness for each edit in the appropriate content space
    for (old_text, _) in edits {
        let count = fuzzy_count_occurrences(&base_content, old_text);
        if count > 1 {
            return Err(EditError::NotUnique {
                path: path.to_string(),
                count,
            });
        }
    }

    // Build sorted replacements
    let mut replacements: Vec<_> = matches
        .iter()
        .map(|(i, m)| (m.index, m.match_length, edits[*i].1.as_str()))
        .collect();
    replacements.sort_by_key(|(idx, _, _)| *idx);

    // Check overlaps
    for i in 1..replacements.len() {
        let prev_end = replacements[i - 1].0 + replacements[i - 1].1;
        if replacements[i].0 < prev_end {
            return Err(EditError::Overlapping {
                path: path.to_string(),
            });
        }
    }

    // Apply replacements
    let new_content = if used_fuzzy {
        apply_replacements_preserving_unchanged_lines(content, &base_content, &replacements, path)?
    } else {
        apply_replacements(content, &replacements)
    };

    let base_str = base_content.into_owned();
    if base_str == new_content {
        return Err(EditError::NoChange {
            path: path.to_string(),
        });
    }

    Ok(ApplyResult {
        base_content: base_str,
        new_content,
    })
}

/// Generate a unified diff string with line numbers and context.
pub fn generate_diff(old_content: &str, new_content: &str, path: &str) -> String {
    use similar::TextDiff;

    let diff = TextDiff::from_lines(old_content, new_content);
    let header_a = format!("a/{path}");
    let header_b = format!("b/{path}");
    let mut unified = diff.unified_diff();
    unified.header(&header_a, &header_b).context_radius(3);

    let output = unified.to_string();

    // If diff is very small, just show summary
    if output.len() < 80 {
        format!("Edits applied to {path}")
    } else {
        output
    }
}


