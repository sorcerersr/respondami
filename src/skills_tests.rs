use tempfile::TempDir;
use std::path::PathBuf;
use std::collections::HashMap;
use std::fs;

use crate::skills::*;

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_frontmatter_valid() {
    let content = "---\nname: pdf\ndescription: Handle PDF files\nlicense: MIT\n---\n# Body\nContent here";
    let fm = parse_frontmatter(content).expect("should parse");
    assert_eq!(fm.name, Some("pdf".to_string()));
    assert_eq!(fm.description, Some("Handle PDF files".to_string()));
    assert_eq!(fm.license, Some("MIT".to_string()));
}

#[test]
fn parse_frontmatter_missing_fields() {
    let content = "---\nname: minimal\n---\nBody";
    let fm = parse_frontmatter(content).expect("should parse");
    assert_eq!(fm.name, Some("minimal".to_string()));
    assert_eq!(fm.description, None);
    assert_eq!(fm.license, None);
}

#[test]
fn parse_frontmatter_with_metadata() {
    let content = "---\nname: test\ndescription: has metadata\nmetadata:\n  author: me\n  version: \"1.0\"\n---\nBody";
    let fm = parse_frontmatter(content).expect("should parse");
    assert_eq!(fm.name, Some("test".to_string()));
    let meta = fm.metadata.as_ref().expect("should have metadata");
    assert_eq!(meta.get("author"), Some(&"me".to_string()));
    assert_eq!(meta.get("version"), Some(&"1.0".to_string()));
}

#[test]
fn parse_frontmatter_unknown_fields_ignored() {
    let content = "---\nname: test\nunknown_field: value\nanother: 123\ndescription: works\n---\nBody";
    let fm = parse_frontmatter(content).expect("should parse");
    assert_eq!(fm.name, Some("test".to_string()));
    assert_eq!(fm.description, Some("works".to_string()));
}

#[test]
fn parse_frontmatter_no_frontmatter() {
    let content = "# No frontmatter\nJust body text";
    assert!(parse_frontmatter(content).is_none());
}

#[test]
fn parse_frontmatter_empty_block() {
    let content = "---\n---\nBody";
    let fm = parse_frontmatter(content).expect("should parse empty block");
    assert_eq!(fm.name, None);
    assert_eq!(fm.description, None);
}

// ---------------------------------------------------------------------------
// Name validation
// ---------------------------------------------------------------------------

#[test]
fn validate_name_valid() {
    assert!(validate_name("my-skill").is_empty());
    assert!(validate_name("pdf").is_empty());
    assert!(validate_name("skill-123").is_empty());
    assert!(validate_name("a").is_empty());
}

#[test]
fn validate_name_too_long() {
    let long = "a".repeat(65);
    let warnings = validate_name(&long);
    assert!(!warnings.is_empty());
}

#[test]
fn validate_name_leading_hyphen() {
    let warnings = validate_name("-bad");
    assert!(!warnings.is_empty());
}

#[test]
fn validate_name_trailing_hyphen() {
    let warnings = validate_name("bad-");
    assert!(!warnings.is_empty());
}

#[test]
fn validate_name_consecutive_hyphens() {
    let warnings = validate_name("bad--name");
    assert!(!warnings.is_empty());
}

#[test]
fn validate_name_invalid_characters() {
    let warnings = validate_name("bad_name");
    assert!(!warnings.is_empty());
    let warnings = validate_name("BadName");
    assert!(!warnings.is_empty());
}

// ---------------------------------------------------------------------------
// Description validation
// ---------------------------------------------------------------------------

#[test]
fn parse_skill_missing_description_returns_empty() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("no-desc");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "---\nname: no-desc\n---\nNo description provided.").unwrap();

    let result = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert!(result.0.description.is_empty());
}

#[test]
fn parse_skill_valid_description() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("valid");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: valid\ndescription: This is a valid skill\n---\nBody.",
    )
    .unwrap();

    let (skill, _) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(skill.description, "This is a valid skill");
    assert_eq!(skill.name, "valid");
}

// ---------------------------------------------------------------------------
// Directory scanning
// ---------------------------------------------------------------------------

#[test]
fn load_skills_from_dir_empty() {
    let dir = TempDir::new().unwrap();
    let results = load_skills_from_dir(dir.path(), SkillSource::Global);
    assert!(results.is_empty());
}

#[test]
fn load_skills_from_dir_finds_skills() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("test-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\ndescription: A test skill\n---\nBody.",
    )
    .unwrap();

    let results = load_skills_from_dir(dir.path(), SkillSource::Global);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.name, "test-skill");
}

#[test]
fn load_skills_from_dir_skips_nested_skill_md() {
    let dir = TempDir::new().unwrap();
    // Skill in immediate subdirectory (should be found)
    let top_skill = dir.path().join("top-skill");
    fs::create_dir(&top_skill).unwrap();
    fs::write(
        top_skill.join("SKILL.md"),
        "---\nname: top\ndescription: Top level\n---\nBody.",
    )
    .unwrap();
    // SKILL.md nested deeper (should NOT be found)
    let nested = dir.path().join("sub").join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        nested.join("SKILL.md"),
        "---\nname: nested\ndescription: Nested\n---\nBody.",
    )
    .unwrap();

    let results = load_skills_from_dir(dir.path(), SkillSource::Global);
    // Only finds SKILL.md in immediate subdirectories
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.name, "top");
}

// ---------------------------------------------------------------------------
// Collision resolution
// ---------------------------------------------------------------------------

#[test]
fn load_skills_project_wins_over_global() {
    let config_dir = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();

    // Global skill
    let global_skills = config_dir.path().join("skills").join("pdf");
    fs::create_dir_all(&global_skills).unwrap();
    fs::write(
        global_skills.join("SKILL.md"),
        "---\nname: pdf\ndescription: Global PDF skill\n---\nBody.",
    )
    .unwrap();

    // Project skill with same name
    let project_skills = cwd.path().join(".respondami").join("skills").join("pdf");
    fs::create_dir_all(&project_skills).unwrap();
    fs::write(
        project_skills.join("SKILL.md"),
        "---\nname: pdf\ndescription: Project PDF skill\n---\nBody.",
    )
    .unwrap();

    let (skills, diagnostics) = load_skills(config_dir.path(), cwd.path());

    // Should have exactly one "pdf" skill
    let pdf_skills: Vec<&Skill> = skills.iter().filter(|s| s.name == "pdf").collect();
    assert_eq!(pdf_skills.len(), 1);
    // Project skill wins
    assert_eq!(pdf_skills[0].description, "Project PDF skill");
    assert_eq!(pdf_skills[0].source, SkillSource::Project);

    // Collision diagnostic
    let collisions: Vec<&SkillDiagnostic> = diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Collision)
        .collect();
    assert_eq!(collisions.len(), 1);
}

#[test]
fn load_skills_no_collision() {
    let config_dir = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();

    // Different names
    let global_skills = config_dir.path().join("skills").join("global-only");
    fs::create_dir_all(&global_skills).unwrap();
    fs::write(
        global_skills.join("SKILL.md"),
        "---\nname: global-only\ndescription: Only global\n---\nBody.",
    )
    .unwrap();

    let project_skills = cwd.path().join(".respondami").join("skills").join("project-only");
    fs::create_dir_all(&project_skills).unwrap();
    fs::write(
        project_skills.join("SKILL.md"),
        "---\nname: project-only\ndescription: Only project\n---\nBody.",
    )
    .unwrap();

    let (skills, diagnostics) = load_skills(config_dir.path(), cwd.path());
    assert_eq!(skills.len(), 2);
    let collisions: Vec<&SkillDiagnostic> = diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Collision)
        .collect();
    assert!(collisions.is_empty());
}

// ---------------------------------------------------------------------------
// XML formatting
// ---------------------------------------------------------------------------

#[test]
fn format_skills_for_prompt_empty() {
    assert!(format_skills_for_prompt(&[]).is_empty());
}

#[test]
fn format_skills_for_prompt_single() {
    let skills = vec![Skill {
        name: "pdf".to_string(),
        description: "Handle PDF files".to_string(),
        license: None,
        metadata: HashMap::new(),
        file_path: PathBuf::from("/home/user/.config/respondami/skills/pdf/SKILL.md"),
        base_dir: PathBuf::from("/home/user/.config/respondami/skills/pdf"),
        source: SkillSource::Global,
    }];

    let xml = format_skills_for_prompt(&skills);
    assert!(xml.contains("<available_skills>"));
    assert!(xml.contains("<name>pdf</name>"));
    assert!(xml.contains("<description>Handle PDF files</description>"));
    assert!(xml.contains("</available_skills>"));
}

#[test]
fn format_skills_for_prompt_escapes_xml() {
    let skills = vec![Skill {
        name: "test".to_string(),
        description: "A <b>bold</b> & \"special\" description".to_string(),
        license: None,
        metadata: HashMap::new(),
        file_path: PathBuf::from("/skills/test/SKILL.md"),
        base_dir: PathBuf::from("/skills/test"),
        source: SkillSource::Global,
    }];

    let xml = format_skills_for_prompt(&skills);
    assert!(xml.contains("&lt;b&gt;"));
    assert!(xml.contains("&amp;"));
    assert!(xml.contains("&quot;special&quot;"));
}

// ---------------------------------------------------------------------------
// Name fallback to directory name
// ---------------------------------------------------------------------------

#[test]
fn parse_skill_name_falls_back_to_dir_name() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("fallback-name");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\ndescription: Uses dir name\n---\nBody.",
    )
    .unwrap();

    let (skill, _) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(skill.name, "fallback-name");
}

// ---------------------------------------------------------------------------
// Name mismatch diagnostic
// ---------------------------------------------------------------------------

#[test]
fn parse_skill_name_mismatch_warning() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("my-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: different-name\ndescription: Name differs from dir\n---\nBody.",
    )
    .unwrap();

    let (skill, diagnostics) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(skill.name, "different-name");
    let warnings: Vec<&SkillDiagnostic> = diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Warning && d.message.contains("does not match directory name"))
        .collect();
    assert_eq!(warnings.len(), 1);
}

#[test]
fn parse_skill_name_match_no_warning() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("my-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Name matches dir\n---\nBody.",
    )
    .unwrap();

    let (_, diagnostics) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    let warnings: Vec<&SkillDiagnostic> = diagnostics
        .iter()
        .filter(|d| d.message.contains("does not match directory name"))
        .collect();
    assert!(warnings.is_empty());
}

// ---------------------------------------------------------------------------
// License and metadata
// ---------------------------------------------------------------------------

#[test]
fn parse_skill_license() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("lic-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: lic-skill\ndescription: Has license\nlicense: MIT\n---\nBody.",
    )
    .unwrap();

    let (skill, _) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(skill.license, Some("MIT".to_string()));
}

#[test]
fn parse_skill_metadata() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("meta-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: meta-skill\ndescription: Has metadata\nmetadata:\n  author: me\n  version: \"2.0\"\n---\nBody.",
    )
    .unwrap();

    let (skill, _) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(skill.metadata.get("author"), Some(&"me".to_string()));
    assert_eq!(skill.metadata.get("version"), Some(&"2.0".to_string()));
}
