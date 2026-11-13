#!/usr/bin/env bash
# Pack the koyomi engine for Debian: dist/koyomi-debian-<ver>-<amd64|arm64>.tar.gz
# with the binary, install.sh, koyomi-secret, koyomi-user, the unit and the
# README. The release has no plain koyomi-v<ver>-linux-*.tar.gz
# (release_linux=no in scripts/lanes.conf): these files are koyomi's Linux build.
#
#   scripts/package-linux.sh [x86_64|aarch64] [--binary <path>] [--deb]
#
# Without --binary it cross-builds with cargo-zigbuild against glibc 2.31 (as
# scripts/verify.sh does for its zigbuild). With --binary it wraps that file
# as it is, which is how to try the packing without a build.
# With --deb it also writes dist/koyomi_<ver>_<amd64|arm64>.deb (scripts/make-deb.py).
# Permissions are set by tar, since this may run in Git Bash on Windows where
# the executable bit of a file means nothing.
set -euo pipefail

arch=x86_64
binary=
want_deb=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    x86_64|aarch64) arch=$1; shift ;;
    --binary)
      [ "$#" -ge 2 ] || { echo "--binary needs a path" >&2; exit 2; }
      binary=$2; shift 2 ;;
    --deb) want_deb=1; shift ;;
    -h|--help)
      echo "usage: scripts/package-linux.sh [x86_64|aarch64] [--binary <path>] [--deb]"
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

case "$arch" in
  x86_64) deb_arch=amd64 ;;
  aarch64) deb_arch=arm64 ;;
esac
name="koyomi-debian-$ver-$deb_arch"
rm -rf "dist/$name" "dist/$name.tar.gz" "dist/$name.tar"
mkdir -p "dist/$name"
cp "$binary" "dist/$name/koyomi"
cp packaging/linux/install.sh packaging/linux/koyomi-secret packaging/linux/koyomi-user packaging/linux/koyomi.service packaging/linux/README.md "dist/$name/"

# Relative paths from dist/, so no drive letter `:` reaches tar.
(
  cd dist
  own=(--owner=0 --group=0 --numeric-owner)
  tar -cf "$name.tar" "${own[@]}" --mode=0755 "$name/koyomi" "$name/install.sh" "$name/koyomi-secret" "$name/koyomi-user"
  tar -rf "$name.tar" "${own[@]}" --mode=0644 "$name/koyomi.service" "$name/README.md"
  gzip -f "$name.tar"
)

rm -rf "dist/$name"
echo "dist/$name.tar.gz"
sha256sum "dist/$name.tar.gz"

if [ "$want_deb" -eq 1 ]; then
  deb="dist/koyomi_${ver}_${deb_arch}.deb"
  stage="dist/deb-stage-$deb_arch"
  control="dist/deb-control-$deb_arch"
  rm -rf "$stage" "$control" "$deb"
  # The staging goes on a failure too (make-deb.py writes the .deb only at its end).
  trap 'rm -rf "$stage" "$control"' EXIT
  mkdir -p "$stage/usr/bin" "$stage/usr/sbin" "$stage/usr/lib/systemd/system" "$stage/usr/share/koyomi" "$stage/usr/share/doc/koyomi"
  cp "$binary" "$stage/usr/bin/koyomi"
  cp packaging/linux/koyomi-secret packaging/linux/koyomi-user "$stage/usr/sbin/"
  # One unit file: the same as the tar.gz one, with the .deb's binary path.
  sed 's#/usr/local/bin/koyomi#/usr/bin/koyomi#g' packaging/linux/koyomi.service > "$stage/usr/lib/systemd/system/koyomi.service"
  cp packaging/linux/install.sh "$stage/usr/share/koyomi/install.sh"
  cp packaging/linux/README.md "$stage/usr/share/doc/koyomi/README.md"
  cp packaging/deb/copyright "$stage/usr/share/doc/koyomi/copyright"

  bytes=$(find "$stage" -type f -exec cat {} + | wc -c)
  size_kib=$(( (bytes + 1023) / 1024 ))
  sed "s/@VERSION@/$ver/; s/@ARCH@/$deb_arch/; s/@SIZE@/$size_kib/" packaging/deb/control.in > "$control"
  # The maintainer scripts sit next to control.in; make-deb.py reads them from there.
  epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
  python3 -I scripts/make-deb.py --scripts packaging/deb --mtime "$epoch" "$stage" "$control" "$deb"
  echo "$deb"
  sha256sum "$deb"
fi
