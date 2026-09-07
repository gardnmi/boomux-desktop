# Boomux Desktop has moved

Boomux Desktop is developed and released in [gardnmi/boomux](https://github.com/gardnmi/boomux), under [`desktop/`](https://github.com/gardnmi/boomux/tree/main/desktop).

The unified **v1.10.0** release includes Desktop and its matching Boomux daemon.
This repository retains the historical source and issues; it no longer publishes
releases or accepts development changes.

## Install or update

```sh
curl -fsSL https://github.com/gardnmi/boomux/releases/latest/download/boomux-desktop-installer.sh | sh
```

The old `install.sh` URL forwards to that installer. Existing development builds
need this one-time installation to gain in-app updates. Installer-owned bundles
support **Update → Restart now**, preserving running Shells through graceful
handoff. See [migration and compatibility instructions](https://github.com/gardnmi/boomux/blob/main/docs/desktop/releases.md)
for older daemons; do not stop active work to migrate automatically.

Use the consolidated [issue tracker](https://github.com/gardnmi/boomux/issues),
[development guide](https://github.com/gardnmi/boomux/blob/main/DEVELOPMENT.md), and
[Desktop documentation](https://github.com/gardnmi/boomux/blob/main/desktop/README.md).
Other documentation in this repository describes the historical implementation.
