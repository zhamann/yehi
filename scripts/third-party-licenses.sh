#!/bin/sh
# Writes the licenses of the crates compiled into yehi's binaries to a file (releases ship it as
# THIRD-PARTY-LICENSES.txt), using a checksum-verified cargo-about. Fails when a dependency's license isn't one
# about.toml accepts. Needs cargo and curl.
#   scripts/third-party-licenses.sh <out file>
set -eu
out=${1:?usage: scripts/third-party-licenses.sh <out file>}
mkdir -p "$(dirname "$out")"
out="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")"
cd "$(dirname "$0")/.."

version=0.9.3
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl sum=3ad1e94ea1e63d69d1961f4fb15ccbbe9575d906111f96d2cb4dce6df48f71b4 ;;
  Linux-aarch64) target=aarch64-unknown-linux-musl sum=eb4d9ff6ebd2518152a8308729431a04b3836e71d2bd5ead7aea69f91a71162e ;;
  Darwin-arm64) target=aarch64-apple-darwin sum=45520df8680978a6e7c4023ace0e3f311c73efc208cd68e5a7f4ed4822fa0349 ;;
  *) echo "no cargo-about $version build for $(uname -sm) here; run it on Linux or Apple silicon" >&2; exit 1 ;;
esac
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
name=cargo-about-$version-$target
curl -sSfL "https://github.com/EmbarkStudios/cargo-about/releases/download/$version/$name.tar.gz" -o "$tmp/about.tar.gz"
if command -v sha256sum >/dev/null; then
  echo "$sum  $tmp/about.tar.gz" | sha256sum -c - >/dev/null
else
  echo "$sum  $tmp/about.tar.gz" | shasum -a 256 -c - >/dev/null
fi
tar -xzf "$tmp/about.tar.gz" -C "$tmp"
"$tmp/$name/cargo-about" generate --locked about.hbs -o "$out" 2>"$tmp/log" || { cat "$tmp/log" >&2; exit 1; }
echo "wrote $out"
