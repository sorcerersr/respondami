//! VCC section extractors — build the 5 summary sections from blocks.
//!
//! Each section is rendered only when non-empty:
//! - **[Session Goal]** — initial goal + scope changes.
//! - **[Files And Changes]** — Modified / Created / Read, deduped, capped.
//! - **[Commits]** — last 8 `git commit` invocations (hash + subject).
//! - **[Outstanding Context]** — blocker/failure lines from the last 20 blocks.
//! - **[User Preferences]** — preference lines, deduped against goals.
//!
//! Ported from pi-vcc's `extract/*` + `build-sections.ts`. There is no Tracked
//! Commands section (respondami YAGNI).

use std::collections::HashSet;

use super::normalize::{clip, collapse_skill_lines, non_empty_lines};
use super::normalize::VccBlock;

/// Extract a file path from tool-call arguments.
///
/// Handles the keys `path`, `file_path`, `filePath`, `file`.
#[must_use]
pub fn extract_path(args: &serde_json::Value) -> Option<String> {
    for key in ["path", "file_path", "filePath", "file"] {
        if let Some(s) = args.get(key).and_then(serde_json::Value::as_str) {
            return Some(s.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Session Goal
// ---------------------------------------------------------------------------

const MAX_GOAL_CHARS: usize = 200;
const LEADING_CHARS: usize = 200;

const SCOPE_CHANGE_RE: &[&str] = &[
    "instead",
    "actually",
    "change of plan",
    "forget that",
    "new task",
    "switch to",
    "now I want",
    "pivot",
    "let's do",
];

const TASK_RE: &[&str] = &[
    "fix",
    "implement",
    "add",
    "create",
    "build",
    "refactor",
    "debug",
    "investigate",
    "update",
    "remove",
    "delete",
    "migrate",
    "deploy",
    "test",
    "write",
    "set up",
];

const NOISE_SHORT_RE: &[&str] = &[
    "ok", "yes", "no", "sure", "yeah", "yep", "go", "hi", "hey", "thx", "thanks", "y", "n", "k",
];

/// Signals that the rest of the user message is a command template.
const TEMPLATE_SIGNAL_RE: &[&str] = &[
    "For each",
    "Do NOT implement",
    "Analyze and propose",
    "If Task/context",
    "Output:",
];

fn contains_any(lower: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| lower.contains(n))
}

fn strip_leading_bullet(line: &str) -> String {
    let t = line.trim_start();
    if let Some(r) = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))
    {
        return r.trim().to_string();
    }
    // Numbered bullet: "1. "
    if let Some(dot) = t.find('.')
        && t[..dot].chars().all(|c| c.is_ascii_digit())
    {
        return t[dot + 1..].trim().to_string();
    }
    t.to_string()
}

fn is_substantive_goal(text: &str) -> bool {
    let t = text.trim();
    if t.len() <= 5 || t.len() > MAX_GOAL_CHARS {
        return false;
    }
    let lower = t.to_lowercase();
    if NOISE_SHORT_RE.iter().any(|n| lower == *n || lower.starts_with(&format!("{n} "))) {
        return false;
    }
    // Reject clearly non-goal lines (pasted output, code, paths).
    if t.starts_with('[')
        || t.starts_with("```")
        || t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with('/')
        || t.starts_with("For each")
    {
        return false;
    }
    true
}

/// Extract session goals from user blocks.
#[must_use]
pub fn extract_goals(blocks: &[VccBlock]) -> Vec<String> {
    let mut goals: Vec<String> = Vec::new();
    let mut latest_scope_change: Option<Vec<String>> = None;

    for b in blocks {
        let VccBlock::User { text } = b else {
            continue;
        };
        let raw_lines = non_empty_lines(text);
        // Truncate at a command-template signal.
        let truncated: Vec<String> = match raw_lines.iter().find(|l| {
            let lt = l.trim_start().to_lowercase();
            TEMPLATE_SIGNAL_RE
                .iter()
                .any(|s| lt.starts_with(&s.to_lowercase()))
        }) {
            Some(sig) => raw_lines[..raw_lines.iter().position(|x| x == sig).unwrap()].to_vec(),
            None => raw_lines,
        };
        let lines = collapse_skill_lines(truncated)
            .into_iter()
            .filter(|l| is_substantive_goal(l))
            .map(|l| strip_leading_bullet(&l))
            .filter(|l| l.len() > 5)
            .collect::<Vec<_>>();
        if lines.is_empty() {
            continue;
        }

        if goals.is_empty() {
            goals.extend(lines.iter().take(6).cloned());
            continue;
        }

        let leading = text.chars().take(LEADING_CHARS).collect::<String>();
        let lower = leading.to_lowercase();
        if contains_any(&lower, SCOPE_CHANGE_RE) {
            latest_scope_change = Some(lines.iter().take(3).map(|l| clip(l, MAX_GOAL_CHARS)).collect());
        } else if contains_any(&lower, TASK_RE) && lines[0].len() > 15 {
            latest_scope_change = Some(lines.iter().take(2).map(|l| clip(l, MAX_GOAL_CHARS)).collect());
        }
    }

    if let Some(scope) = latest_scope_change
        && !scope.is_empty()
    {
        goals.push("[Scope change]".to_string());
        goals.extend(scope);
    }

    goals.into_iter().take(8).collect()
}

// ---------------------------------------------------------------------------
// Files And Changes
// ---------------------------------------------------------------------------

const FILE_READ_TOOLS: &[&str] = &["read", "read_file", "view"];
const FILE_WRITE_TOOLS: &[&str] = &[
    "edit",
    "write",
    "edit_file",
    "write_file",
    "multiedit",
    "quick_edit",
    "target_edit",
    "apply_patch",
];
const FILE_CREATE_TOOLS: &[&str] = &["write", "write_file"];

fn matches_tool(tools: &[&str], name: &str) -> bool {
    tools.iter().any(|t| t.eq_ignore_ascii_case(name))
}

/// Find the longest common directory prefix among absolute paths.
fn longest_common_dir_prefix(paths: &[String]) -> String {
    let abs: Vec<&String> = paths.iter().filter(|p| p.starts_with('/')).collect();
    if abs.len() < 2 {
        return String::new();
    }
    let split: Vec<Vec<&str>> = abs.iter().map(|p| p.split('/').collect()).collect();
    let min = split.iter().map(Vec::len).min().unwrap_or(0);
    let mut i = 0;
    while i < min.saturating_sub(1) {
        let seg = split[0][i];
        if !split.iter().all(|s| s[i] == seg) {
            break;
        }
        i += 1;
    }
    if i < 2 {
        return String::new();
    }
    format!("{}/", split[0][..i].join("/"))
}

fn trim_paths(set: &mut HashSet<String>, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    let owned: Vec<String> = set.iter().cloned().collect();
    set.clear();
    for p in owned {
        set.insert(p.strip_prefix(prefix).unwrap_or(&p).to_string());
    }
}

/// Extract file activity from tool-call blocks.
#[must_use]
pub fn extract_files(blocks: &[VccBlock]) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let (mut read, mut modified, mut created) = (HashSet::new(), HashSet::new(), HashSet::new());
    for b in blocks {
        let VccBlock::ToolCall { name, args } = b else {
            continue;
        };
        let Some(p) = extract_path(args) else {
            continue;
        };
        if matches_tool(FILE_READ_TOOLS, name) {
            read.insert(p.clone());
        }
        if matches_tool(FILE_WRITE_TOOLS, name) {
            modified.insert(p.clone());
        }
        if matches_tool(FILE_CREATE_TOOLS, name) {
            created.insert(p);
        }
    }

    let all: Vec<String> = read.iter().chain(modified.iter()).chain(created.iter()).cloned().collect();
    let prefix = longest_common_dir_prefix(&all);
    trim_paths(&mut read, &prefix);
    trim_paths(&mut modified, &prefix);
    trim_paths(&mut created, &prefix);
    // Dedup: if already Modified, drop from Created (file existed before).
    for p in modified.iter().cloned().collect::<Vec<_>>() {
        created.remove(&p);
    }
    (read, modified, created)
}

fn cap_join(set: &HashSet<String>, limit: usize) -> String {
    let mut arr: Vec<&str> = set.iter().map(String::as_str).collect();
    arr.sort_unstable();
    if arr.len() <= limit {
        arr.join(", ")
    } else {
        format!("{} (+{} more)", arr[..limit].join(", "), arr.len() - limit)
    }
}

/// Format file activity as section lines.
#[must_use]
pub fn format_file_activity(blocks: &[VccBlock]) -> Vec<String> {
    let (read, modified, created) = extract_files(blocks);
    let mut lines = Vec::new();
    if !modified.is_empty() {
        lines.push(format!("Modified: {}", cap_join(&modified, 10)));
    }
    if !created.is_empty() {
        lines.push(format!("Created: {}", cap_join(&created, 10)));
    }
    if !read.is_empty() {
        lines.push(format!("Read: {}", cap_join(&read, 10)));
    }
    lines
}

// ---------------------------------------------------------------------------
// Commits
// ---------------------------------------------------------------------------

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

/// Extract git commits from bash blocks and pair with a hash from the output.
#[must_use]
pub fn extract_commits(blocks: &[VccBlock]) -> Vec<(Option<String>, String)> {
    let mut commits: Vec<(Option<String>, String)> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let VccBlock::Bash { command, output, .. } = b else {
            continue;
        };
        if !command.contains("git commit") {
            continue;
        }
        // Extract the -m "..." subject.
        let message = extract_commit_message(command);
        let Some(message) = message else {
            continue;
        };
        if message.is_empty() {
            continue;
        }
        // Look at the output for a hash.
        let mut hash: Option<String> = None;
        for line in output.lines() {
            if let Some(h) = find_hash(line) {
                hash = Some(h);
                break;
            }
        }
        let key = format!("{}::{message}", hash.as_deref().unwrap_or_default());
        if !commits.iter().any(|(h, m)| format!("{}::{m}", h.as_deref().unwrap_or_default()) == key) {
            commits.push((hash, message));
        }
        let _ = i;
    }
    commits
}

/// Extract the `-m "subject"` from a `git commit` command.
fn extract_commit_message(cmd: &str) -> Option<String> {
    let m_pos = cmd.find("-m")?;
    let after = &cmd[m_pos + 2..];
    let after = after.trim_start();
    let (inner, _) = if let Some(rest) = after.strip_prefix('"') {
        match rest.find('"') {
            Some(end) => (rest[..end].to_string(), true),
            None => (String::new(), false),
        }
    } else if let Some(rest) = after.strip_prefix('\'') {
        match rest.find('\'') {
            Some(end) => (rest[..end].to_string(), true),
            None => (String::new(), false),
        }
    } else {
        (String::new(), false)
    };
    if inner.is_empty() {
        return None;
    }
    Some(first_line(&inner.replace("\\\"", "\"").replace("\\'", "'")))
}

/// Find a 7-12 char hex hash in a line (git output).
fn find_hash(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_hexdigit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            let len = i - start;
            if (7..=12).contains(&len) {
                return Some(line[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Format commits as section lines (most recent `limit`).
#[must_use]
pub fn format_commits(commits: &[(Option<String>, String)], limit: usize) -> Vec<String> {
    let items = if commits.len() > limit {
        &commits[commits.len() - limit..]
    } else {
        commits
    };
    items
        .iter()
        .map(|(hash, message)| match hash {
            Some(h) => format!("{h}: {message}"),
            None => message.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Outstanding Context
// ---------------------------------------------------------------------------

const BLOCKER_NEEDLES: &[&str] = &[
    "fail",
    "broken",
    "cannot",
    "can't",
    "won't work",
    "does not work",
    "doesn't work",
    "still broken",
    "still failing",
    "still wrong",
    "blocked",
    "blocker",
    "not fixed",
    "not resolved",
    "not working",
    "crash",
];

/// Extract blocker/failure lines from the last 20 blocks.
#[must_use]
pub fn extract_outstanding_context(blocks: &[VccBlock]) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let tail_start = blocks.len().saturating_sub(20);
    for b in &blocks[tail_start..] {
        let (text, is_user) = match b {
            VccBlock::Assistant { text } => (text, false),
            VccBlock::User { text } => (text, true),
            _ => continue,
        };
        for line in non_empty_lines(text) {
            let lower = line.to_lowercase();
            if !BLOCKER_NEEDLES.iter().any(|n| lower.contains(n)) {
                continue;
            }
            if line.len() < 15 {
                continue;
            }
            // Skip continuation fragments.
            if line.trim_start().starts_with('-')
                || line.trim_start().starts_with('*')
                || line.trim_start().starts_with('+')
                || line.trim_start().starts_with('>')
                || line.trim_start().starts_with('(')
            {
                continue;
            }
            // Require a sentence-like start.
            let first = line.trim_start().chars().next().unwrap_or(' ');
            if !first.is_ascii_uppercase() && first != '`' && first != '"' {
                continue;
            }
            let clipped = if is_user {
                format!("[user] {}", clip(&line, 150))
            } else {
                clip(&line, 150)
            };
            if !items.contains(&clipped) {
                items.push(clipped);
            }
            break;
        }
    }
    items.into_iter().take(5).collect()
}

// ---------------------------------------------------------------------------
// User Preferences
// ---------------------------------------------------------------------------

/// Extract preference lines from user blocks.
#[must_use]
pub fn extract_preferences(blocks: &[VccBlock]) -> Vec<String> {
    let mut prefs: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for b in blocks {
        let VccBlock::User { text } = b else {
            continue;
        };
        let mut per_block = 0;
        for line in non_empty_lines(text) {
            let trimmed = line.trim();
            if trimmed.len() < 5 || trimmed.len() > 200 {
                continue;
            }
            if trimmed.ends_with('?') || trimmed.contains("?...") {
                continue;
            }
            if !is_preference(trimmed) {
                continue;
            }
            let clipped = clip(trimmed, 200);
            let key = clipped.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);
            prefs.push(clipped);
            if per_block >= 1 {
                break;
            }
            per_block += 1;
        }
    }
    prefs.into_iter().take(10).collect()
}

fn is_preference(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("prefer")
        || lower.contains("don't want")
        || lower.contains("dont want")
        || lower.contains("always use")
        || lower.contains("always do")
        || lower.contains("always run")
        || lower.contains("always prefer")
        || lower.contains("always keep")
        || lower.contains("always make")
        || lower.contains("always format")
        || lower.contains("always write")
        || lower.contains("always add")
        || lower.contains("always set")
        || lower.contains("never use")
        || lower.contains("never do")
        || lower.contains("never run")
        || lower.contains("never push")
        || lower.contains("never commit")
        || lower.contains("never write")
        || lower.contains("never add")
        || lower.contains("please use")
        || lower.contains("please avoid")
        || lower.contains("please keep")
        || lower.contains("please make")
        || lower.contains("style:")
        || lower.contains("format:")
        || lower.contains("language:")
        || lower.contains("naming:")
}

/// Remove preferences that duplicate goals (case-insensitive, trimmed).
#[must_use]
pub fn dedup_preferences_against_goals(prefs: Vec<String>, goals: &[String]) -> Vec<String> {
    let goal_set: HashSet<String> = goals.iter().map(|g| g.trim().to_lowercase()).collect();
    prefs
        .into_iter()
        .filter(|p| !goal_set.contains(&p.trim().to_lowercase()))
        .collect()
}
