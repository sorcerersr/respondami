//! Autocomplete key handlers for file (@) and skill (/) popups.
//!
//! Handles character input, navigation (Up/Down), Tab/Enter selection, and
//! fuzzy filtering for both file and skill autocomplete modes.

use crossterm::event::KeyEvent;

use super::app::App;
use super::editor::{cursor_backspace, FileMatch};
use super::mode::AutocompleteMode;

/// Get the query text after a trigger character.
#[must_use]
pub fn get_query_after_char(input: &str, trigger: char) -> String {
    if let Some(pos) = input.rfind(trigger) {
        input[pos + 1..].to_string()
    } else {
        String::new()
    }
}

const FILE_POPUP_PAGE_SIZE: usize = 10;
const SKILL_POPUP_PAGE_SIZE: usize = 10;

/// Clamp `scroll_offset` so the selected item is always visible within the page.
pub(crate) fn clamp_scroll(selected: usize, offset: usize, _total: usize, page_size: usize) -> usize {
    if selected >= offset + page_size {
        selected - page_size + 1
    } else if selected < offset {
        selected
    } else {
        offset
    }
}

/// Handle key events during file (@) autocomplete.
///
/// # Errors
///
/// Does not currently return errors. Return type is `Result` for consistency
/// with other autocomplete handlers.
pub fn handle_file_autocomplete(
    app: &mut App,
    key: &KeyEvent,
    matches: &[FileMatch],
    selected: &usize,
    scroll_offset: &usize,
    show_hidden: &bool,
) -> anyhow::Result<bool> {
    let selected = *selected;
    let scroll_offset = *scroll_offset;
    let show_hidden = *show_hidden;
    match key.code {
        crossterm::event::KeyCode::Tab => {
            if !matches.is_empty() {
                let path = &matches[selected.min(matches.len() - 1)].path;
                if let Some(at_pos) = app.editor.input_buffer.rfind('@') {
                    let before = &app.editor.input_buffer[..=at_pos];
                    app.editor.input_buffer = format!("{before}{path}");
                    app.editor.cursor_pos = app.editor.input_buffer.len();
                }
            }
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Enter => {
            if !matches.is_empty() {
                let path = &matches[selected.min(matches.len() - 1)].path;
                if let Some(at_pos) = app.editor.input_buffer.rfind('@') {
                    let before = &app.editor.input_buffer[..=at_pos];
                    app.editor.input_buffer = format!("{before}{path}");
                    app.editor.cursor_pos = app.editor.input_buffer.len();
                }
            }
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Esc => {
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            app.editor.input_buffer.clear();
            app.editor.cursor_pos = 0;
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Char('k') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            app.editor.input_buffer.clear();
            app.editor.cursor_pos = 0;
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        // Ctrl+. toggles hidden files visibility
        crossterm::event::KeyCode::Char('.') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            let new_show_hidden = !show_hidden;
            app.config.config.ui.file_show_hidden = new_show_hidden;
            let _ = app.save_config();
            let query = get_query_after_char(&app.editor.input_buffer, '@');
            let new_matches = app.fuzzy_match_files(&query, new_show_hidden);
            app.editor.autocomplete_mode = AutocompleteMode::File {
                matches: new_matches,
                selected: 0,
                scroll_offset: 0,
                show_hidden: new_show_hidden,
            };
            return Ok(true);
        }
        crossterm::event::KeyCode::Up => {
            let new_sel = if selected > 0 { selected - 1 } else { matches.len().saturating_sub(1) };
            let new_scroll = clamp_scroll(new_sel, scroll_offset, matches.len(), FILE_POPUP_PAGE_SIZE);
            app.editor.autocomplete_mode = AutocompleteMode::File {
                matches: matches.to_vec(),
                selected: new_sel,
                scroll_offset: new_scroll,
                show_hidden,
            };
            return Ok(true);
        }
        crossterm::event::KeyCode::Down => {
            let new_sel = if selected < matches.len().saturating_sub(1) { selected + 1 } else { 0 };
            let new_scroll = clamp_scroll(new_sel, scroll_offset, matches.len(), FILE_POPUP_PAGE_SIZE);
            app.editor.autocomplete_mode = AutocompleteMode::File {
                matches: matches.to_vec(),
                selected: new_sel,
                scroll_offset: new_scroll,
                show_hidden,
            };
            return Ok(true);
        }
        crossterm::event::KeyCode::Char(c) => {
            let char_width = c.len_utf8();
            app.editor.input_buffer.insert_str(app.editor.cursor_pos, c.encode_utf8(&mut [0; 4]));
            app.editor.cursor_pos += char_width;
            let query = get_query_after_char(&app.editor.input_buffer, '@');
            let new_matches = app.fuzzy_match_files(&query, show_hidden);
            tracing::debug!(
                char = %c,
                input_buf = ?app.editor.input_buffer,
                query = ?query,
                query_bytes = ?query.as_bytes(),
                matches = new_matches.len(),
                "autocomplete file char"
            );
            app.editor.autocomplete_mode = AutocompleteMode::File {
                matches: new_matches,
                selected: 0,
                scroll_offset: 0,
                show_hidden,
            };
            return Ok(true);
        }
        crossterm::event::KeyCode::Backspace => {
            cursor_backspace(&mut app.editor.input_buffer, &mut app.editor.cursor_pos);
            if app.editor.input_buffer.contains('@') {
                // @ still in buffer — re-filter with updated query
                let query = get_query_after_char(&app.editor.input_buffer, '@');
                let new_matches = app.fuzzy_match_files(&query, show_hidden);
                app.editor.autocomplete_mode = AutocompleteMode::File {
                    matches: new_matches,
                    selected: 0,
                    scroll_offset: 0,
                    show_hidden,
                };
            } else {
                app.editor.autocomplete_mode = AutocompleteMode::None;
            }
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

/// Fuzzy-filter skill names against a query.
#[must_use]
pub fn fuzzy_match_skills(query: &str, skills: &[crate::skills::Skill]) -> Vec<String> {
    use super::editor::fuzzy_match_case_insensitive;

    if query.is_empty() {
        return skills.iter().map(|s| s.name.clone()).collect();
    }

    let mut scored: Vec<(usize, String)> = Vec::new();
    for skill in skills {
        if let Some(score) = fuzzy_match_case_insensitive(query, &skill.name) {
            scored.push((score, skill.name.clone()));
        }
    }
    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    scored.into_iter().map(|(_, name)| name).collect()
}

/// Handle key events during skill (/) autocomplete.
///
/// # Errors
///
/// Does not currently return errors. Return type is `Result` for consistency
/// with other autocomplete handlers.
pub fn handle_skill_autocomplete(
    app: &mut App,
    key: &KeyEvent,
    matches: &[String],
    selected: &usize,
    scroll_offset: &usize,
) -> anyhow::Result<bool> {
    let selected = *selected;
    let scroll_offset = *scroll_offset;
    match key.code {
        crossterm::event::KeyCode::Tab => {
            if !matches.is_empty() {
                let name = &matches[selected.min(matches.len() - 1)];
                app.editor.input_buffer = format!("/{name}");
                app.editor.cursor_pos = app.editor.input_buffer.len();
            }
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Enter => {
            if !matches.is_empty() {
                let name = &matches[selected.min(matches.len() - 1)];
                app.editor.input_buffer = format!("/{name}");
                app.editor.cursor_pos = app.editor.input_buffer.len();
            }
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Esc => {
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            app.editor.input_buffer.clear();
            app.editor.cursor_pos = 0;
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Char('k') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            app.editor.input_buffer.clear();
            app.editor.cursor_pos = 0;
            app.editor.autocomplete_mode = AutocompleteMode::None;
            return Ok(true);
        }
        crossterm::event::KeyCode::Up => {
            let new_sel = if selected > 0 { selected - 1 } else { matches.len().saturating_sub(1) };
            let new_scroll = clamp_scroll(new_sel, scroll_offset, matches.len(), SKILL_POPUP_PAGE_SIZE);
            app.editor.autocomplete_mode = AutocompleteMode::Skill {
                matches: matches.to_vec(),
                selected: new_sel,
                scroll_offset: new_scroll,
            };
            return Ok(true);
        }
        crossterm::event::KeyCode::Down => {
            let new_sel = if selected < matches.len().saturating_sub(1) { selected + 1 } else { 0 };
            let new_scroll = clamp_scroll(new_sel, scroll_offset, matches.len(), SKILL_POPUP_PAGE_SIZE);
            app.editor.autocomplete_mode = AutocompleteMode::Skill {
                matches: matches.to_vec(),
                selected: new_sel,
                scroll_offset: new_scroll,
            };
            return Ok(true);
        }
        crossterm::event::KeyCode::Char(c) => {
            // Dismiss popup on space — user is typing their prompt
            if c == ' ' {
                let char_width = c.len_utf8();
                app.editor.input_buffer.insert_str(app.editor.cursor_pos, c.encode_utf8(&mut [0; 4]));
                app.editor.cursor_pos += char_width;
                app.editor.autocomplete_mode = AutocompleteMode::None;
                return Ok(true);
            }
            let char_width = c.len_utf8();
            app.editor.input_buffer.insert_str(app.editor.cursor_pos, c.encode_utf8(&mut [0; 4]));
            app.editor.cursor_pos += char_width;
            let query = get_query_after_char(&app.editor.input_buffer, '/');
            let new_matches = fuzzy_match_skills(&query, &app.config.skills);
            app.editor.autocomplete_mode = AutocompleteMode::Skill {
                matches: new_matches,
                selected: 0,
                scroll_offset: 0,
            };
            return Ok(true);
        }
        crossterm::event::KeyCode::Backspace => {
            cursor_backspace(&mut app.editor.input_buffer, &mut app.editor.cursor_pos);
            if app.editor.input_buffer.starts_with('/') {
                let query = get_query_after_char(&app.editor.input_buffer, '/');
                let new_matches = fuzzy_match_skills(&query, &app.config.skills);
                app.editor.autocomplete_mode = AutocompleteMode::Skill {
                    matches: new_matches,
                    selected: 0,
                    scroll_offset: 0,
                };
            } else {
                app.editor.input_buffer.clear();
                app.editor.cursor_pos = 0;
                app.editor.autocomplete_mode = AutocompleteMode::None;
            }
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}
