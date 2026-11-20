#!/bin/sh
# Quillon installer (curl | sh). Downloads the release archive for the
# running platform, verifies its sha256 against the checksum published
# in the same release, and unpacks the binary into ~/.local/bin.
#
#   curl -fsSL https://github.com/AlbiFahriiza/quillon/releases/latest/download/installer.sh | sh
#
# MP4 rendering uses the system ffmpeg at runtime (not bundled):
#   Debian/Ubuntu: sudo apt-get install ffmpeg
#   macOS:         brew install ffmpeg
#   Windows:       choco install ffmpeg   (or use winget)
set -eu

REPO="AlbiFahriiza/quillon"
DEST="${HOME}/.local/bin"

fail() {
    echo "install failed: $1" >&2
    exit 1
}

# --- resolve platform -> target triple -----------------------------------
os=$(uname -s)
arch=$(uname -m)
case "$os:$arch" in
    Linux:x86_64)        target="x86_64-unknown-linux-gnu" ;;
    Linux:aarch64|Linux:arm64) target="aarch64-unknown-linux-gnu" ;;
    Darwin:x86_64)       target="x86_64-apple-darwin" ;;
    Darwin:arm64)        target="aarch64-apple-darwin" ;;
    *) fail "unsupported platform ($os $arch); build from source: cargo build --release -p quillon-cli" ;;
esac

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q "$1" -O "$2"; }
else
    fail "need curl or wget"
fi

command -v sha256sum >/dev/null 2>&1 || fail "need sha256sum (coreutils)"

# --- resolve the newest release tag ---------------------------------------
# /releases/latest never returns prereleases, so list and take the
# first (newest) release instead — beta builds install the same way.
tag=$(fetch "https://api.github.com/repos/${REPO}/releases?per_page=1" - \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
[ -n "$tag" ] || fail "cannot resolve the newest release"

base="https://github.com/${REPO}/releases/download/${tag}"
archive="quillon-${target}.tar.gz"

# --- download, verify, unpack ---------------------------------------------
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch "${base}/${archive}"     "${tmp}/${archive}"
fetch "${base}/${archive}.sha256" "${tmp}/${archive}.sha256"

(cd "$tmp" && sha256sum -c "${archive}.sha256") || fail "checksum mismatch for ${archive}"

mkdir -p "$DEST"
tar -xzf "${tmp}/${archive}" -C "$tmp"
# r2 R2-7: install atomically — copy to a staging name beside the target,
# then rename over it. Two concurrent installs (or a Ctrl-C mid-copy) can
# otherwise leave a half-written binary at $DEST/quillon.
staging="$DEST/quillon.new.$$"
cp "${tmp}/quillon-${target}/quillon" "$staging"
chmod +x "$staging"
mv -f "$staging" "$DEST/quillon"

echo "installed: $DEST/quillon ($tag, $target)"
case ":${PATH}:" in
    *":${DEST}:"*) ;;
    *) echo "note: add ${DEST} to your PATH" ;;
esac
command -v ffmpeg >/dev/null 2>&1 || echo "note: ffmpeg not found — MP4 rendering needs it (see README)"
