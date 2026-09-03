# Changelog

All notable changes to Boomux Desktop will be documented in this file.

## Unreleased

- Establish the standalone Boomux Desktop project from the GPUI tiling-terminal
  proof of concept.
- Add Workspace and Shell action menus, background lifecycle mutations, rename
  dialogs, and confirmed permanent removal without changing `Ctrl+W`'s
  detach-only behavior.
- Add a keyboard-first, scrollable shortcut help menu opened with `F1` or the
  application header's `?` button.
- Make `Ctrl+Enter` create a Shell in the selected sidebar row's exact
  Workspace while preserving click-to-collapse behavior.
- Make the sidebar's focus-following Workspace and Shell treatment more subtle
  while preserving its stronger keyboard-navigation highlight.
- Include the focused Workspace in the application title and use Boomux-style
  random names when creating Workspaces and Shells.
- Add `Ctrl+Shift+Arrow` aliases for Hyprland-style directional pane swaps and
  floating-pane movement.
- Animate keyboard-driven tiled pane swaps between their old and new positions.
