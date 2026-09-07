# Retired repository

Development moved to https://github.com/gardnmi/boomux/tree/main/desktop.
Do product work in that repository and follow its root and Desktop AGENTS.md.
Historical source remains here for reference and must not receive new releases.

The only maintained compatibility surface at cutover is `install.sh`, which
forwards to the canonical release installer. Validate forwarding changes with:

```console
sh -n install.sh
python3 scripts/test-installer.py
```

Use Conventional Commits. Keep agent-managed worktrees under
`~/Worktrees/boomux-desktop/<branch-slug>`, replacing branch slashes with hyphens.
Inspect `git worktree list` first and remove worktrees through Git.
