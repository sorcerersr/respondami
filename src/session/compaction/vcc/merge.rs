//! VCC summary merging — fold a fresh summary into a previous one.
//!
//! Each named section is merged with its own rules:
//! - **Outstanding Context** is volatile — always use the fresh value.
//! - **Files And Changes** merges by category (Modified/Created/Read), deduped.
//! - **Session Goal / Commits / User Preferences** do line-level dedup with a cap.
//!
//! The brief transcript is concatenated (previous + fresh) and capped to the
//! line budget. Ported from pi-vcc's `summarize.ts` merge logic (minus the
//! Tracked Commands section and `#N` refs).

const SEPARATOR: &str = "\n\n---\n\n";
const HEADER_NAMES: &[&str] =
    &["Session Goal", "Files And Changes", "Commits", "Outstanding Context", "User Preferences"];
const FILE_CATEGORIES: &[&str] = &["Modified", "Created", "Read"];

/// Extract a named section's body from summary text.
fn section_of(text: &str, header: &str) -> String {
    let tag = format!("[{header}]");
    let Some(start) = text.find(&tag) else {
        return String::new();
    };
    let after = &text[start + tag.len()..];
    // Find the next section header or separator.
    let mut candidates: Vec<usize> = Vec::new();
    for h in HEADER_NAMES {
        if *h == header {
            continue;
        }
        if let Some(pos) = after.find(&format!("[{h}]"))
            && pos > 0
        {
            candidates.push(pos);
        }
    }
    if let Some(pos) = after.find(SEPARATOR)
        && pos > 0
    {
        candidates.push(pos);
    }
    let end = candidates.into_iter().min();
    match end {
        Some(e) => after[..e].trim().to_string(),
        None => after.trim().to_string(),
    }
}

/// Extract the brief transcript part (everything after the last separator).
fn brief_of(text: &str) -> String {
    let Some(idx) = text.rfind(SEPARATOR) else {
        return String::new();
    };
    text[idx + SEPARATOR.len()..].trim().to_string()
}

/// Strip a cap marker like "(+N more)" or "(+N earlier)" from a line.
fn strip_cap_marker(text: &str) -> String {
    let t = text.trim_start();
    let t = t
        .strip_prefix(|c: char| c == '(')
        .and_then(|rest| rest.strip_prefix("+"))
        .and_then(|rest| {
            let num_end = rest.find(|c: char| c.is_whitespace() || c == ')')?;
            rest[num_end..].strip_prefix(')')
        });
    match t {
        Some(rest) => rest.trim().to_string(),
        None => text
            .rsplit_once(" (+")
            .map(|(head, _)| head.to_string())
            .unwrap_or_else(|| text.to_string()),
    }
}

/// Remove every `<active-skills>...</active-skills>` block from a summary.
///
/// The fresh block is always re-read and re-appended by `compute()`, so any
/// block carried over from a previous summary is stale garbage. A block whose
/// closing tag is missing (truncated by the brief cap) is stripped to the end
/// of the string. Loops until no opening tag remains.
fn strip_active_skills(text: &str) -> String {
    const OPEN: &str = "<active-skills>";
    const CLOSE: &str = "</active-skills>";
    let mut out = text.to_string();
    while let Some(start) = out.find(OPEN) {
        let after = &out[start + OPEN.len()..];
        let end = after
            .find(CLOSE)
            .map(|i| start + OPEN.len() + i + CLOSE.len())
            .unwrap_or(out.len());
        out = format!("{}{}", &out[..start], &out[end..]);
    }
    out.trim_end().to_string()
}

/// Merge a header section.
fn merge_header_section(header: &str, prev: &str, fresh: &str) -> String {
    if header == "Outstanding Context" {
        return fresh.to_string();
    }
    if prev.is_empty() {
        return fresh.to_string();
    }
    if fresh.is_empty() {
        return prev.to_string();
    }

    if header == "Files And Changes" {
        return merge_file_lines(prev, fresh);
    }

    // Session Goal, Commits, User Preferences: line-level dedup, cap.
    let is_clean = |l: &str| l.starts_with("- ") && !l.contains("<skill");
    let prev_lines: Vec<&str> = prev.lines().filter(|l| is_clean(l)).collect();
    let fresh_lines: Vec<&str> = fresh.lines().filter(|l| is_clean(l)).collect();
    let mut combined: Vec<String> = Vec::new();
    for l in prev_lines.iter().chain(fresh_lines.iter()) {
        let s = l.to_string();
        if !combined.contains(&s) {
            combined.push(s);
        }
    }
    let cap = match header {
        "Session Goal" => 8,
        "Commits" => 8,
        _ => 15,
    };
    let capped = if combined.len() > cap {
        combined[combined.len() - cap..].to_vec()
    } else {
        combined
    };
    if capped.is_empty() {
        return String::new();
    }
    format!("[{header}]\n{}", capped.join("\n"))
}

/// Merge Files And Changes by category, dedup paths.
fn merge_file_lines(prev: &str, fresh: &str) -> String {
    let mut merged: std::collections::HashMap<&str, std::collections::HashSet<String>> =
        FILE_CATEGORIES.iter().map(|c| (*c, std::collections::HashSet::new())).collect();
    for text in [prev, fresh] {
        for line in text.lines() {
            for cat in FILE_CATEGORIES {
                let prefix = format!("- {cat}: ");
                if let Some(rest) = line.strip_prefix(&prefix) {
                    let rest = strip_cap_marker(rest);
                    for p in rest.split(',') {
                        let trimmed = p.trim();
                        if !trimmed.is_empty() {
                            merged.get_mut(cat).unwrap().insert(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }
    // Dedup: if already Modified, drop from Created.
    for p in merged.get("Modified").unwrap().iter().cloned().collect::<Vec<_>>() {
        merged.get_mut("Created").unwrap().remove(&p);
    }
    let mut lines: Vec<String> = Vec::new();
    for cat in FILE_CATEGORIES {
        let set = merged.get(cat).unwrap();
        if !set.is_empty() {
            let mut items: Vec<&str> = set.iter().map(String::as_str).collect();
            items.sort_unstable();
            let item_limit = 10;
            let joined = if items.len() <= item_limit {
                items.join(", ")
            } else {
                format!("{} (+{} more)", items[..item_limit].join(", "), items.len() - item_limit)
            };
            lines.push(format!("- {cat}: {joined}"));
        }
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("[Files And Changes]\n{}", lines.join("\n"))
    }
}

/// Cap a brief to a line budget, keeping the tail.
fn cap_brief_to_budget(text: &str, max_lines: usize) -> String {
    if text.is_empty() || max_lines == 0 {
        return String::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    let kept = &lines[lines.len() - max_lines..];
    let first_header = kept.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with('[') && t.contains(']')
    });
    let clean = match first_header {
        Some(p) if p > 0 => &kept[p..],
        _ => kept,
    };
    let omitted = lines.len() - clean.len();
    format!("...({omitted} earlier lines omitted)\n\n{}", clean.join("\n"))
}

/// Merge a previous summary with a fresh one.
///
/// `brief_max_lines` caps the merged brief transcript.
#[must_use]
pub fn merge_previous(prev: &str, fresh: &str, brief_max_lines: usize) -> String {
    // No previous summary: return the fresh one as-is (header tags intact).
    if prev.trim().is_empty() {
        return fresh.to_string();
    }
    // Drop any stale skills block carried over from a previous summary — the
    // fresh one is re-appended by `compute()` after the merge.
    let prev = strip_active_skills(prev);
    // Merge header sections.
    let headers: Vec<String> = HEADER_NAMES
        .iter()
        .map(|header| {
            let fresh_sec = section_of(fresh, header);
            let prev_sec = section_of(&prev, header);
            merge_header_section(header, &prev_sec, &fresh_sec)
        })
        .filter(|s| !s.is_empty())
        .collect();

    // Merge brief transcript.
    let prev_brief = brief_of(&prev);
    let fresh_brief = brief_of(fresh);
    let merged_brief = if prev_brief.is_empty() {
        fresh_brief
    } else if fresh_brief.is_empty() {
        prev_brief
    } else {
        let fresh_lines = fresh_brief.lines().count();
        let remaining = brief_max_lines.saturating_sub(fresh_lines);
        let prev_tail = cap_brief_to_budget(&prev_brief, remaining);
        if prev_tail.is_empty() {
            fresh_brief
        } else {
            format!("{prev_tail}\n\n{fresh_brief}")
        }
    };

    let mut parts: Vec<String> = Vec::new();
    if !headers.is_empty() {
        parts.push(headers.join("\n\n"));
    }
    if !merged_brief.is_empty() {
        parts.push(merged_brief);
    }
    parts.join(SEPARATOR)
}
