use tempfile::TempDir;
use std::path::PathBuf;
use std::collections::HashMap;
use std::fs;

use crate::hooks::HookEvent;
use crate::skills::*;

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_frontmatter_valid() {
    let content = "---\nname: pdf\ndescription: Handle PDF files\nlicense: MIT\n---\n# Body\nContent here";
    let (fm, raw_yaml) = parse_frontmatter(content).expect("should parse");
    assert_eq!(fm.name, Some("pdf".to_string()));
    assert_eq!(fm.description, Some("Handle PDF files".to_string()));
    assert_eq!(fm.license, Some("MIT".to_string()));
    // The raw YAML block is returned for re-parsing (L3: no second scan).
    assert_eq!(raw_yaml, "name: pdf\ndescription: Handle PDF files\nlicense: MIT");
}

#[test]
fn parse_frontmatter_missing_fields() {
    let content = "---\nname: minimal\n---\nBody";
    let (fm, _) = parse_frontmatter(content).expect("should parse");
    assert_eq!(fm.name, Some("minimal".to_string()));
    assert_eq!(fm.description, None);
    assert_eq!(fm.license, None);
}

#[test]
fn parse_frontmatter_with_metadata() {
    let content = "---\nname: test\ndescription: has metadata\nmetadata:\n  author: me\n  version: \"1.0\"\n---\nBody";
    let (fm, _) = parse_frontmatter(content).expect("should parse");
    assert_eq!(fm.name, Some("test".to_string()));
    let meta = fm.metadata.as_ref().expect("should have metadata");
    assert_eq!(meta.get("author"), Some(&"me".to_string()));
    assert_eq!(meta.get("version"), Some(&"1.0".to_string()));
}

#[test]
fn parse_frontmatter_unknown_fields_ignored() {
    let content = "---\nname: test\nunknown_field: value\nanother: 123\ndescription: works\n---\nBody";
    let (fm, _) = parse_frontmatter(content).expect("should parse");
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
    let (fm, raw_yaml) = parse_frontmatter(content).expect("should parse empty block");
    assert_eq!(fm.name, None);
    assert_eq!(fm.description, None);
    assert_eq!(raw_yaml, "");
}

// ---------------------------------------------------------------------------
// skill_body (L3: frontmatter-stripped body, shared by agent + compaction)
// ---------------------------------------------------------------------------

#[test]
fn skill_body_strips_frontmatter() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(
        &path,
        "---\nname: test\ndescription: d\n---\n# Body\nContent here\n",
    )
    .unwrap();
    assert_eq!(
        skill_body(&path).as_deref(),
        Some("# Body\nContent here"),
        "frontmatter must be stripped"
    );
}

#[test]
fn skill_body_without_frontmatter_returns_content() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(&path, "  # Just body\nText\n").unwrap();
    assert_eq!(
        skill_body(&path).as_deref(),
        Some("# Just body\nText"),
        "no frontmatter: content as-is (trimmed)"
    );
}

#[test]
fn skill_body_missing_file_is_none() {
    assert!(skill_body(std::path::Path::new("/nonexistent/SKILL.md")).is_none());
}

// ---------------------------------------------------------------------------
// Name validation
// ---------------------------------------------------------------------------

#[test]
fn validate_name_valid() {
    assert_eq!(validate_name("my-skill"), [] as [String; 0]);
    assert_eq!(validate_name("pdf"), [] as [String; 0]);
    assert_eq!(validate_name("skill-123"), [] as [String; 0]);
    assert_eq!(validate_name("a"), [] as [String; 0]);
}

#[test]
fn validate_name_too_long() {
    let long = "a".repeat(65);
    let warnings = validate_name(&long);
    assert_ne!(warnings, [] as [String; 0]);
}

#[test]
fn validate_name_leading_hyphen() {
    let warnings = validate_name("-bad");
    assert_ne!(warnings, [] as [String; 0]);
}

#[test]
fn validate_name_trailing_hyphen() {
    let warnings = validate_name("bad-");
    assert_ne!(warnings, [] as [String; 0]);
}

#[test]
fn validate_name_consecutive_hyphens() {
    let warnings = validate_name("bad--name");
    assert_ne!(warnings, [] as [String; 0]);
}

#[test]
fn validate_name_invalid_characters() {
    let warnings = validate_name("bad_name");
    assert_ne!(warnings, [] as [String; 0]);
    let warnings = validate_name("BadName");
    assert_ne!(warnings, [] as [String; 0]);
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
    assert_eq!(result.0.description, "");
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
    assert_eq!(format_skills_for_prompt(&[]), "");
}

#[test]
fn format_skills_for_prompt_single() {
    let skills = vec![Skill {
        name: "pdf".to_string(),
        description: "Handle PDF files".to_string(),
        license: None,
        metadata: HashMap::new(),
        hooks: Vec::new(),
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
        hooks: Vec::new(),
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

// ---------------------------------------------------------------------------
// Frontmatter hooks parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_hooks_field_no_hooks() {
    let (hooks, warnings) = parse_hooks_field("name: x\ndescription: y");
    assert_eq!(hooks.len(), 0);
    assert_eq!(warnings.len(), 0);
}

#[test]
fn parse_hooks_field_with_matcher() {
    let yaml = "name: x\n\nhooks:\n  PreToolUse:\n    - matcher: \"Write|Edit\"\n      hooks:\n        - type: command\n          command: \"echo guard\"";
    let (hooks, warnings) = parse_hooks_field(yaml);
    assert_eq!(warnings.len(), 0);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].event, HookEvent::PreToolUse);
    assert_eq!(hooks[0].matcher, Some("Write|Edit".to_string()));
    assert_eq!(hooks[0].command, "echo guard");
}

#[test]
fn parse_hooks_field_no_matcher() {
    let yaml = "hooks:\n  UserPromptSubmit:\n    - hooks:\n        - type: command\n          command: \"echo hi\"";
    let (hooks, warnings) = parse_hooks_field(yaml);
    assert_eq!(warnings.len(), 0);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].event, HookEvent::UserPromptSubmit);
    assert_eq!(hooks[0].matcher, None);
}

#[test]
fn parse_hooks_field_unknown_event_warns() {
    let yaml = "hooks:\n  NotAnEvent:\n    - hooks:\n        - type: command\n          command: \"echo x\"";
    let (hooks, warnings) = parse_hooks_field(yaml);
    assert_eq!(hooks.len(), 0);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("NotAnEvent"));
}

#[test]
fn parse_hooks_field_non_command_type_warns() {
    let yaml = "hooks:\n  Stop:\n    - hooks:\n        - type: prompt\n          command: \"echo x\"";
    let (hooks, warnings) = parse_hooks_field(yaml);
    assert_eq!(hooks.len(), 0);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("type must be \"command\""));
}

#[test]
fn parse_hooks_field_missing_command_warns() {
    let yaml = "hooks:\n  Stop:\n    - hooks:\n        - type: command";
    let (hooks, warnings) = parse_hooks_field(yaml);
    assert_eq!(hooks.len(), 0);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("missing command"));
}

#[test]
fn parse_hooks_field_malformed_hooks_node_warns() {
    // Valid YAML, but `hooks` is a scalar, not a mapping → the node can't
    // deserialize into the expected shape → a single "malformed" warning.
    let yaml = "name: x\nhooks: just-a-string";
    let (hooks, warnings) = parse_hooks_field(yaml);
    assert_eq!(hooks.len(), 0);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("malformed"));
}

#[test]
fn parse_hooks_field_multiple_events_and_groups() {
    let yaml = "hooks:\n  PreToolUse:\n    - matcher: \"Write\"\n      hooks:\n        - type: command\n          command: \"a\"\n    - hooks:\n        - type: command\n          command: \"b\"\n  Stop:\n    - hooks:\n        - type: command\n          command: \"c\"";
    let (hooks, warnings) = parse_hooks_field(yaml);
    assert_eq!(warnings.len(), 0);
    assert_eq!(hooks.len(), 3);
    assert_eq!(hooks[0].command, "a");
    assert_eq!(hooks[0].matcher, Some("Write".to_string()));
    assert_eq!(hooks[1].command, "b");
    assert_eq!(hooks[1].matcher, None);
    assert_eq!(hooks[2].event, HookEvent::Stop);
}

#[test]
fn parse_skill_with_hooks_end_to_end() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("hooky");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: hooky\ndescription: has hooks\n\nhooks:\n  PreToolUse:\n    - matcher: \"Write|Edit\"\n      hooks:\n        - type: command\n          command: \"sh guard.sh\"\n  Stop:\n    - hooks:\n        - type: command\n          command: \"echo done\"\n---\nBody.",
    )
    .unwrap();

    let (skill, diagnostics) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(diagnostics.len(), 0, "no diagnostics expected: {diagnostics:?}");
    assert_eq!(skill.hooks.len(), 2);
    assert_eq!(skill.hooks[0].event, HookEvent::PreToolUse);
    assert_eq!(skill.hooks[0].matcher, Some("Write|Edit".to_string()));
    assert_eq!(skill.hooks[1].event, HookEvent::Stop);
}

#[test]
fn parse_skill_malformed_hooks_keeps_other_fields() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("broken-hooks");
    fs::create_dir(&skill_dir).unwrap();
    // `hooks:` is a scalar, not a mapping → malformed. The rest of the
    // frontmatter must still parse.
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: broken-hooks\ndescription: still loads\nhooks: just-a-string\n---\nBody.",
    )
    .unwrap();

    let (skill, diagnostics) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(skill.name, "broken-hooks");
    assert_eq!(skill.description, "still loads");
    assert_eq!(skill.hooks.len(), 0);
    assert!(diagnostics.iter().any(|d| d.message.contains("malformed")));
}

#[test]
fn parse_skill_no_hooks_backward_compatible() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("plain");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: plain\ndescription: no hooks\n---\nBody.",
    )
    .unwrap();

    let (skill, diagnostics) = parse_skill(&skill_dir.join("SKILL.md"), SkillSource::Global).unwrap();
    assert_eq!(skill.hooks.len(), 0);
    assert_eq!(diagnostics.len(), 0);
}
