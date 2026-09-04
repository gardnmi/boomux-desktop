# Architecture

## System Boundary

Boomux Desktop is a presentation client. Boomux remains the source of truth for
terminal processes and durable resources.

```text
Boomux daemon / PTYs
        │ attach protocol with backpressure
        ▼
per-pane reader ── bounded command queue ── libghostty-vt worker
                                                │ immutable screen snapshot
                                                ▼
                                      GPUI model and image cache
                                                │
                                                ▼
                                      tiled/floating GPU scene
```

The client sends input, focus, and terminal-size frames back through the same
Boomux attachment. Keystrokes enter the pane's bounded emulator command queue,
where Ghostty's reusable key encoder reads the current terminal modes before
encoding and forwarding them; this keeps Kitty keyboard, modifyOtherKeys,
cursor, keypad, and backarrow negotiation ordered with terminal output.
Detaching a pane never implies closing its Boomux Shell.

GPUI key contexts separate ordinary terminal input from desktop layout actions.
The default `Terminal` context reserves only explicit lifecycle, clipboard, and
mode-entry commands; other keys reach the pane encoder. `Ctrl+Space` activates
the `Layout` context, where unmodified navigation keys and their Shift/Alt
variants manipulate panes until `Escape` exits. The active context is reflected
by a persistent canvas indicator rather than inferred from the last command.

## Module Map

- `src/main.rs`: application model, Boomux sidebar projection, input routing,
  pane lifecycle, GPUI elements, terminal cell drawing, and GPU image caching.
- `src/layout.rs`: binary split tree, normalized rectangles, spatial focus,
  pane insertion/removal, swaps, and bounded split ratios.
- `src/terminal.rs`: Boomux discovery/attachment adapter, per-pane terminal
  worker, Ghostty VT state, scrollback, key/paste encoding, and Kitty graphics
  extraction.
- `src/theme.rs`: bounded Omarchy palette loading, semantic application and
  terminal colors, built-in fallback, and the current-theme filesystem watcher.

## Threading And Backpressure

GPUI's thread owns presentation state and must not wait on the daemon, PTY, or
terminal parser. Each attached pane has a socket reader and one terminal worker.
The reader sends byte chunks through a bounded queue. When decoding falls behind,
pressure propagates back through the Boomux socket to the PTY producer; arbitrary
terminal bytes are never discarded because doing so could corrupt escape or
Kitty graphics sequences.

The worker coalesces queued commands before publishing a reference-counted,
immutable screen snapshot. A bounded one-event mailbox wakes GPUI only when a
new snapshot or terminal status exists; bursts collapse into one wakeup because
the consumer always reads the newest snapshot. Synchronized-output mode delays
publication until the terminal frame is complete.

Omarchy theme loading follows the same boundary. A native filesystem watcher
observes `~/.local/state/omarchy/current`, because Omarchy replaces its `theme`
directory atomically. Its capacity-one notification channel is debounced before
a bounded `colors.toml` read runs on the background executor. GPUI installs the
result through atomic semantic color slots and notifies once. Each terminal
worker receives only its latest pending palette through its existing bounded
command path and republishes a screen without reconnecting the Boomux Shell.

## Rendering

Text cells and Kitty image placements come from the same Ghostty terminal state.
The GPUI layer draws background images, cells, and foreground images in z-order,
clips every placement to its pane, and caches GPU images by terminal generation.
Images are explicitly dropped when their generation disappears or their pane
closes. Each pane also owns one shaped-text paint cache keyed by the exact screen
snapshot and selection. Layout-only animation frames reuse that cache, while a
new snapshot or selection invalidates it.

The default presentation draws the binary tile layout and floating layer. The
optional tabbed-minimization presentation keeps every open pane in that canvas.
`Ctrl+W` still detaches and releases the pane-owned emulator and GPU state, but
the minimized Shell is represented in a restorable strip above the canvas and
is represented only by its restore tab. Tabs hides all nested Shell rows from
the sidebar, leaving Workspace rows as its navigation surface. Restoring a tab
creates a new pane attachment without changing Boomux's durable Shell identity.

## Resource Projection

The sidebar is a bounded, read-only Boomux snapshot. Active Agent rows require an
exact current ShellRun. Historical records appear only through explicit Boomux
attention/history semantics; durable records are not assumed to be active.
The client keeps a bounded presentation marker for an observed `working` to
`idle` transition so successful completion remains visible until the user
dismisses it. This does not change the Agent lifecycle. Durable attention is
acknowledged against Boomux with the exact Agent ID and observation revision.

Workspace ordering is client-owned presentation state keyed by exact Workspace
identity. Overview refreshes retain the current order, newly discovered
Workspaces append, and drag or keyboard reordering never mutates Boomux's
Workspace authority.

## Dependency Boundary

Until Boomux publishes a focused client crate, Cargo pins an exact Boomux Git
revision. This avoids an implicit sibling checkout while keeping compatibility
reviewable. A future `boomux-client`/`boomux-protocol` crate would reduce desktop
compile time and dependency surface without moving UI code into Boomux.
