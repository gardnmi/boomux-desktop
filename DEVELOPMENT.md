# Development

## Requirements

- Linux with Wayland or X11
- Current stable Rust
- Zig 0.15.2 for building the vendored Ghostty terminal core
- A compatible running Boomux daemon
- Linux development libraries required by GPUI CE: XKB Common, Wayland, X11,
  XCB shape/fixes, and Fontconfig

On Ubuntu, the GPUI CE project uses:

```console
sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-dev libxcb-shape0-dev libxcb-xfixes0-dev libfontconfig-dev
```

## Local Loop

```console
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo run --release
```

If Zig is not globally installed:

```console
PATH=/path/to/zig-0.15.2:$PATH cargo run --release
```

Use `BOOMUX_DESKTOP_SHELL_ID=<exact-shell-id>` to choose the initial local Shell
for repeatable integration work.

## Architecture Work

Read `CONTEXT.md`, `docs/architecture.md`, and the relevant ADR before changing
cross-component behavior. Protocol and daemon changes are developed and tested
in the Boomux repository first, then consumed here through an updated exact Git
revision.

## Performance Work

Build with `--release`, keep the workload and pane dimensions fixed, and capture
before/after measurements on the same machine. At minimum record:

- Boomux Desktop commit and Boomux version/revision
- pane count and terminal dimensions
- idle or busy workload description
- sample duration
- RSS, PSS, private dirty memory, thread count, and file-descriptor count
- CPU behavior and any visible frame stalls

Use `scripts/memory-report.sh <pid>` for a point-in-time Linux memory report.
See `docs/performance.md` before changing a hot path.
