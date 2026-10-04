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

# A pushed tag is not a published artifact. Select the highest stable core tag
# with both assets available for this target; only 404 permits trying an older one.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "https://api.github.com/repos/$REPO/git/matching-refs/tags/core-v" \
    -o "$tmp/tags.json"
versions="$(sed -n 's|.*"ref": *"refs/tags/core-v\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*|\1|p' "$tmp/tags.json" \
    | sort -t. -k1,1nr -k2,2nr -k3,3nr)"

download_release_asset() {
    if status="$(curl -sSL -o "$2" -w '%{http_code}' "$1")"; then
        case "$status" in
            200) return 0 ;;
            404) return 1 ;;
            *) echo "gripsack: release asset request failed (HTTP $status): $1" >&2; exit 1 ;;
        esac
    else
        echo "gripsack: release asset download failed: $1" >&2
        exit 1
    fi
}

selected=""
for version in $versions; do
    pkg="gripsack-$version-$TARGET"
    base="https://github.com/$REPO/releases/download/core-v$version"
    echo "checking gripsack $version ($TARGET)"
    if download_release_asset "$base/$pkg.tar.gz.sha256" "$tmp/$pkg.tar.gz.sha256" \
        && download_release_asset "$base/$pkg.tar.gz" "$tmp/$pkg.tar.gz"; then
        selected="$version"
        break
    fi
done
if [ -z "$selected" ]; then
    echo "gripsack: no complete published core release is available for $TARGET" >&2
    exit 1
fi

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$@"
    else
        shasum -a 256 "$@"
    fi
}

echo "installing gripsack $selected ($TARGET)"

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
