# Boomux Desktop

A fast native tiling terminal workspace for [Boomux](https://github.com/gardnmi/boomux), built with GPUI and Ghostty's terminal core.

> [!WARNING]
> Boomux Desktop is experimental. It currently proves the terminal, graphics,
> input, and tiling architecture; it is not yet packaged as a stable desktop
> application.

The prototype uses [GPUI Community Edition](https://gpui-ce.github.io/) and `libghostty-vt`, pinned to released crate versions for reproducible builds. Boomux remains the shell backend: it owns the PTY lifecycle, persistence, reconstruction, and transport, while libghostty interprets those bytes and maintains the reflowing terminal grid rendered by GPUI.

The layout panes are managed inside one GPUI window, and every pane hosts an independent **real local Boomux shell**. Each pane renders its shell's terminal output in GPUI and sends keyboard input and terminal resize events back through Boomux's attachment protocol. This lets us test the product shape without embedding Wayland client buffers or running a terminal emulator process per tile.

An Omarchy Boomux-inspired sidebar presents the local workspace tree, shell status, and current/attention-bearing agents. Workspace rows expand and collapse. Clicking a shell or agent focuses its existing tile, or attaches its shell in a new tile when it is not already open. The overview refreshes from Boomux in the background without putting daemon requests on the GPUI render path. Nodes, web controls, and settings remain outside this proof of concept.

## Run it

```sh
cargo run
```

Building the vendored libghostty dependency requires Zig 0.15.2 on `PATH`. It is only needed at build time. If Zig is installed somewhere outside `PATH`, prepend that directory when invoking Cargo:

```sh
PATH=/path/to/zig-0.15.2:$PATH cargo run
```

The Boomux client dependency is pinned to an exact upstream revision for reproducible standalone builds. On startup the app automatically attaches the most recently focused local Boomux shell, falling back to the first available shell. A pending shell starts, an exited shell restarts, and a running shell is taken over from its current terminal attachment. There is no intermediate shell picker.

High-volume Kitty graphics also require the primary-controller backpressure fix
currently being prepared in Boomux. The first Boomux Desktop release must name
the released Boomux version containing that daemon behavior.

For development and repeatable terminal integration tests, `BOOMUX_DESKTOP_SHELL_ID=<exact-local-shell-id>` overrides the initial shell selection.

`Ctrl + Enter` creates a new Shell and opens it in a new tile. With terminal focus, it uses the focused terminal's Boomux Workspace. With sidebar focus, it uses the selected Workspace or the parent Workspace of the selected Shell or Agent. New Shells use collision-safe random `adjective-noun` names. The sidebar's `+` button creates a randomly named Workspace with its first randomly named Shell and opens it immediately. Clicking a Workspace continues to select and collapse or expand it. `Ctrl + W` closes the focused tile and detaches from its Shell; the Shell and its process remain alive in Boomux and can be reopened from the sidebar. Closing the app itself has the same detach-only behavior, preserving Boomux's normal persistence. Each Workspace row has a `⋮` menu for creating a Shell, renaming, or permanently removing the Workspace. Each Shell row has a `⋮` menu for renaming or permanently removing that Shell. Destructive operations require confirmation.

The application and in-app window titles include the Workspace owning the
focused terminal pane.

Kitty graphics are decoded by `libghostty-vt` and composited into each GPUI terminal canvas with placement cropping and z-ordering. This is sufficient for raw RGB/RGBA applications such as Terminal Doom. From a Boomux shell whose working directory contains `doom1.wad`, run:

```sh
/home/gardnmi/Projects/terminal-doom/zig-out/bin/terminal-doom
```

The prototype uses Ctrl on Linux so its input reaches the app while it is running under Hyprland; Super combinations are normally intercepted by the real compositor. GPUI's portable `secondary` modifier makes these Command shortcuts on macOS. The app shows all controls in its footer:

| Shortcut | Behavior |
| --- | --- |
| `F1` | Open or close the keyboard-shortcut help menu |
| `F6` | Move keyboard focus between the sidebar and the active terminal |
| `F2` | Rename the selected sidebar Workspace/Shell, or the focused terminal's Shell |
| `Ctrl + left drag` | Lift, move, and re-tile a tiled pane |
| `Ctrl + right drag` | Resize a tiled split or floating pane in both axes |
| `Ctrl + Arrow keys` or `Ctrl + H/J/K/L` | Focus a spatially adjacent pane; moving left past the terminal layout enters the sidebar |
| `Ctrl + Shift + Arrow keys` or `Ctrl + Shift + H/J/K/L` | Slide-swap a tiled pane, or move a floating pane |
| `Ctrl + Alt + H/J/K/L` | Resize a tiled split, or resize a floating pane |
| `Ctrl + Enter` | Create a Boomux Shell in the selected or focused Workspace and open it in a new tile |
| `Ctrl + Space` | Toggle the focused pane between tiled and floating |
| `Ctrl + F` | Toggle the focused pane fullscreen without changing its layout |
| `Ctrl + W` | Close the focused terminal tile and detach, preserving its Boomux shell |
| `Ctrl + Shift + W` | Permanently remove the selected or focused Boomux Shell after confirmation |
| `Mouse wheel over terminal` | Scroll through retained terminal history |
| `Shift + Page Up/Page Down` | Scroll terminal history by one viewport, matching Ghostty on Linux |
| `Shift + Home/End` | Jump to the top or bottom of terminal history |
| `Left drag over terminal text` | Select visible terminal cells and publish the selection to the Linux primary clipboard |
| `Ctrl + Shift + C` | Copy the active terminal selection to the system clipboard |
| `Ctrl + Shift + V` | Paste the system clipboard into the focused terminal |
| `Super + C/V` on Omarchy | Universal copy/paste through Omarchy's terminal bindings |
| `Middle click` | Paste the Linux primary selection into the focused terminal |

While the sidebar has keyboard focus, `Up`/`Down` or `J`/`K` moves through
visible rows, `Left`/`Right` or `H`/`L` collapses and expands Workspaces,
`Enter` activates the selected row, `Ctrl+Enter` creates a Shell in that row's
Workspace, `F2` renames the selected Workspace or Shell, `Tab` moves between the Workspaces and
Agents sections, and `Escape` returns to the terminal. `Ctrl+Right` also moves
spatial focus from the sidebar back to the previously active terminal.

Focus follows the pointer as it moves over panes. Clicking still focuses a pane and begins any requested drag operation.

The `?` button in the application header opens the same keyboard-shortcut help menu as `F1`. The menu lists shortcuts by section, prioritizes sidebar commands while the sidebar is active, supports the mouse wheel, Arrow keys, `J`/`K`, Page Up/Page Down, Home/End, and closes with `Escape` or `F1`. While it is open, its separate key context prevents pane commands from reaching the terminal or changing the layout behind the overlay.

Attached terminals show a scrollbar on their right edge. Its thumb is derived from libghostty's native `total`, `offset`, and `len` viewport state and can be dragged anywhere in retained history.

When the Boomux terminal tile is focused, ordinary typing and common control/navigation keys are sent to the shell. The compositor shortcuts above remain owned by the GPUI workspace.

After a small movement threshold, a tiled pane is temporarily lifted into the floating layer and follows the pointer. Neighboring tiles ease into the freed space, and dropping over the left, right, top, or bottom side of another tile animates the pane back into the resulting tiled layout. A modified click without movement does not mutate geometry. Explicitly floating panes remain floating and are clamped to the panel; `Ctrl + Space` toggles that persistent mode.

Keyboard swaps use the same eased geometry transition, so both affected panes
slide between their previous and new tiled positions.

Fullscreen temporarily draws the focused pane over the entire compositor surface. Its tiled position or floating bounds remain unchanged and return when fullscreen is toggled off.

## What this proves

- A binary split tree maps naturally to nested GPUI flex layouts.
- Focus can be calculated spatially instead of relying on DOM/render order.
- Tiled and floating layers can coexist in one GPUI scene.
- Hyprland-like operations can be expressed as ordinary GPUI actions.
- Multiple Boomux shells can be created, attached, reconstructed, rendered, typed into, resized, closed, detached, and reconnected without launching separate terminal windows.
- Kitty graphics capability negotiation, decoded RGB/RGBA images, placement clipping, and z-ordering can be bridged from a Boomux PTY through `libghostty-vt` into GPUI.

## What it does not prove yet

- Full Ghostty rendering parity. GPUI still performs its own cell and image drawing, so advanced cursor styles, PNG transmission, Unicode placeholders, animation, and some Kitty graphics edge cases remain incomplete.
- Mouse reporting for terminal applications, IME, selection spanning unloaded scrollback, hyperlinks, or ligatures.
- Hosting or compositing arbitrary Wayland clients.
- Compositor-grade animation physics and configurable animation curves
- Multiple workspaces and monitors
- Damage tracking, client lifecycle, or compositor security boundaries

The layout model is isolated in `src/layout.rs`; the Boomux lifecycle, attachment, and libghostty terminal adapter live in `src/terminal.rs`.

## Performance

Boomux Desktop is intended to stay responsive and memory-efficient as humans
and agents create more terminals than traditional single-user workflows. The
desktop client therefore treats bounded state, backpressure, render-path
isolation, and measured regressions as architectural requirements rather than
late optimization work.

Boomux owns server-side scale across Shells and attached clients. Boomux Desktop
owns the incremental cost of each visible terminal pane, decoded image, and
frame. See [docs/performance.md](docs/performance.md) for the measurement model
and [docs/architecture.md](docs/architecture.md) for the ownership boundary.
