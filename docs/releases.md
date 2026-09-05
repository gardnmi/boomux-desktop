# Release Packaging

## Distribution Contract

One Linux download contains the Desktop executable, a launcher, and the Boomux
executable built from the exact Git revision in Desktop's `Cargo.toml`.
Boomux remains the daemon and resource authority. The launcher delegates to
`boomux daemon start` before executing Desktop, outside GPUI's event loop.
It resolves the bundle location once and prepends its `bin` directory to PATH
so Shells launched from this environment can find the bundled CLI.

The initial target is `x86_64-unknown-linux-gnu`, built on Ubuntu 24.04. This is
not a static or universal Linux binary: the host supplies glibc, graphics
drivers, Fontconfig, Wayland/X11, XCB shape/xfixes, and xkbcommon libraries.
macOS, Windows, ARM, older glibc hosts, and musl hosts are not release targets.
Wayland and X11 availability in the source does not establish compatibility
with an untested distribution.

## Installer

`install.sh` resolves GitHub's latest stable release, downloads the archive and
its SHA-256 checksum over HTTPS, verifies the archive, and installs per user.
It does not invoke sudo, install system libraries, edit shell startup files,
start the daemon, or modify Boomux configuration during installation.

Defaults:

```text
~/.local/share/boomux-desktop/
  releases/<tag>-<archive-sha256>/
    bin/boomux
    bin/boomux-desktop          # launcher
    libexec/boomux-desktop      # graphical executable
    LICENSE
    LICENSE.boomux
    release.txt
  current -> releases/<tag>-<archive-sha256>
~/.local/bin/boomux-desktop -> <install-root>/current/bin/boomux-desktop
~/.local/bin/boomux -> <install-root>/current/bin/boomux
```

`XDG_DATA_HOME` changes the default data directory. Explicit absolute paths can
be supplied with `BOOMUX_DESKTOP_INSTALL_DIR` and `BOOMUX_DESKTOP_BIN_DIR`.
An existing CLI link or executable is preserved. An unrelated executable at
the Desktop command's destination makes installation fail with an explanation.

To select a specific published version, pass the environment variable to `sh`:

```sh
curl -fsSL https://raw.githubusercontent.com/gardnmi/boomux-desktop/main/install.sh \
  | BOOMUX_DESKTOP_VERSION=v0.1.0 sh
```

The example version must actually have published assets. The default channel is
GitHub's latest stable release; there is no background updater or preview-channel
state. Rerunning the installer updates the `current` symlink atomically and
retains previous release folders. Cleanup is user-managed: remove an old folder
only after no Desktop or Boomux process uses it, including for daemon restart or
handoff. The installer never restarts an existing daemon. If protocol
compatibility prevents connection, resolve the daemon upgrade through Boomux's
own lifecycle commands; installing a bundle does not migrate a running daemon.

## Build And Publish

Release Please follows Boomux's release process. After successful push CI on
the default branch, it verifies that CI covered the current commit and creates
or updates a Conventional Commits release PR. Its Rust strategy manages
`Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`, using
`.release-please-manifest.json` for the last known version. The manifest starts
at the project's current version, `0.1.0`.

After the release PR merges and CI succeeds, Release Please creates a draft.
The workflow verifies the release source matches the CI-validated commit and
directly calls `Release bundle` with that exact source and version. This avoids
depending on tag events emitted by the Actions token. An outstanding draft
defers new release proposals until it is completed.

The bundle workflow also supports manual artifact-only builds. It verifies the
requested release version matches `Cargo.toml`, reuses successful triggering
push CI only when its commit exactly matches the checkout (otherwise manual
builds and recovery run the repository checks), checks out the pinned Boomux
revision, builds both projects from a clean checkout with
`--release --locked`, and packages:

- `boomux-desktop-x86_64-unknown-linux-gnu.tar.gz`
- `boomux-desktop-x86_64-unknown-linux-gnu.tar.gz.sha256`

Manual bundle runs upload Actions artifacts only. The Release Please pipeline
waits for the X11 and Wayland smoke jobs to pass, downloads the successful build's artifacts, verifies checksums, uploads both
assets, and then publishes the completed draft, matching Boomux's sequence.
Matching uploaded assets are reused on retry; conflicting digests fail instead
of replacing files. Already published releases are not modified by the helper.

To recover an unfinished draft, manually run **Release Please** with its
`vMAJOR.MINOR.PATCH` tag. The workflow resolves and pins the draft's source,
rebuilds, and retries publication. Dispatching an already published tag does
not rebuild or overwrite it. If rebuilt bytes differ from an uploaded asset,
resolve the conflict explicitly instead of replacing assets automatically.

Publication is automatic after the configured checks, including real GUI smoke
tests of the candidate bundle, pass. The initial automated platform is Ubuntu
24.04 x86-64 using virtual displays and Mesa software rendering. Review
license/notice requirements before the first release. Broader hardware and
distribution coverage can grow with the project; it does not require owning
every target machine.
Drafts are not installable through the public installer. A short domain URL can
serve the installer later without changing the archive layout.

## GitHub Setup

Like Boomux, the workflow uses `RELEASE_PLEASE_TOKEN` when configured and falls
back to `github.token`. Configure that repository secret with a token that can
manage contents, issues, and pull requests if release PRs should automatically
trigger CI. PRs created using only `GITHUB_TOKEN` do not trigger normal PR
workflows. Enable **Allow GitHub Actions to create and approve pull requests**
in repository Actions settings when needed. See the
[Release Please token documentation](https://github.com/googleapis/release-please-action#other-actions-on-release-please-prs).

These files configure the workflow; secrets, repository settings, and the first
hosted workflow run still need to be verified after pushing.

For local packaging, build Desktop and an exact pinned Boomux checkout, then
run `sh scripts/package-release.sh /path/to/boomux`. The script rejects a
checkout whose HEAD differs from the dependency pin. Outputs go in `dist/`.

## Validation

`python3 scripts/test-installer.py` exercises piped installation using local
download fixtures: clean installation and launch ordering, paths with spaces,
repeat installation, updates retaining the previous executable, corrupt
downloads preserving the current installation, existing CLI preservation,
unowned Desktop conflicts, concurrent installation rejection, daemon startup
failure, and unsupported platform/version rejection.

`python3 scripts/test-release.py` uses a mock GitHub CLI to exercise publication
ordering, matching retries, conflicting assets, corrupt archives, network
failures, and already published release protection without contacting GitHub.

`scripts/smoke-desktop.py` runs the actual archive in a temporary environment:

- Verify its checksum and extract the bundled executables.
- Start an isolated Xvfb display; for Wayland, start Weston with an X11 backend
  and an input seat inside that display. Force Mesa software rendering.
- Launch the packaged launcher in a private runtime and verify it starts Boomux.
- Require a mapped X11 window or a committed Wayland window frame and callback.
- Create a pending Shell, launch Desktop with its exact ID, and require a running
  ShellRun with PTY output.
- Exit and reopen Desktop, verifying the same ShellRun and daemon survive.
- Close the fixture Shell, stop its daemon, and terminate owned display/bus
  processes. Save diagnostic logs and `result.json` in `smoke-results/`.

The harness tests the bundled Boomux revision as Desktop's backend. It does not
run Boomux's full test suite or replace its own protocol, persistence, Agent,
Node, or other-client coverage. `scripts/test-smoke.py` checks that crashes,
missing frames, unrelated cursor buffers, and timeouts cannot satisfy readiness.

To run locally after building a bundle, install the runtime/display dependencies
listed in `desktop-smoke.yml`, then run either backend:

```sh
python3 scripts/smoke-desktop.py --backend x11 \
  --archive dist/boomux-desktop-x86_64-unknown-linux-gnu.tar.gz \
  --output smoke-results/x11
```

Use `--backend wayland` for Weston. An optional `--software-driver` selects a
lavapipe ICD JSON outside the system installation. No current display or daemon
is used. A private D-Bus has no host service activation directories.

Additional manual or future automated coverage remains useful:

1. Confirm library resolution for both binaries and install with no Boomux or
   development toolchains present.
2. Launch Desktop, create a Workspace and Shell, and verify terminal input and
   output. Close Desktop and reopen it to confirm the same Shell survives.
3. Repeat with a compatible existing daemon and CLI; confirm there is one daemon
   and existing Shell identities are retained.
4. Install an update while Desktop and a Shell workload are running. Confirm
   the running processes survive and the next launch uses the new bundle.
5. Exercise an incompatible daemon and verify a clear error without an automatic
   stop, restart, or second daemon.

The virtual-display tests are startup and integration checks, not pixel
comparisons or comprehensive keyboard/mouse tests. They do not prove real GPU
behavior or all distributions. macOS and Windows checks will be added with
their corresponding build targets. Record platform and backend when reporting
additional testing.
