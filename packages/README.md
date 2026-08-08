# Shared packages

Create a package here only when at least two real workspaces need the same contract or
implementation. Every non-trivial package must include an `AGENTS.md`, public entry points, tests,
and a clearly named owner boundary.

Prefer these narrow package categories:

- `contracts` is active and owns versioned JSON API shapes plus narrow boundary validators;
- `ui` for design-system primitives used by multiple frontends;
- `config` for shared build, lint, or TypeScript configuration.

Do not create generic `common`, `shared`, or `utils` packages.
