#!/bin/sh
# Installs `ordo` and `ordo-engine` from a GitHub release, checking their
# SHA-256 sum:
#
#   curl -fsSL https://raw.githubusercontent.com/FoamScience/ordo/main/install.sh | sh
#
# ORDO_INSTALL_DIR  where to put them (default ~/.local/bin)
# ORDO_VERSION      which release (default the latest)
set -eu

repo=FoamScience/ordo
dir=${ORDO_INSTALL_DIR:-$HOME/.local/bin}
version=${ORDO_VERSION:-latest}

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) target=x86_64-unknown-linux-musl ;;
    Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    Darwin-arm64) target=aarch64-apple-darwin ;;
    *)
        echo "ordo: no prebuilt binary for $(uname -s) $(uname -m); try: cargo install ordo-diff --features web" >&2
        exit 1
        ;;
esac

if [ "$version" = latest ]; then
    base=https://github.com/$repo/releases/latest/download
else
    base=https://github.com/$repo/releases/download/v${version#v}
fi
archive=ordo-$target.tar.gz

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$base/$archive" -o "$tmp/$archive"
curl -fsSL "$base/$archive.sha256" -o "$tmp/$archive.sha256"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "$archive.sha256" >/dev/null)
else
    (cd "$tmp" && shasum -a 256 -c "$archive.sha256" >/dev/null)
fi

mkdir -p "$dir"
tar -xzf "$tmp/$archive" -C "$dir" ordo ordo-engine
echo "ordo: installed ordo and ordo-engine in $dir"
case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "ordo: $dir is not on PATH" >&2 ;;
esac
