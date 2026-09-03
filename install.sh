#!/bin/sh
# gripsack installer — curl -fsSL https://gripsack.dev/install.sh | sh
#
# Detects OS/arch (linux + macOS, x86_64 and aarch64), downloads the
# matching static binary from GitHub releases, verifies the checksum,
# and installs to ~/.local/bin (override: GRIPSACK_BIN).
# macOS users may prefer: brew install --cask gripsack-dev/tap/gripsack
set -eu

REPO="gripsack-dev/gripsack"
DEST="${GRIPSACK_BIN:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) echo "gripsack: unsupported architecture: $arch" >&2; exit 1 ;;
esac
case "$os" in
    Linux)  TARGET="$arch-unknown-linux-musl" ;;
    Darwin) TARGET="$arch-apple-darwin" ;;
    MINGW*|MSYS*|CYGWIN*)
        echo "gripsack: no native Windows build by design — use WSL and run this script inside it" >&2
        exit 1 ;;
    *) echo "gripsack: unsupported OS: $os" >&2; exit 1 ;;
esac

# release objects sort by creation time, which re-cut tags scramble —
# resolve the HIGHEST core-v* git tag by semver instead
latest="$(curl -fsSL "https://api.github.com/repos/$REPO/git/matching-refs/tags/core-v" \
    | sed -n 's|.*"ref": *"refs/tags/core-v\([^"]*\)".*|\1|p' \
    | sort -t. -k1,1nr -k2,2nr -k3,3nr | head -1)"
if [ -z "$latest" ]; then
    echo "gripsack: could not determine the latest core release" >&2
    exit 1
fi

pkg="gripsack-$latest-$TARGET"
base="https://github.com/$REPO/releases/download/core-v$latest"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$@"
    else
        shasum -a 256 "$@"
    fi
}

echo "downloading gripsack $latest ($TARGET)"
curl -fsSL "$base/$pkg.tar.gz" -o "$tmp/$pkg.tar.gz"
curl -fsSL "$base/$pkg.tar.gz.sha256" -o "$tmp/$pkg.tar.gz.sha256"

( cd "$tmp" && sha256 -c "$pkg.tar.gz.sha256" >/dev/null )

tar -xzf "$tmp/$pkg.tar.gz" -C "$tmp"
mkdir -p "$DEST"
install -m755 "$tmp/$pkg/grip" "$DEST/grip"

echo "installed: $DEST/grip ($("$DEST/grip" --version))"
echo "note: your first eval downloads the pinned Deno runtime (~40MB,"
echo "hash-verified, cached under \$GRIPSACK_HOME) — eval is sandboxed in it"
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) echo "note: $DEST is not on your PATH — add it to your shell profile" ;;
esac
