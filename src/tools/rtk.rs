//! RTK (Rewrite Tool Kit) integration — command rewriting and version management.
//!
//! Discovers the RTK binary, checks version compatibility (≥ 0.23.0), and
//! rewrites tool commands through RTK when available. State is resolved once
//! at startup and cached in `RtkState`.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Cached RTK state: resolved binary path and version check result.
///
/// Resolved once at startup. `version_ok` is `true` only when the binary
/// is found AND reports version ≥ 0.23.0.
#[derive(Debug, Clone)]
pub struct RtkState {
    pub path: Option<PathBuf>,
    pub version_ok: bool,
}

impl RtkState {
    /// Whether rewrite is available (binary found and version OK).
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.path.is_some() && self.version_ok
    }
}

/// Check if the `rtk` binary is available on the system.
///
/// Runs `which rtk` (Unix) or `where rtk` (Windows).
/// Returns the resolved path if found, `None` otherwise.
#[must_use]
pub fn resolve_rtk() -> Option<PathBuf> {
    let (cmd, args) = if cfg!(target_os = "windows") {
        ("where", ["rtk"])
    } else {
        ("which", ["rtk"])
    };

    let output = std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim().trim_matches('"').trim_matches('\'');
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    None
}

/// Parse the version from `rtk --version` output.
///
/// Returns `(major, minor, patch)` on success, `None` on failure.
#[must_use]
pub fn check_rtk_version(rtk_path: &Path) -> Option<(u32, u32, u32)> {
    let output = std::process::Command::new(rtk_path)
        .arg("--version")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Expected format: "rtk X.Y.Z" or just "X.Y.Z"
    let version_str = stdout
        .lines()
        .flat_map(|l| l.split_whitespace())
        .find(|t| t.matches('.').count() == 2)?;

    let parts: Vec<u32> = version_str.split('.').filter_map(|p| p.parse().ok()).collect();
    (parts.len() >= 3).then(|| (parts[0], parts[1], parts[2]))
}

/// Resolve RTK state: binary path + version check.
///
/// Returns `RtkState` with `version_ok = true` only when binary is found
/// and reports version ≥ 0.23.0.
#[must_use]
pub fn resolve_rtk_state() -> RtkState {
    let path = resolve_rtk();
    let version_ok = if let Some(ref p) = path {
        if let Some((major, minor, _patch)) = check_rtk_version(p) {
            (major, minor) >= (0, 23)
        } else {
            false
        }
    } else {
        false
    };
    RtkState { path, version_ok }
}

/// Rewrite a bash command using `rtk rewrite <command>`.
///
/// Returns `Some(rewritten)` if RTK produced a different command,
/// `None` if unchanged, denied, or an error occurred.
pub async fn rewrite_command(
    rtk_path: &Path,
    rtk_db_dir: &Path,
    command: &str,
) -> Option<String> {
    const TIMEOUT_MS: u64 = 3000;

    // Guard: empty or whitespace-only command
    if command.trim().is_empty() {
        return None;
    }

    // Guard: prevent infinite recursion — command already starts with "rtk"
    let trimmed = command.trim_start();
    if trimmed == "rtk" || trimmed.starts_with("rtk ") {
        return None;
    }

    // Ensure the RTK database directory exists
    if let Err(e) = std::fs::create_dir_all(rtk_db_dir) {
        tracing::warn!("Failed to create rtk db dir {}: {}", rtk_db_dir.display(), e);
        return None;
    }

    let db_path = rtk_db_dir.join("history.db");
    let db_path_str = db_path.to_string_lossy().to_string();

    let result = tokio::time::timeout(
        Duration::from_millis(TIMEOUT_MS),
        tokio::process::Command::new(rtk_path)
            .arg("rewrite")
            .arg(command)
            .env("RTK_DB_PATH", &db_path_str)
            .output(),
    )
    .await;

    let output = match result {
        Ok(Ok(o)) => o,
        Ok(Err(_)) => return None,
        Err(_) => return None, // timeout
    };

    let exit_code = output.status.code().unwrap_or(-1);

    // Exit code protocol (matches Crush hook):
    //   0 or 3 = rewrite found → allow with updated input
    //   1       = no RTK equivalent → pass through unchanged
    //   2       = deny rule matched → pass through
    if exit_code != 0 && exit_code != 3 {
        return None;
    }

    let rewritten = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if rewritten.is_empty() {
        return None;
    }

    // If RTK returned the same command, no rewrite
    if rewritten == command {
        return None;
    }

    Some(rewritten)
}
