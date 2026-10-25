//! Bash command compression for the VCC brief.
//!
//! `compress_bash` semantically compresses a (possibly multi-line) bash command:
//! it drops heredoc bodies, trivial preamble lines (`set -e`, `cd`, `export`,
//! comments), `cd <path> &&` prefixes, and pipe-tail formatting, then joins the
//! remaining meaningful lines and caps the length. Ported from pi-vcc's
//! `compressBash`.

/// Maximum compressed command length.
const BASH_CAP: usize = 240;
/// Maximum first-body-line preview for a non-file-writer heredoc.
const HEREDOC_BODY_CAP: usize = 80;

/// A real heredoc opener: `<<` at a command boundary with an identifier
/// terminator.
const HEREDOC_OPEN_RE: &str = "<<";

/// Preamble/boilerplate lines that carry no durable fact on their own.
fn is_trivial_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("set -")
        || t.starts_with("set +")
        || t == "pwd"
        || t == "true"
        || t == ":"
        || t.starts_with('#')
        || t.starts_with("cd ")
        || t == "cd"
        || t.starts_with("export ")
        || t.starts_with("source ")
        || t.starts_with(". ")
}

/// Pipe-tail formatting commands to strip.
const PIPE_TAIL: &[&str] = &[
    "head", "tail", "sort", "wc", "column", "tr", "cut", "awk", "uniq", "python3",
    "node", "bun",
];

/// Strip a trailing `| <cmd> ...` pipe tail (up to 3 levels).
fn strip_pipe_tail(line: &str) -> String {
    let mut c = line.to_string();
    for _ in 0..3 {
        let mut changed = false;
        if let Some(pipe_pos) = c.rfind('|') {
            let tail = c[pipe_pos + 1..].trim();
            let head_cmd = tail.split_whitespace().next().unwrap_or("");
            if PIPE_TAIL.contains(&head_cmd) {
                c = c[..pipe_pos].trim_end().to_string();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    c.trim().to_string()
}

/// Strip a leading `cd <path> &&` prefix.
fn strip_cd_prefix(line: &str) -> String {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("cd ")
        && let Some(and_pos) = rest.find(" && ")
    {
        return rest[and_pos + 4..].trim().to_string();
    }
    t.to_string()
}

/// If `lines[i]` opens a heredoc whose terminator appears on a later line,
/// return that terminator's index; otherwise `None`.
fn heredoc_close_index(lines: &[String], i: usize) -> Option<usize> {
    let line = &lines[i];
    let open = line.find(HEREDOC_OPEN_RE)?;
    let after = &line[open + HEREDOC_OPEN_RE.len()..];
    // Skip optional `-` and quotes to get the terminator identifier.
    let mut term_chars: Vec<char> = Vec::new();
    for c in after.chars() {
        if c == '-' {
            continue;
        }
        if c == '"' || c == '\'' {
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            term_chars.push(c);
        } else if !term_chars.is_empty() {
            break;
        }
    }
    let term: String = term_chars.iter().collect();
    if term.is_empty() {
        return None;
    }
    lines[i + 1..]
        .iter()
        .enumerate()
        .find(|(_, l)| l.trim() == term)
        .map(|(off, _)| i + 1 + off)
}

/// A file-writer heredoc (`cat > f <<EOF`, tee, dd) names its target.
fn is_filewriter_heredoc(line: &str) -> bool {
    line.contains("cat") || line.contains("tee") || line.contains("dd")
}

/// Body lines that are pure boilerplate and make a poor preview.
fn is_body_noise(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("import ")
        || t.starts_with("from ")
        || t.starts_with("require(")
        || t.starts_with('#')
        || t.starts_with("//")
        || t.starts_with("\"\"\"")
        || t.starts_with("'''")
}

/// Semantic compression of a (possibly multi-line) bash command.
///
/// 1. Drop heredoc bodies (keep the opener line).
/// 2. Drop trivial preamble lines unless they are the only line.
/// 3. Strip `cd <path> &&` prefixes and pipe-tail formatting per line.
/// 4. Join the remaining meaningful lines with `; ` and cap length.
#[must_use]
pub fn compress_bash(raw: &str) -> String {
    let raw_lines: Vec<String> = raw.lines().map(str::to_string).collect();

    // Pass 1: keep heredoc opener lines, skip their bodies + terminators.
    let mut without_bodies: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw_lines.len() {
        let line = raw_lines[i].clone();
        if let Some(close) = heredoc_close_index(&raw_lines, i) {
            let mut preview = String::new();
            if !is_filewriter_heredoc(&line) {
                for t in &raw_lines[i + 1..close] {
                    let t = t.trim();
                    if !t.is_empty() && !is_body_noise(t) {
                        preview = t.to_string();
                        break;
                    }
                }
            }
            if preview.is_empty() {
                without_bodies.push(line.trim().to_string());
            } else {
                let clipped = if preview.len() > HEREDOC_BODY_CAP {
                    let end = preview.floor_char_boundary(HEREDOC_BODY_CAP.saturating_sub(1));
                    format!("{}…", &preview[..end])
                } else {
                    preview
                };
                without_bodies.push(format!("{} {clipped}", line.trim()));
            }
            i = close;
        } else {
            without_bodies.push(line.trim().to_string());
        }
        i += 1;
    }

    let lines: Vec<String> = without_bodies
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return raw.trim().to_string();
    }

    let meaningful: Vec<String> = lines
        .iter()
        .filter(|l| !is_trivial_line(l))
        .map(|l| strip_pipe_tail(&strip_cd_prefix(l)))
        .filter(|l| !l.is_empty())
        .collect();

    let chosen = if meaningful.is_empty() {
        vec![strip_pipe_tail(&strip_cd_prefix(&lines[0]))]
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        meaningful
    };

    let cmd = chosen.join("; ");
    if cmd.len() > BASH_CAP {
        let end = cmd.floor_char_boundary(BASH_CAP.saturating_sub(3));
        format!("{}...", &cmd[..end])
    } else {
        cmd
    }
}
