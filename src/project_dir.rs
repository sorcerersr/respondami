//! Project data directory management.
//!
//! Ensures the per-project `.respondami/` data directory exists and contains
//! a `.gitignore` so runtime artifacts (sessions, logs, rtk, sse-debug) never
//! pollute `git status` — without touching the project root `.gitignore`.
//!
//! The generated gitignore ignores everything inside `.respondami/` except the
//! documented project-config locations (`skills/`, `hooks/`, `AGENTS.md`,
//! `config.json`), which remain committable. An existing `.gitignore` is never
//! overwritten: the file is written only if absent.

use std::fs;
use std::path::Path;

/// Directory name for per-project respondami data (relative to CWD).
const RESPONDAMI_DIR: &str = ".respondami";

/// Canonical content of the auto-generated `.respondami/.gitignore`.
///
/// `*` hides the whole directory from git (including this file); the negations
/// re-include the project-config locations so they can be committed. Each
/// location needs both `!dir/` and `!dir/**` because a directory-only negation
/// does not re-include files inside it.
pub(crate) const GITIGNORE_CONTENT: &str = "*\n!skills/\n!skills/**\n!hooks/\n!hooks/**\n!AGENTS.md\n!config.json\n";

/// Ensure `<cwd>/.respondami/` exists and contains a `.gitignore`.
///
/// Creates the directory if missing, and writes the canonical gitignore
/// content if no `.gitignore` is present. Never overwrites an existing file.
///
/// Non-fatal by design: on any I/O error a warning is logged and the app
/// continues — the gitignore is pure git hygiene, not a runtime requirement.
pub(crate) fn ensure_respondami_dir(cwd: &Path) {
    let dir = cwd.join(RESPONDAMI_DIR);

    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!("Cannot create .respondami directory: {e}");
        return;
    }

    let gitignore = dir.join(".gitignore");
    if gitignore.exists() {
        return;
    }

    if let Err(e) = fs::write(&gitignore, GITIGNORE_CONTENT) {
        tracing::warn!("Cannot write .respondami/.gitignore: {e}");
    }
}
