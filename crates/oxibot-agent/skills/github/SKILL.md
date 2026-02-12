---
name: github
description: "Interact with GitHub using the `gh` CLI. Use `gh issue`, `gh pr`, `gh run`, and `gh api` for issues, PRs, CI runs, and advanced queries."
metadata: {"nanobot":{"emoji":"🐙","requires":{"bins":["gh"]}}}
---

# GitHub Skill

Use the `gh` CLI to interact with GitHub. Always specify `--repo owner/repo`
when not in a git directory, or use URLs directly.

## Common Commands

### Issues

```bash
# List open issues
gh issue list --repo owner/repo

# Create an issue
gh issue create --repo owner/repo --title "Bug" --body "Description"

# View issue details
gh issue view 42 --repo owner/repo

# Close an issue
gh issue close 42 --repo owner/repo
```

### Pull Requests

```bash
# List open PRs
gh pr list --repo owner/repo

# Create a PR
gh pr create --title "Feature" --body "Description" --base main

# View PR details
gh pr view 42 --repo owner/repo

# Merge a PR
gh pr merge 42 --repo owner/repo --merge
```

### CI / Actions

```bash
# List recent workflow runs
gh run list --repo owner/repo --limit 5

# View a specific run
gh run view <run-id> --repo owner/repo

# Watch a running workflow
gh run watch <run-id> --repo owner/repo
```

### API (advanced)

```bash
# GraphQL query
gh api graphql -f query='{ viewer { login } }'

# REST endpoint
gh api /repos/owner/repo/releases --jq '.[0].tag_name'
```

## Tips

- Use `--json` flag for structured output: `gh issue list --json number,title`
- Use `--jq` for filtering: `gh pr list --json number,title --jq '.[] | .title'`
- For repos not in current directory, always use `--repo owner/repo`
- Check auth status: `gh auth status`
