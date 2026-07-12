//! Hook loading from directory-based scripts.
//!
//! Hooks are shell scripts discovered from two locations:
//! 1. **Global**: `~/.config/respondami/hooks/`
//! 2. **Project**: `.respondami/hooks/`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// The five hook events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Stop,
    PreCompact,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookEvent::UserPromptSubmit => write!(f, "UserPromptSubmit"),
            HookEvent::PreToolUse => write!(f, "PreToolUse"),
            HookEvent::PostToolUse => write!(f, "PostToolUse"),
            HookEvent::Stop => write!(f, "Stop"),
            HookEvent::PreCompact => write!(f, "PreCompact"),
        }
    }
}

impl HookEvent {
    /// Directory name for directory-based hooks (e.g., "`PreToolUse`").
    #[must_use]
    pub fn as_dir_name(&self) -> &'static str {
        match self {
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::Stop => "Stop",
            HookEvent::PreCompact => "PreCompact",
        }
    }

    /// All possible hook events.
    #[must_use]
    pub fn all() -> &'static [HookEvent] {
        &[
            HookEvent::UserPromptSubmit,
            HookEvent::PreToolUse,
            HookEvent::PostToolUse,
            HookEvent::Stop,
            HookEvent::PreCompact,
        ]
    }
}

/// Where a hook was loaded from.
#[derive(Debug, Clone, PartialEq)]
pub enum HookSource {
    /// From `~/.config/respondami/hooks/`.
    Global,
    /// From `.respondami/hooks/`.
    Project,
}

impl std::fmt::Display for HookSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookSource::Global => write!(f, "global"),
            HookSource::Project => write!(f, "project"),
        }
    }
}

/// A hook definition.
#[derive(Debug, Clone)]
pub struct Hook {
    /// The event this hook fires on.
    pub event: HookEvent,
    /// The hook name (script filename).
    pub name: String,
    /// The shell command to execute.
    pub command: String,
    /// Where this hook was loaded from.
    pub source: HookSource,
}

/// Registry of all active hooks, indexed by event.
#[derive(Debug, Clone, Default)]
pub struct HookRegistry {
    /// Hooks grouped by event type.
    hooks: HashMap<HookEvent, Vec<Hook>>,
}

impl HookRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    /// Get hooks for a given event.
    #[must_use]
    pub fn hooks(&self, event: HookEvent) -> &[Hook] {
        self.hooks.get(&event).map(std::vec::Vec::as_slice).unwrap_or(&[])
    }

    /// Add a hook to the registry.
    pub fn add(&mut self, hook: Hook) {
        self.hooks.entry(hook.event).or_default().push(hook);
    }

    /// Check if any hooks are registered for a given event.
    #[must_use]
    pub fn has_hooks(&self, event: HookEvent) -> bool {
        self.hooks.get(&event).is_some_and(|v| !v.is_empty())
    }

    /// Total number of hooks across all events.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.hooks.values().map(std::vec::Vec::len).sum()
    }
}

// ---------------------------------------------------------------------------
// Directory-based hook loading
// ---------------------------------------------------------------------------

/// Load hooks from a directory structure like:
/// ```text
/// hooks/
/// ├── UserPromptSubmit/
/// │   └── inject-plan.sh
/// ├── PreToolUse/
/// │   └── security-check.sh
/// └── ...
/// ```
pub fn load_directory_hooks(base_dir: &Path, source: HookSource) -> Vec<Hook> {
    let hooks_dir = base_dir.join("hooks");
    if !hooks_dir.is_dir() {
        return Vec::new();
    }

    let mut hooks = Vec::new();

    // Iterate over each event directory
    for event in HookEvent::all() {
        let event_dir = hooks_dir.join(event.as_dir_name());
        if !event_dir.is_dir() {
            continue;
        }

        // List all executable files in the event directory
        let entries = match fs::read_dir(&event_dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    "Failed to read hook directory {}: {}",
                    event_dir.display(),
                    e
                );
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Check if file is executable (Unix) or has a supported extension
            let is_executable = is_executable_file(&path);
            if !is_executable {
                continue;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }

            hooks.push(Hook {
                event: *event,
                name: name.clone(),
                command: format!("bash {}", path.display()),
                source: source.clone(),
            });
        }
    }

    // Sort hooks within each event by name for deterministic order
    hooks.sort_by(|a, b| a.name.cmp(&b.name));

    hooks
}

/// Check if a file is executable (Unix permission or supported extension).
fn is_executable_file(path: &Path) -> bool {
    // Check file extension for common shell scripts
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "sh" || ext == "bash" || ext == "zsh" || ext == "ps1" {
        return true;
    }

    // Check Unix executable permission
    #[cfg(unix)]
    {
        if let Ok(metadata) = std::fs::metadata(path) {
            use std::os::unix::fs::PermissionsExt;
            return metadata.permissions().mode() & 0o111 != 0;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Combined loading
// ---------------------------------------------------------------------------

/// Load all hooks from directory-based sources.
///
/// Order of priority: project hooks > global hooks.
#[must_use]
pub fn load_all_hooks(config_dir: &Path, cwd: &Path) -> HookRegistry {
    let mut registry = HookRegistry::new();

    // 1. Load project hooks
    let project_dir = cwd.join(".respondami");
    let project_hooks = load_directory_hooks(&project_dir, HookSource::Project);
    for hook in project_hooks {
        registry.add(hook);
    }

    // 2. Load global hooks
    let global_hooks = load_directory_hooks(config_dir, HookSource::Global);
    for hook in global_hooks {
        registry.add(hook);
    }

    registry
}
