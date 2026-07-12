# Skills

Skills are self-contained capability packages that teach the agent specialized workflows. The skill format is an open standard defined at [agentskills.io](https://agentskills.io). This document covers Respondami-specific integration.

## Using Skills

### Activating a skill

Type `/` in the input area to trigger skill autocomplete. A fuzzy-matched list of available skills appears. Navigate with `↑`/`↓`, select with `Enter` or `Tab`.

```
/<skill_name>
```

Press **Space** to dismiss the popup and continue typing your prompt.

### Agent auto-activation

The agent can activate skills automatically when it detects a matching task. For example, asking to review code may trigger the `arch-review` skill.

## Discovery Locations

Skills are auto-discovered from two locations:

| Scope   | Path                                          |
| ------- | --------------------------------------------- |
| Global  | `~/.config/respondami/skills/<name>/SKILL.md` |
| Project | `.respondami/skills/<name>/SKILL.md`          |

Project-level skills override global skills on name collision. Each skill is a directory containing a `SKILL.md` file.

## Creating Skills

The skill format is defined by the [agentskills.io](https://agentskills.io) standard. Create a directory with a `SKILL.md` file:

```
~/.config/respondami/skills/my-skill/
└── SKILL.md
```

For the SKILL.md format, frontmatter fields, and best practices, see [agentskills.io](https://agentskills.io).
