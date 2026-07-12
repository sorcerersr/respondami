//! Project-scoped AGENTS.md loading and generation.
//!
//! Checks two locations in order:
//! 1. `cwd/AGENTS.md` — project root
//! 2. `cwd/.respondami/AGENTS.md` — hidden project dir
//!
//! Returns `Ok(Some(content))` if found and read, `Ok(None)` if neither exists,
//! `Err(e)` if found but unreadable.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FILENAME: &str = "AGENTS.md";

/// Prompt sent to the LLM to generate AGENTS.md.
pub const GENERATE_PROMPT: &str = r"Analyze this codebase and create/update **AGENTS.md** with project-specific instructions for AI coding agents.

AGENTS.md should include:

## Architecture
- Key technologies, frameworks, and patterns
- How major components interact

## Development
- Build, test, and run commands
- Code style and conventions
- Pre-commit checks

## Known Pitfalls
- Common bugs, gotchas, and their fixes
- Regression risks and how to avoid them

## File Map
- Table of important files with brief descriptions

## Test Files
- Table of test files with coverage notes

Rules:
- Keep it concise — agents skim, they don't read novels
- Focus on decisions and gotchas, not obvious things
- Use tables for file maps (File | Purpose)
- Include actual commands that work (verify them)
- If AGENTS.md already exists, update it with new findings while keeping existing good content
- Output ONLY the AGENTS.md content, no wrapping text";

/// Chat display message for the generation turn.
pub const GENERATE_CHAT_MESSAGE: &str = "🤖 Generate AGENTS.md for this project";

/// Load AGENTS.md content from the project.
///
/// Checks two locations in order:
/// 1. `cwd/AGENTS.md` — project root
/// 2. `cwd/.respondami/AGENTS.md` — hidden project dir
///
/// Returns:
/// - `Ok(Some((content, path)))` if found and read successfully (non-empty)
/// - `Ok(None)` if neither location has a file, or file is empty/whitespace
/// - `Err(e)` if found but unreadable (permissions, encoding, etc.)
///
/// # Errors
///
/// - I/O errors when reading the file (permissions, encoding, etc.).
pub fn load_agents_md(cwd: &Path) -> io::Result<Option<(String, PathBuf)>> {
    let candidates: Vec<PathBuf> = vec![
        cwd.join(FILENAME),
        cwd.join(".respondami").join(FILENAME),
    ];

    for path in &candidates {
        if path.exists() {
            let content = fs::read_to_string(path)?;
            if content.trim().is_empty() {
                // Empty file — treat as not found
                return Ok(None);
            }
            return Ok(Some((content, path.clone())));
        }
    }

    Ok(None)
}


