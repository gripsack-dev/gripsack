#!/bin/sh
# Install grip from a GitHub release: download the tarball, verify its
# published SHA-256, install atomically. Needs only curl, tar and sha256sum.
#   install.sh [--version X.Y.Z]      (or GRIPSACK_VERSION=X.Y.Z)
# GRIPSACK_BIN selects the destination directory (default ~/.local/bin).
set -eu

die() { echo "install.sh: $*" >&2; exit 1; }

base=${GRIPSACK_BASE_URL:-https://github.com/gripsack-dev/gripsack}
version=${GRIPSACK_VERSION:-}
while [ $# -gt 0 ]; do
  case $1 in
    --version) [ $# -ge 2 ] || die '--version needs a value'; version=$2; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] ||
  die "only Linux x86_64 is supported (this is $(uname -s) $(uname -m))"
for tool in curl tar sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
done

if [ -z "$version" ]; then
  tag=$(curl -fsS --retry 3 -o /dev/null -w '%{url_effective}' -L "$base/releases/latest") ||
    die 'cannot resolve the latest release'
  version=${tag##*/core-v}
fi
case $version in
  *[!0-9.]* | '' | .* | *..* | *.) die "invalid version: $version" ;;
esac

name=gripsack-$version-x86_64-unknown-linux-musl
url=$base/releases/download/core-v$version
dest=${GRIPSACK_BIN:-$HOME/.local/bin}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

curl -fsSL --retry 3 -o "$work/$name.tar.gz" "$url/$name.tar.gz" || die "cannot download $name.tar.gz"
curl -fsSL --retry 3 -o "$work/$name.tar.gz.sha256" "$url/$name.tar.gz.sha256" || die 'cannot download the checksum'
(cd "$work" && sha256sum -c "$name.tar.gz.sha256" >/dev/null 2>&1) ||
  die "checksum mismatch for $name.tar.gz; nothing installed"

tar -xzf "$work/$name.tar.gz" -C "$work" "$name/grip"
mkdir -p "$dest"
cp "$work/$name/grip" "$dest/.grip.$$"
chmod 755 "$dest/.grip.$$"
mv -f "$dest/.grip.$$" "$dest/grip"
echo "installed grip $version to $dest/grip"
