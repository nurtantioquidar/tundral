# Upstream provenance

This directory is a vendored copy of a third-party skill pack.

- **Source**: <https://github.com/pageton/tui-design-skill>
- **Commit**: `53194a47eca31172f0e0377e8ffcba1386c15629` (2026-09-04)
- **License**: not stated by upstream at that commit. Treat as read-mostly reference material;
  do not redistribute the pack outside this repository.

## What was vendored

`SKILL.md`, `references/`, `frameworks/`, `patterns/`, and `scripts/check-mockups.py`.

## What was deliberately left out

- `templates/` — four single-file starter apps (Go, Python, Rust, TypeScript).
- `projects/` — two runnable example apps (`dbview-go`, `log-monitor-rust`).
- `commands/`, `README.md`, `AGENTS.md`, `justfile`, `.markdownlint.json` — upstream install and
  repo-maintenance scaffolding that does not apply here.

Vendoring four extra language toolchains into a Rust + Bun monorepo buys nothing: `go.mod`,
`Cargo.toml`, and `package.json` files under `.claude/` would be dead weight that tooling has to
learn to ignore. When a runnable reference is genuinely useful — `log-monitor-rust` is the
reference implementation of `references/stability-and-robustness.md` — clone it into a scratch
directory instead:

```bash
git clone --depth 1 https://github.com/pageton/tui-design-skill /tmp/tui-design-skill
```

## Local modifications

1. Added Claude Code skill frontmatter (`name`, `description`, `version`) to `SKILL.md`; upstream
   ships it without frontmatter, so it is not discoverable as a skill unmodified.
2. Added the **Monet Constraints** section to `SKILL.md` — Butterfly's inline ratatui TUI, the
   ambiguous-width measurement rule, and the cached-prefix and engine/UI boundaries, plus the
   required verification command. (Until 2026-09-17 this section said Butterfly was *not* a
   Ratatui app and forbade adopting it; the migration landed that day, so the constraint now
   covers what remains off-limits — the alternate screen for the session — instead.)
3. Replaced the `projects/` bullet in the reference list with a pointer to upstream.

## Re-syncing

Diff upstream against this copy, apply upstream changes to the vendored files, then re-apply the
three local modifications above and bump the commit recorded here. Keep the Monet Constraints
section at the top of `SKILL.md`; it is the part that must survive every sync.
