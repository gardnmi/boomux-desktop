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
Boomux attachment. Detaching a pane never implies closing its Boomux Shell.

## Module Map

- `src/main.rs`: application model, Boomux sidebar projection, input routing,
  pane lifecycle, GPUI elements, terminal cell drawing, and GPU image caching.
- `src/layout.rs`: binary split tree, normalized rectangles, spatial focus,
  pane insertion/removal, swaps, and bounded split ratios.
- `src/terminal.rs`: Boomux discovery/attachment adapter, per-pane terminal
  worker, Ghostty VT state, scrollback, key/paste encoding, and Kitty graphics
  extraction.

## Threading And Backpressure

GPUI's thread owns presentation state and must not wait on the daemon, PTY, or
terminal parser. Each attached pane has a socket reader and one terminal worker.
The reader sends byte chunks through a bounded queue. When decoding falls behind,
pressure propagates back through the Boomux socket to the PTY producer; arbitrary
terminal bytes are never discarded because doing so could corrupt escape or
Kitty graphics sequences.

The worker coalesces queued commands before publishing an immutable screen
snapshot. Synchronized-output mode delays publication until the terminal frame
is complete.

## Rendering

Text cells and Kitty image placements come from the same Ghostty terminal state.
The GPUI layer draws background images, cells, and foreground images in z-order,
clips every placement to its pane, and caches GPU images by terminal generation.
Images are explicitly dropped when their generation disappears or their pane
closes.

## Resource Projection

The sidebar is a bounded, read-only Boomux snapshot. Active Agent rows require an
exact current ShellRun. Historical records appear only through explicit Boomux
attention/history semantics; durable records are not assumed to be active.

## Dependency Boundary

Until Boomux publishes a focused client crate, Cargo pins an exact Boomux Git
revision. This avoids an implicit sibling checkout while keeping compatibility
reviewable. A future `boomux-client`/`boomux-protocol` crate would reduce desktop
compile time and dependency surface without moving UI code into Boomux.
