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
    share/applications/org.omarchy.boomux-desktop.desktop
    share/icons/hicolor/scalable/apps/org.omarchy.boomux-desktop.svg
    LICENSE
    LICENSE.boomux
    release.txt
  desktop-entry               # generated entry with the absolute launcher path
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

## Desktop Integration And Preferences

The bundle includes a temporary four-tile SVG icon and an application entry.
The installer registers `org.omarchy.boomux-desktop.desktop` in
`${XDG_DATA_HOME:-$HOME/.local/share}/applications` and links the icon into the
matching `icons/hicolor/scalable/apps` directory. It preserves unrelated menu
entries and icons by refusing to replace them. The installed entry uses the
absolute bundled launcher path, including proper quoting for paths with spaces;
launching from the menu does not depend on the session's PATH.

Desktop preferences are stored in
`${XDG_CONFIG_HOME:-$HOME/.config}/boomux-desktop/settings.toml`. The app loads
this file on startup and saves changes made through Settings automatically.
It persists sidebar visibility, pane headings, corner style, spacing, focus
highlight strength, motion speed, pane scope, tiled/tabbed presentation, and
removal confirmation. Window geometry, pane arrangements, minimized Shells, and
Workspace ordering are not yet restored across restarts. Boomux still owns
Shell persistence and resource identities.

Example preferences (omitted keys use defaults):

```toml
pane_gap = 8
motion_speed = "smooth" # instant, fast, smooth
pane_corner_style = "rounded" # rounded, square, mixed
pane_headings_visible = true
confirm_destructive_actions = true
```

Writes use a capacity-one background queue and atomic file replacement. Normal
application shutdown waits asynchronously for the final queued write, within
GPUI's shutdown deadline. Forced termination can still interrupt a pending save.
With several Desktop instances, the last completed save wins. Manual file edits
are read on the next launch. Invalid or oversized files are left intact: the app
uses defaults, disables saving for that session, and reports the problem in
Settings. Correct the file (or move it aside) and restart to resume saving.

Settings is one list grouped into Layout & workspaces, Appearance, Notifications &
sounds, Recovery & history, and Safety.
Changes save automatically. Text fields use Done or Enter to save, Escape to cancel,
and Ctrl+A to clear. Clipboard paste is supported.

Shared preferences are saved through Boomux's supported `config edit` transaction.
Controls show configured values from the active file, global configuration, and
pinned defaults; only edited fields are written. `BOOMUX_CONFIG` selects the active
override file when set. Boomux validates the candidate, checks ownership and
conflicts, and atomically commits it. Comments and unrelated fields are preserved.

Notifications is a master switch for desktop popups and sounds. Turning it off
disables both channels and dims dependent controls while preserving event choices
and sound names. Turning it back on enables popups; sounds can then be enabled
separately. Sound-name controls are unavailable while sounds are off.

Notification and recovery changes set one restart reminder
in the settings header. Individual settings have no restart labels and saving
does not interrupt editing. Closing settings offers **Restart now** or **Later**;
the header button also opens confirmation when ready. The reminder survives
Desktop restarts. Restart invokes the
local `boomux daemon restart` graceful handoff on a worker thread, preserving
running shells and commands. A failed restart retains the reminder and reports
the error. An external restart may leave a conservative reminder until a confirmed
restart from Settings. Other preferences do not request a daemon restart.
Remote Node configuration remains managed on those Nodes.

## Uninstall

Close Desktop first. Remove only the links owned by this installer:

```sh
release_data_dir=${XDG_DATA_HOME:-$HOME/.local/share}
release_install_dir=${BOOMUX_DESKTOP_INSTALL_DIR:-$release_data_dir/boomux-desktop}
release_bin_dir=${BOOMUX_DESKTOP_BIN_DIR:-$HOME/.local/bin}
release_install_dir=$(cd "$release_install_dir" && pwd -P) || exit 1
release_app_id=org.omarchy.boomux-desktop

remove_owned_link() {
    if [ "$(readlink "$1" 2>/dev/null)" = "$2" ]; then
        rm -- "$1"
    fi
}
remove_owned_link "$release_bin_dir/boomux-desktop" "$release_install_dir/current/bin/boomux-desktop"
remove_owned_link "$release_bin_dir/boomux" "$release_install_dir/current/bin/boomux"
remove_owned_link "$release_data_dir/applications/$release_app_id.desktop" "$release_install_dir/desktop-entry"
remove_owned_link "$release_data_dir/icons/hicolor/scalable/apps/$release_app_id.svg" "$release_install_dir/current/share/icons/hicolor/scalable/apps/$release_app_id.svg"
```

Use the same custom directory overrides used during installation. Existing
standalone Boomux executables are preserved. These steps do not stop a daemon,
close Shells, or delete Boomux configuration/history.

The release directory may also supply a running Boomux daemon. Keep it until no
Desktop or Boomux process uses its executables. If you will keep using Boomux,
install it separately and use Boomux's own lifecycle procedure to move off the
bundled daemon before deleting the directory. Once it is unused, remove the
bundle directory printed by `printf '%s\n' "$release_install_dir"`.

Desktop preferences are retained for reinstalling. To reset them, remove only
`${XDG_CONFIG_HOME:-$HOME/.config}/boomux-desktop/settings.toml`. Do not remove
Boomux's configuration or data directories to uninstall Desktop.
