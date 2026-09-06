#!/bin/sh
# Run after building Desktop and the pinned Boomux checkout in release mode.
set -eu

[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || {
    echo 'this package target requires Linux x86_64' >&2
    exit 1
}

boomux_source=${1:?usage: scripts/package-release.sh /path/to/pinned/boomux}
revision=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["dependencies"]["boomux"]["rev"])')
[ "$(git -C "$boomux_source" rev-parse HEAD)" = "$revision" ] || {
    echo 'Boomux checkout does not match Cargo.toml' >&2
    exit 1
}
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$stage/bin" "$stage/libexec" dist
install -m755 target/release/boomux-desktop "$stage/libexec/boomux-desktop"
install -m755 "$boomux_source/target/release/boomux" "$stage/bin/boomux"
install -m755 packaging/boomux-desktop "$stage/bin/boomux-desktop"
cp -R packaging/share "$stage/share"
cp LICENSE "$stage/LICENSE"
cp "$boomux_source/LICENSE" "$stage/LICENSE.boomux"
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')
printf 'boomux-desktop %s\nboomux revision %s\n' "$version" "$revision" > "$stage/release.txt"
asset=boomux-desktop-x86_64-unknown-linux-gnu.tar.gz
tar -czf "dist/$asset" -C "$stage" bin libexec share LICENSE LICENSE.boomux release.txt
(cd dist && sha256sum "$asset" > "$asset.sha256")
