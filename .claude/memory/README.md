# Agent memory

Claude Code's memory for this repository, checked in so work can continue on another machine.
`MEMORY.md` is the index; `handoff-2026-10-26.md` is the latest handoff and is read first. Each
other file holds one fact, with the reason and how to apply it. They are notes, not instructions:
the `AGENTS.md` files govern.

Claude Code keeps memory per checkout path, outside the repository, at
`~/.claude/projects/<path with / and . as ->/memory/`. On a new machine, after cloning:

```bash
repo="$PWD"   # run from the clone's root
dir="$HOME/.claude/projects/$(printf '%s' "$repo" | tr '/.' '--')/memory"
mkdir -p "$dir" && cp .claude/memory/*.md "$dir"/ && rm "$dir/README.md"
```

To keep one copy instead, replace that folder with a symlink to `.claude/memory` (memories
written later then show up as changes here to commit).

`private-task-suite.md` (Butterfly's private benchmark built from employer repositories) is
deliberately absent: nothing from those repositories may be committed. It exists only in the
original machine's memory folder.
