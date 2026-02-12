---
name: skill-creator
description: Create or update skills. Use when designing, structuring, or packaging skills with scripts, references, and assets.
metadata: {"nanobot":{"always":false}}
---

# Skill Creator

This skill provides guidance for creating effective skills.

## About Skills

Skills are markdown files that teach you (the AI agent) how to use existing tools
for specific domains. They do NOT add new tools — they are instructional documents.

## Skill Directory Structure

```
skill-name/
├── SKILL.md          # Required — instructions for the agent
├── scripts/          # Optional — shell/python scripts the agent can exec
├── references/       # Optional — extra docs loaded on demand
└── assets/           # Optional — templates, config files, etc.
```

## SKILL.md Format

```markdown
---
name: my-skill
description: "One-line description for the skill catalogue"
metadata: {"nanobot":{"requires":{"bins":["curl"],"env":["API_KEY"]},"always":false}}
---

# Skill Title

Instructions for the agent on how to use this skill...
```

### Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Skill identifier (matches directory name) |
| `description` | Yes | Shown in the skills catalogue XML |
| `metadata` | No | JSON with requirements and flags |

### Metadata JSON

```json
{
  "nanobot": {
    "requires": {
      "bins": ["curl", "jq"],
      "env": ["API_TOKEN"]
    },
    "always": false
  }
}
```

- `requires.bins` — CLI binaries that must be on PATH
- `requires.env` — environment variables that must be set
- `always` — if `true`, skill content is always injected into the system prompt

## Creating a New Skill

1. Create directory: `workspace/skills/<name>/`
2. Write `SKILL.md` with frontmatter + instructions
3. Optionally add `scripts/` with executable helpers
4. The skill will be auto-discovered on next agent invocation

## Best Practices

- Keep instructions concise — the agent reads them on demand
- Use `requires.bins` to declare CLI dependencies
- Prefer `always: false` unless the skill is needed on every interaction
- Include example commands the agent can run with `exec`
- Workspace skills override built-in skills of the same name
