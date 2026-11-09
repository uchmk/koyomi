#!/usr/bin/env bash
# Pack the koyomi engine for Debian: dist/koyomi-<ver>-linux-<arch>.tar.gz with
# the binary, install.sh, koyomi-secret, the unit and the README.
#
#   scripts/package-linux.sh [x86_64|aarch64] [--binary <path>]
#
# Without --binary it cross-builds with cargo-zigbuild against glibc 2.31 (as
# scripts/verify.sh does for its zigbuild). With --binary it wraps that file
# as it is, which is how to try the packing without a build.
# Permissions are set by tar, since this may run in Git Bash on Windows where
# the executable bit of a file means nothing.
set -euo pipefail

arch=x86_64
binary=
while [ "$#" -gt 0 ]; do
  case "$1" in
    x86_64|aarch64) arch=$1; shift ;;
    --binary)
      [ "$#" -ge 2 ] || { echo "--binary needs a path" >&2; exit 2; }
      binary=$2; shift 2 ;;
    -h|--help)
      echo "usage: scripts/package-linux.sh [x86_64|aarch64] [--binary <path>]"
      exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# A relative --binary is relative to where this was called from.
if [ -n "$binary" ]; then
  [ -s "$binary" ] || { echo "binary not found or empty: $binary" >&2; exit 1; }
  binary="$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")"
fi

cd "$(dirname "$0")/.."

# The workspace version only, not any other `version =` in the file.
ver=$(awk '
  /^\[/ { in_pkg = ($0 == "[workspace.package]"); next }
  in_pkg && /^version[[:space:]]*=/ { gsub(/^[^"]*"|".*$/, ""); print; exit }
' Cargo.toml)
[ -n "$ver" ] || { echo "no workspace version in Cargo.toml" >&2; exit 1; }

target="$arch-unknown-linux-gnu"
if [ -z "$binary" ]; then
  if ! command -v cargo-zigbuild >/dev/null 2>&1; then
    python3 -m pip install -q cargo-zigbuild >/dev/null 2>&1 \
      || python3 -m pip install -q --user --break-system-packages cargo-zigbuild >/dev/null 2>&1 || true
    PATH="$PATH:$HOME/.local/bin"
    command -v cargo-zigbuild >/dev/null 2>&1 || { echo "no cargo-zigbuild (pip install cargo-zigbuild)" >&2; exit 1; }
  fi
  tdir=$(cargo metadata -q --no-deps --format-version 1 | sed 's/.*"target_directory":"\([^"]*\)".*/\1/; s/\\/\//g')
  RUSTUP_TOOLCHAIN=stable rustup target add "$target" >/dev/null 2>&1 || true
  env RUSTUP_TOOLCHAIN=stable ZIG_GLOBAL_CACHE_DIR="$tdir/zig" \
    cargo-zigbuild build --release --locked -p koyomi --bin koyomi --target "$target.2.31"
  binary="$tdir/$target/release/koyomi"
fi
[ -s "$binary" ] || { echo "binary not found or empty: $binary" >&2; exit 1; }

name="koyomi-$ver-linux-$arch"
rm -rf "dist/$name" "dist/$name.tar.gz" "dist/$name.tar"
mkdir -p "dist/$name"
cp "$binary" "dist/$name/koyomi"
cp packaging/linux/install.sh packaging/linux/koyomi-secret packaging/linux/koyomi.service packaging/linux/README.md "dist/$name/"

# Relative paths from dist/, so no drive letter `:` reaches tar.
(
  cd dist
  own=(--owner=0 --group=0 --numeric-owner)
  tar -cf "$name.tar" "${own[@]}" --mode=0755 "$name/koyomi" "$name/install.sh" "$name/koyomi-secret"
  tar -rf "$name.tar" "${own[@]}" --mode=0644 "$name/koyomi.service" "$name/README.md"
  gzip -f "$name.tar"
)

rm -rf "dist/$name"
echo "dist/$name.tar.gz"
sha256sum "dist/$name.tar.gz"
