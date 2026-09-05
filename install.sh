#!/bin/sh
# Install the latest stable Linux release, or BOOMUX_DESKTOP_VERSION=vX.Y.Z.
set -eu

fail() { printf 'boomux-desktop: %s\n' "$*" >&2; exit 1; }

main() {
    [ "$(uname -s)" = Linux ] || fail 'only Linux is currently supported'
    [ "$(uname -m)" = x86_64 ] || fail 'only x86_64 is currently supported'
    for tool in curl tar sha256sum readlink mktemp; do
        command -v "$tool" >/dev/null 2>&1 || fail "required command missing: $tool"
    done

    repository=https://github.com/gardnmi/boomux-desktop
    version=${BOOMUX_DESKTOP_VERSION:-}
    if [ -z "$version" ]; then
        latest=$(curl --proto '=https' --proto-redir '=https' -fsSL --retry 3 \
            --connect-timeout 15 --max-time 120 -o /dev/null -w '%{url_effective}' \
            "$repository/releases/latest")
        version=${latest##*/}
    fi
    case "$version" in
        v[0-9]*) ;;
        *) fail 'could not resolve a release; set BOOMUX_DESKTOP_VERSION=vX.Y.Z' ;;
    esac
    case "$version" in *[!a-zA-Z0-9._-]*) fail 'invalid release version' ;; esac

    install_dir=${BOOMUX_DESKTOP_INSTALL_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/boomux-desktop}
    bin_dir=${BOOMUX_DESKTOP_BIN_DIR:-$HOME/.local/bin}
    case "$install_dir:$bin_dir" in /*:/*) ;; *) fail 'install directories must be absolute paths' ;; esac
    mkdir -p "$install_dir/releases" "$bin_dir"
    # Resolve parent symlinks so ownership checks also work on subsequent installs.
    install_dir=$(cd "$install_dir" && pwd -P)
    bin_dir=$(cd "$bin_dir" && pwd -P)
    desktop_link=$install_dir/current/bin/boomux-desktop
    if [ -e "$bin_dir/boomux-desktop" ] || [ -L "$bin_dir/boomux-desktop" ]; then
        [ "$(readlink "$bin_dir/boomux-desktop" || true)" = "$desktop_link" ] ||
            fail "$bin_dir/boomux-desktop already exists and is not owned by this installer"
    fi

    lock=$install_dir/.install-lock
    mkdir "$lock" 2>/dev/null || fail "another install is active; if it was interrupted, remove $lock and retry"
    stage=
    trap '[ -z "$stage" ] || rm -rf -- "$stage"; rmdir "$lock"' EXIT
    trap 'exit 1' HUP INT TERM
    stage=$(mktemp -d "$install_dir/.install.XXXXXXXX")
    asset=boomux-desktop-x86_64-unknown-linux-gnu.tar.gz
    base=$repository/releases/download/$version
    for file in "$asset" "$asset.sha256"; do
        curl --proto '=https' --proto-redir '=https' -fsSL --retry 3 \
            --connect-timeout 15 --max-time 600 "$base/$file" -o "$stage/$file"
    done
    digest=$(awk 'NR == 1 { print $1 }' "$stage/$asset.sha256")
    [ "${#digest}" -eq 64 ] || fail 'invalid checksum file'
    case "$digest" in *[!0-9a-f]*) fail 'invalid checksum file' ;; esac
    printf '%s  %s\n' "$digest" "$asset" > "$stage/checksum"
    (cd "$stage" && sha256sum -c checksum) || fail 'download checksum mismatch'

    mkdir "$stage/payload"
    tar -xzf "$stage/$asset" -C "$stage/payload" --no-same-owner --no-same-permissions \
        bin/boomux bin/boomux-desktop libexec/boomux-desktop LICENSE LICENSE.boomux release.txt
    for file in bin/boomux bin/boomux-desktop libexec/boomux-desktop; do
        [ -f "$stage/payload/$file" ] && [ ! -L "$stage/payload/$file" ] &&
            [ -x "$stage/payload/$file" ] || fail "invalid executable: $file"
    done

    release=$install_dir/releases/$version-$digest
    if [ ! -d "$release" ]; then
        mv "$stage/payload" "$release"
    fi
    ln -s "$release" "$stage/current"
    mv -Tf "$stage/current" "$install_dir/current"
    if [ ! -L "$bin_dir/boomux-desktop" ]; then
        ln -s "$desktop_link" "$bin_dir/boomux-desktop"
    fi
    # Keep an existing user's CLI installation intact.
    if [ ! -e "$bin_dir/boomux" ] && [ ! -L "$bin_dir/boomux" ]; then
        ln -s "$install_dir/current/bin/boomux" "$bin_dir/boomux"
    fi

    printf '\nInstalled Boomux Desktop %s (Boomux included).\n' "$version"
    printf 'Launch: %s/boomux-desktop\n' "$bin_dir"
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *) printf 'Add this directory to your shell PATH: %s\n' "$bin_dir" ;;
    esac
    printf 'Rerun this installer to update. Existing daemon and Shells are left running.\n'
}

# Keep execution last so a truncated piped download cannot run a partial install.
main "$@"
