# Roadmap

This is non-authoritative future intent. Current behavior is defined by source,
tests, `CONTEXT.md`, architecture documentation, and accepted ADRs.

## Repository Readiness

- Create the `gardnmi/boomux-desktop` remote and connect release automation after
  Linux packaging and smoke-test contracts are defined.
- Add desktop metadata, icons, and an Omarchy installation/integration path.
- Replace the full Boomux Git dependency with a focused published client and
  protocol crate when available.

## Performance

- Add deterministic terminal workloads for 1, 10, 50, and 100 idle panes.
- Add controlled text throughput and Kitty graphics frame-rate fixtures.
- Establish same-machine RSS/PSS and CPU baselines before selecting numeric
  regression budgets.
- Measure retained memory after repeated pane and image open/close cycles.
- Introduce a global graphics budget if per-pane Kitty ceilings scale poorly.
- Track damage-driven painting and avoid rebuilding unchanged terminal cells.

## Product

- Preserve the current terminal, graphics, input, and tiling proof while moving
  from fixed demo presentation to durable user configuration.
- Reach feature parity with the essential `omarchy-boomux` Workspace and Agent
  workflows before considering replacement of that client.
- Add multi-Workspace and multi-monitor presentation without duplicating Boomux
  ownership or Hyprland compositor authority.
