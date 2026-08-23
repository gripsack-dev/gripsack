#!/bin/sh
# gripsack installer — curl -fsSL https://gripsack.dev/install.sh | sh
#
# Downloads the latest static musl binary from GitHub releases, verifies
# the checksum, and installs to ~/.local/bin (override: GRIPSACK_BIN).
# Linux x86_64 for now; macOS users: brew install gripsack-dev/tap/gripsack.
set -eu

REPO="gripsack-dev/gripsack"
DEST="${GRIPSACK_BIN:-$HOME/.local/bin}"
TARGET="x86_64-unknown-linux-musl"

if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
    echo "gripsack install.sh supports linux x86_64 for now." >&2
    echo "  macOS: brew install $REPO/tap/gripsack" >&2
    echo "  elsewhere: cargo install gripsack" >&2
    exit 1
fi

latest="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"core-v\([^"]*\)".*/\1/p' | head -1)"
if [ -z "$latest" ]; then
    echo "could not determine the latest release" >&2
    exit 1
fi

pkg="gripsack-$latest-$TARGET"
base="https://github.com/$REPO/releases/download/core-v$latest"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading gripsack $latest ($TARGET)"
curl -fsSL "$base/$pkg.tar.gz" -o "$tmp/$pkg.tar.gz"
curl -fsSL "$base/$pkg.tar.gz.sha256" -o "$tmp/$pkg.tar.gz.sha256"

( cd "$tmp" && sha256sum -c "$pkg.tar.gz.sha256" >/dev/null )

tar -xzf "$tmp/$pkg.tar.gz" -C "$tmp"
mkdir -p "$DEST"
install -m755 "$tmp/$pkg/grip" "$DEST/grip"

echo "installed: $DEST/grip ($("$DEST/grip" --version))"
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) echo "note: $DEST is not on your PATH — add it to your shell profile" ;;
esac
