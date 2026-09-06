# Continuous Integration

`.github/workflows/ci.yml` runs on pull requests, pushes to `main`, merge queue
requests, and manual dispatch. Superseded runs for the same ref are cancelled.
Each job has a timeout and read-only repository permissions.

| Check | Coverage |
| --- | --- |
| Quick checks | Workflow lint, Rust formatting, shell syntax, installer and publication fixture tests |
| Rust | Clippy with warnings denied and serial tests; locked release build on main, merge queue, and manual runs |
| Dependency policy | Advisory, license, dependency-source, and duplicate policy via cargo-deny |
| Desktop smoke (x11 / wayland) | Bundled GUI startup, daemon startup, pending Shell attachment, and ShellRun survival; integration/manual builds and releases |

Rust runs on Ubuntu 24.04 with Zig 0.15.2 and the Wayland/X11 build libraries.
The dependency cache is keyed by Rust's toolchain, Cargo manifests/lockfiles,
and an explicit Ubuntu/Zig key. Only `main` runs save caches; PRs can restore
them. Update the explicit key when changing the runner or Zig version.
Caching does not replace the clean builds in the release workflow.

Ordinary PR updates omit the optimized build: tests already compile and link
the application in the test profile. Optimization/link failures are caught by
merge queue validation, when enabled, or by main CI before release automation.
PR and integration tests remain separate because GitHub checks different
commits (the PR merge ref, merge-group commit, and actual main commit).

Main, merge-queue, and manual CI builds also package the pinned Boomux executable
and pass the exact archive to `desktop-smoke.yml`. Separate Ubuntu 24.04 jobs run
X11 on Xvfb with Desktop running under QEMU's Nehalem CPU (no AVX),
and native Wayland on Weston nested inside Xvfb. The nested backend supplies
the input seat GPUI requires; it still needs no physical display. Mesa lavapipe
and llvmpipe provide software rendering. Ordinary PR runs omit these display
jobs along with the optimized build.

The vendored libghostty-vt-sys build forces Zig's baseline CPU target; its
compiler-rt routines must not assume features of the build runner. See
`vendor/libghostty-vt-sys/PATCH.md`. The CI cache key includes baseline-v1
to invalidate earlier CPU-specific objects.

The harness uses private XDG directories and a private D-Bus without service
activation. It checks a mapped X11 window or a Wayland window buffer commit and
frame callback, starts a pending Shell through Desktop, and verifies that the
same ShellRun and daemon survive client exit and reopening. In CPU-emulated
mode, the harness starts Boomux explicitly and sets the bundled PATH before
launching the Desktop ELF through QEMU; native Wayland also exercises the shell
launcher itself. Commands and waits
have deadlines; cleanup closes only the fixture Shell and its isolated daemon.
Logs and exact resource IDs are uploaded for 14 days, including on failures.

Workflow syntax and expressions are checked with actionlint 1.7.7, whose
download is SHA-256 pinned. Its optional ShellCheck/Pyflakes integrations are
disabled; shell parsing and the Python fixture suites run as separate steps.

Release Please follows Boomux's process: successful default-branch push CI
updates a release PR, merging that PR creates a draft, and a successful bundle
build uploads assets before publication. It verifies the release commit passed
the triggering CI run. The bundle contains Desktop and the exact pinned Boomux
revision. Actions artifacts are retained for 14 days. See
[release packaging](releases.md) for the token setup and draft recovery flow.
Concurrent release jobs for the same ref are serialized rather than interrupted.

Automatic bundle builds reuse the triggering CI result only after verifying
that the checked-out source is exactly the successful default-branch push CI
commit. They skip formatting, Clippy, unit/fixture tests, and dependency policy
already covered by that run. They still build both executables from a clean
checkout and verify the archive, checksum, CLI startup, and Desktop library
resolution. Both display jobs then exercise that freshly built archive, and a
failure blocks publication. Manual bundle builds and draft recovery have no triggering CI
proof, so they retain full validation. Publication only verifies and publishes
the completed artifacts; it does not run the test suite again.
The guard uses GitHub's documented
[caller context for reusable workflows](https://docs.github.com/en/actions/reference/workflows-and-actions/reusing-workflow-configurations#github-context).

This removes repeated work without claiming a measured runtime reduction.
The optimized Desktop build still runs in main CI and again during clean
release packaging. Reusing that binary would require artifact provenance and
retention/recovery handling; the clean packaging build is intentional for now.

The existing weekly/manual Performance workflow records terminal-core smoke
measurements and retains artifacts for 90 days. Hosted runner measurements are
diagnostic trends, not evidence for precise performance improvements or hard
regression thresholds. Follow `docs/performance.md` for comparable measurements.

Dependabot proposes weekly updates to the commit-pinned GitHub Actions. Rust
dependency updates remain deliberate compatibility reviews, especially Boomux,
GPUI, and Ghostty.

## Recommended Repository Settings And Follow-up

- After a successful GitHub run, require **Quick checks**, **Rust**, and
  **Dependency policy** in the `main` branch ruleset. Workflow files alone do
  not enable branch protection.
- Review and merge dependency-update PRs through the same required checks.
- Configure `RELEASE_PLEASE_TOKEN` as in Boomux so generated release PRs receive
  CI checks automatically, and enable Actions PR creation in repository settings.
- Expand distro smoke coverage as release targets are added. The automated
  Ubuntu/software-rendering checks do not establish every distribution or GPU's
  compatibility, nor do they cover visual quality and all interactive behavior.
- Add a scheduled dependency audit if advisories should be detected between
  commits. The current dependency check runs with ordinary CI events.
- Expand OS/architecture matrices only as corresponding release targets become
  supported; currently the packaging contract is Linux x86-64 with glibc.

GitHub workflow execution and branch settings must be verified on the remote
after these changes are pushed. Local checks cannot confirm hosted runner
behavior, cache hits, or repository permissions.
