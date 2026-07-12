//! Agent Skills — discovery, parsing, and system-prompt injection.
//!
//! Skills are self-contained capability packages discovered from two locations:
//!
//! 1. **Global**: `~/.config/respondami/skills/`
//! 2. **Project**: `.respondami/skills/`
//!
//! Each skill is a directory containing a `SKILL.md` file with YAML frontmatter.
//! Project-level skills take priority over global skills on name collision.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Where a skill was loaded from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SkillSource {
    /// `~/.config/respondami/skills/`
    Global,
    /// `.respondami/skills/`
    Project,
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillSource::Global => write!(f, "global"),
            SkillSource::Project => write!(f, "project"),
        }
    }
}

/// A discovered and parsed skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill name (from frontmatter or directory name).
    pub name: String,
    /// Skill description (from frontmatter).
    pub description: String,
    /// License (from frontmatter, optional).
    pub license: Option<String>,
    /// Arbitrary metadata (from frontmatter, optional).
    pub metadata: HashMap<String, String>,
    /// Absolute path to the `SKILL.md` file.
    pub file_path: PathBuf,
    /// Parent directory of `SKILL.md`.
    pub base_dir: PathBuf,
    /// Where the skill was loaded from.
    pub source: SkillSource,
}

/// Diagnostics from skill loading (warnings, collisions).
#[derive(Debug, Clone)]
pub struct SkillDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiagnosticLevel {
    Warning,
    Collision,
}

impl std::fmt::Display for DiagnosticLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiagnosticLevel::Warning => write!(f, "warning"),
            DiagnosticLevel::Collision => write!(f, "collision"),
        }
    }
}

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SkillFrontmatter {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) license: Option<String>,
    #[serde(default)]
    pub(crate) metadata: Option<HashMap<String, String>>,
}

/// Parse YAML frontmatter from the beginning of a file.
///
/// Returns `Some(frontmatter)` if a valid `--- ... ---` block is found at the
/// start of the file, `None` otherwise.
pub(crate) fn parse_frontmatter(content: &str) -> Option<SkillFrontmatter> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_first = &trimmed[3..];
    let end = after_first.find("---")?;
    let yaml_block = after_first[..end].trim();
    if yaml_block.is_empty() {
        return Some(SkillFrontmatter::default());
    }
    serde_yaml::from_str(yaml_block).ok()
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a skill name. Returns warnings (not errors).
pub(crate) fn validate_name(name: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if name.is_empty() {
        warnings.push("name is empty".to_string());
        return warnings;
    }
    if name.len() > 64 {
        warnings.push(format!("name exceeds 64 characters ({} chars)", name.len()));
    }
    if name.starts_with('-') || name.ends_with('-') {
        warnings.push("name starts or ends with a hyphen".to_string());
    }
    if name.contains("--") {
        warnings.push("name contains consecutive hyphens".to_string());
    }
    for c in name.chars() {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            warnings.push(format!("name contains invalid character '{c}'"));
            break;
        }
    }
    warnings
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a single `SKILL.md` file.
pub(crate) fn parse_skill(file_path: &Path, source: SkillSource) -> Result<(Skill, Vec<SkillDiagnostic>), std::io::Error> {
    let content = fs::read_to_string(file_path)?;
    let base_dir = file_path.parent().unwrap_or(file_path).to_path_buf();

    let mut diagnostics = Vec::new();
    let absolute_path = file_path.canonicalize().unwrap_or_else(|_| file_path.to_path_buf());

    let frontmatter = parse_frontmatter(&content);

    // Determine name: frontmatter > directory name
    let dir_name = base_dir
        .file_name().map_or_else(|| "unknown".to_string(), |n| n.to_string_lossy().to_string());

    let name = match &frontmatter {
        Some(fm) => fm.name.clone().unwrap_or_else(|| dir_name.clone()),
        None => dir_name.clone(),
    };

    // Name validation warnings
    let name_warnings = validate_name(&name);
    for w in name_warnings {
        diagnostics.push(SkillDiagnostic {
            level: DiagnosticLevel::Warning,
            message: format!("name \"{name}\": {w}"),
            path: absolute_path.clone(),
        });
    }

    // Name mismatch warning: spec requires name == directory name
    if let Some(fm) = &frontmatter
        && let Some(ref fm_name) = fm.name
        && *fm_name != dir_name
    {
        diagnostics.push(SkillDiagnostic {
            level: DiagnosticLevel::Warning,
            message: format!(
                "name \"{fm_name}\" does not match directory name \"{dir_name}\""
            ),
            path: absolute_path.clone(),
        });
    }

    // Description is required
    let description = match &frontmatter {
        Some(fm) => fm.description.clone(),
        None => None,
    };

    let description = match description {
        Some(desc) if !desc.trim().is_empty() => desc.trim().to_string(),
        _ => {
            return Ok((
                Skill {
                    name,
                    description: String::new(),
                    license: None,
                    metadata: HashMap::new(),
                    file_path: absolute_path,
                    base_dir,
                    source,
                },
                diagnostics,
            ));
        }
    };

    // Check description length
    if description.len() > 1024 {
        diagnostics.push(SkillDiagnostic {
            level: DiagnosticLevel::Warning,
            message: format!("description exceeds 1024 characters ({} chars)", description.len()),
            path: absolute_path.clone(),
        });
    }

    // Extract license and metadata
    let license = match &frontmatter {
        Some(fm) => fm.license.clone(),
        None => None,
    };

    let metadata = match &frontmatter {
        Some(fm) => fm.metadata.clone().unwrap_or_default(),
        None => HashMap::new(),
    };

    Ok((
        Skill {
            name,
            description,
            license,
            metadata,
            file_path: absolute_path,
            base_dir,
            source,
        },
        diagnostics,
    ))
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Scan a directory for `SKILL.md` files.
///
/// Looks at immediate subdirectories of `dir` for a `SKILL.md` file in each.
/// Flat `SKILL.md` files directly in `dir` are also checked.
pub(crate) fn load_skills_from_dir(dir: &Path, source: SkillSource) -> Vec<(Skill, Vec<SkillDiagnostic>)> {
    let mut results = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return results, // Directory doesn't exist or can't be read
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Check if this is a subdirectory — look for SKILL.md inside it
        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.is_file() {
                match parse_skill(&skill_md, source) {
                    Ok((skill, diags)) => {
                        if skill.description.is_empty() {
                            let mut warn_diags = diags;
                            warn_diags.push(SkillDiagnostic {
                                level: DiagnosticLevel::Warning,
                                message: format!(
                                    "skipping skill \"{}\": missing or empty description",
                                    skill.name
                                ),
                                path: skill.file_path.clone(),
                            });
                            results.push((skill, warn_diags));
                        } else {
                            results.push((skill, diags));
                        }
                    }
                    Err(e) => {
                        results.push((
                            Skill {
                                name: path
                                    .file_name().map_or_else(|| "unknown".to_string(), |n| n.to_string_lossy().to_string()),
                                description: String::new(),
                                license: None,
                                metadata: HashMap::new(),
                                file_path: skill_md.clone(),
                                base_dir: path.clone(),
                                source,
                            },
                            vec![SkillDiagnostic {
                                level: DiagnosticLevel::Warning,
                                message: format!("failed to parse: {e}"),
                                path: skill_md,
                            }],
                        ));
                    }
                }
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load skills from all configured locations.
///
/// Project skills take priority over global skills on name collision.
/// Returns `(skills, diagnostics)`. Skills with empty descriptions are excluded
/// from the returned vec but diagnostics are still included.
#[must_use]
pub fn load_skills(config_dir: &Path, cwd: &Path) -> (Vec<Skill>, Vec<SkillDiagnostic>) {
    let global_dir = config_dir.join("skills");
    let project_dir = cwd.join(".respondami").join("skills");

    let global_results = load_skills_from_dir(&global_dir, SkillSource::Global);
    let project_results = load_skills_from_dir(&project_dir, SkillSource::Project);

    let mut all_diagnostics: Vec<SkillDiagnostic> = Vec::new();

    // Collect all diagnostics
    for (_, diags) in &global_results {
        all_diagnostics.extend(diags.iter().cloned());
    }
    for (_, diags) in &project_results {
        all_diagnostics.extend(diags.iter().cloned());
    }

    // Build a map: name → skill (project wins over global)
    let mut skill_map: std::collections::HashMap<String, Skill> = std::collections::HashMap::new();

    // Insert global skills first
    for (skill, _) in global_results {
        if !skill.description.is_empty() {
            skill_map.insert(skill.name.clone(), skill);
        }
    }

    // Insert project skills (overwrite global on collision)
    for (skill, _) in project_results {
        if !skill.description.is_empty() {
            if skill_map.contains_key(&skill.name) {
                all_diagnostics.push(SkillDiagnostic {
                    level: DiagnosticLevel::Collision,
                    message: format!(
                        "project skill \"{}\" overrides global skill",
                        skill.name
                    ),
                    path: skill.file_path.clone(),
                });
            }
            skill_map.insert(skill.name.clone(), skill);
        }
    }

    // Sort by name for deterministic order
    let mut skills: Vec<Skill> = skill_map.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));

    (skills, all_diagnostics)
}

/// Format skills as an XML block for the system prompt.
///
/// Returns an empty string if no skills exist.
#[must_use]
pub fn format_skills_for_prompt(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut xml = String::from(
        "\n\nThe following skills provide specialized instructions for specific tasks.\n\
         When the user asks you to use a skill, you MUST first call `activate_skill` with the skill name.\n\
         Do not follow a skill's instructions until you have activated it.\n\
         When a skill file references a relative path, resolve it against the skill\n\
         directory (parent of SKILL.md / dirname of the path) and use that absolute\n\
         path in tool commands.\n\n\
         <available_skills>",
    );

    for skill in skills {
        xml.push_str(&format!(
            "\n  <skill>\n    <name>{}</name>\n    <description>{}</description>\n    <location>{}</location>\n  </skill>",
            xml_escape(&skill.name),
            xml_escape(&skill.description),
            skill.file_path.display(),
        ));
    }

    xml.push_str("\n</available_skills>");
    xml
}

/// Escape special XML characters in a string.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}


