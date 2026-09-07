#!/bin/sh
# Compatibility entry point for the retired Desktop repository.
set -eu

main() {
    stage=$(mktemp -d "${TMPDIR:-/tmp}/boomux-desktop-forward.XXXXXXXX")
    trap 'rm -rf -- "$stage"' EXIT
    trap 'exit 1' HUP INT TERM
    curl --proto '=https' --proto-redir '=https' -fsSL --retry 3 \
        --connect-timeout 15 --max-time 120 \
        https://github.com/gardnmi/boomux/releases/latest/download/boomux-desktop-installer.sh \
        -o "$stage/install.sh"
    sh "$stage/install.sh" "$@"
}

# A truncated piped download cannot execute a partial forwarding script.
main "$@"
