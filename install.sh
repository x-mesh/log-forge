#!/bin/sh
set -eu

repo="x-mesh/log-forge"
version="${LOG_FORGE_VERSION:-latest}"
install_dir="${LOG_FORGE_INSTALL_DIR:-$HOME/.local/bin}"
base_url="${LOG_FORGE_BASE_URL:-https://github.com/$repo/releases}"

fail() {
    printf 'install.sh: %s\n' "$1" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) os=unknown-linux-musl ;;
    Darwin) os=apple-darwin ;;
    *) fail "unsupported OS: $(uname -s). Download an archive from https://github.com/$repo/releases" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
esac

target="$arch-$os"
asset="log-forge-$target.tar.gz"

if [ "$version" = latest ]; then
    url="$base_url/latest/download/$asset"
else
    case "$version" in
        v*) ;;
        *) version="v$version" ;;
    esac
    url="$base_url/download/$version/$asset"
fi

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        fail "curl or wget is required"
    fi
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "sha256sum or shasum is required"
    fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

printf 'Downloading %s\n' "$url"
download "$url" "$tmp/$asset" || fail "download failed: $url"
download "$url.sha256" "$tmp/$asset.sha256" || fail "download failed: $url.sha256"

expected="$(cut -d ' ' -f 1 <"$tmp/$asset.sha256")"
actual="$(sha256 "$tmp/$asset")"
[ "$expected" = "$actual" ] || fail "checksum mismatch for $asset"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$install_dir"
install -m 755 "$tmp/log-forge-$target/log-forge" "$install_dir/log-forge"
# log-generator selects the Python-compatible defaults from its executable name.
ln -sf log-forge "$install_dir/log-generator"

"$install_dir/log-forge" --version
printf 'Installed log-forge and log-generator to %s\n' "$install_dir"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) printf 'Add %s to PATH.\n' "$install_dir" ;;
esac
