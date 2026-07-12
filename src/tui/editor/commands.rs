//! Editor commands — fuzzy matching, file references, and command palette.
//!
//! Provides `fuzzy_match_case_insensitive()` for autocomplete and palette filtering,
//! `parse_file_references()` for @-file injection, and `get_palette_commands()` for
//! the Ctrl+G command menu.

/// Simple case-insensitive fuzzy match. Returns `Some(score)` if matched.
///
/// # Panics
///
/// - If the needle string is empty (caller should check before calling).
#[must_use]
pub fn fuzzy_match_case_insensitive(needle: &str, haystack: &str) -> Option<usize> {
    let needle_lower = needle.to_lowercase();
    let haystack_lower = haystack.to_lowercase();
    let mut needle_pos = 0;
    let mut score = 0usize;
    let mut last_match_idx = usize::MAX;

    for (i, c) in haystack_lower.chars().enumerate() {
        if needle_pos < needle_lower.chars().count()
            && c == needle_lower.chars().nth(needle_pos).unwrap()
        {
            if last_match_idx != usize::MAX && last_match_idx == i.saturating_sub(1) {
                score += 1; // consecutive bonus
            }
            needle_pos += 1;
            score += 1;
            last_match_idx = i;
        }
    }

    (needle_pos == needle_lower.chars().count()).then_some(score)
}

/// Parse @-file references from the input buffer.
#[must_use]
pub fn parse_file_references(input: &str) -> Vec<String> {
    let mut refs = Vec::new();
    for part in input.split('@').skip(1) {
        if part.contains('@') {
            continue;
        }
        let path = part.split_whitespace().next().unwrap_or("");
        if !path.is_empty() && !path.starts_with('/') {
            refs.push(path.to_string());
        }
    }
    refs.truncate(8);
    refs
}

// ---------------------------------------------------------------------------
// Command Palette
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct PaletteCommand {
    /// Unique identifier used for execution.
    pub id: &'static str,
    /// Display name shown in the palette list.
    pub name: &'static str,
    /// Short description shown next to the name.
    pub description: &'static str,
}

/// Return all commands available in the palette.
#[must_use]
pub fn get_palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand { id: "new", name: "new", description: "Start a new session" },
        PaletteCommand { id: "resume", name: "resume", description: "Load an existing session" },
        PaletteCommand { id: "quit", name: "quit", description: "Exit the application" },
        PaletteCommand { id: "compact", name: "compact", description: "Manually compact session context" },
        PaletteCommand { id: "help", name: "help", description: "Show available commands" },
        PaletteCommand { id: "model", name: "model", description: "Show current model and context window" },
        PaletteCommand { id: "clear", name: "clear", description: "Clear chat display" },
        PaletteCommand { id: "init", name: "init", description: "Generate/update AGENTS.md" },
        PaletteCommand { id: "toggle_thinking", name: "toggle thinking", description: "Toggle thinking display" },
        PaletteCommand { id: "toggle_tool_output", name: "toggle tool output", description: "Toggle tool output expand" },
        PaletteCommand { id: "toggle_hook_mode", name: "toggle hook display", description: "Toggle hook display mode" },
        PaletteCommand { id: "reload_hooks", name: "reload hooks", description: "Reload hooks from disk" },
    ]
}

/// Fuzzy-filter palette commands against a query.
/// Returns matches sorted by score (highest first).
#[must_use]
pub fn fuzzy_match_palette_commands(query: &str, max_results: usize) -> Vec<(usize, PaletteCommand)> {
    if query.is_empty() {
        return get_palette_commands()
            .into_iter()
            .map(|c| (0, c))
            .take(max_results)
            .collect();
    }

    let mut scored: Vec<(usize, PaletteCommand)> = Vec::new();

    for cmd in get_palette_commands() {
        // Match against name (full score)
        if let Some(score) = fuzzy_match_case_insensitive(query, cmd.name) {
            scored.push((score, cmd));
            continue;
        }
        // Match against description (half score)
        if let Some(score) = fuzzy_match_case_insensitive(query, cmd.description) {
            scored.push((score / 2, cmd));
        }
    }

    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    scored.truncate(max_results);
    scored
}
