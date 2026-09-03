# Product Context

## Product

**Boomux Desktop** is a native graphical client for Boomux. It presents many
persistent Boomux terminals inside one GPU-rendered window with dynamic tiling,
floating panes, spatial focus, terminal graphics, and an Omarchy-oriented
desktop experience.

Boomux Desktop is not a terminal server, multiplexer backend, compositor, or
replacement authority for Boomux resources.

## Canonical Terms

- **Boomux**: the headless authority that owns durable Workspaces, Shells,
  ShellRuns, Agents, Nodes, PTYs, persistence, and attachment transport.
- **Workspace**: a Boomux-owned grouping of Shells and related resources.
- **Shell**: a durable Boomux terminal identity. Closing a desktop pane does not
  close its Shell.
- **ShellRun**: one process incarnation of a Shell. Agent and attachment state is
  run-scoped.
- **Agent**: a Boomux lifecycle record correlated to one exact ShellRun.
- **Pane**: one Boomux Desktop view attached to a Shell. A pane owns presentation
  state such as selection, scrolling, emulator state, and render-image handles.
- **Tile**: a pane currently placed in the binary split layout.
- **Floating pane**: a pane with desktop-owned bounds outside the split layout.
- **Terminal core**: the `libghostty-vt` state machine that interprets one
  attached Shell's byte stream. It does not own the PTY or process.
- **Desktop overview**: a read-only projection of Boomux Workspaces, Shells, and
  currently presentable Agents used by the sidebar.

## Ownership Boundary

Boomux owns resource truth and fleet-scale behavior. Boomux Desktop owns only
the state required to present currently open panes and a bounded overview.

A change belongs in Boomux when it affects daemon lifecycle, attachment
protocol, PTY fan-out/backpressure, persistence, resource identity, or behavior
shared by every client. A change belongs in Boomux Desktop when it affects GPUI
layout, animation, input routing, terminal drawing, image upload, or desktop
interaction.

The existing `omarchy-boomux` repository remains the Quickshell side-panel
client. It can coexist with Boomux Desktop while this native client matures.
