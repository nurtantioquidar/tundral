---
name: install-tools-via-nix-config
description: "Install CLI tools by adding them to ~/.config/nix, not ad-hoc nix run / brew / global installs"
metadata:
  node_type: memory
  type: feedback
  originSessionId: ec8aa4f3-bd29-4a61-bf8d-d0a3ac062b7f
  modified: 2026-10-22T09:22:14.062Z
---

When a task needs a tool that is not installed, add it declaratively to the user's Nix config
(`~/.config/nix`, shared Home Manager profile `nix/home/home.nix`) instead of `nix run nixpkgs#…`,
`brew install` or a global package-manager install.

**Why:** 2026-10-22, after I ran gitleaks via `nix run nixpkgs#gitleaks`, the user said "if you
wanna install from nix, consider modify it from ~/.config/nix".

**How to apply:** read `~/.config/nix/AGENTS.md` first (nixpkgs-fmt, validate with
`flake check --no-build` or by building `homeConfigurations.hades.activationPackage`); activation
of the Darwin host needs `sudo darwin-rebuild switch --flake ~/.config/nix#styx --impure`, which
the user runs. Project-local tools (uv, cargo deps) still go through the project's own tooling.
