#!/usr/bin/env bash
# Verify a directory of Koda release artifacts before they are published.
#
# Usage: verify-release.sh <version> <dist-dir>
#
# Checks, and fails on the first problem:
#   * all four expected archives exist and their names match their targets;
#   * each archive has a sidecar `.sha256` and the checksum validates;
#   * each archive contains an executable `koda`;
#   * each embedded binary has the architecture/linkage its target claims
#     (via check-binary.sh);
#   * the GNU and musl archives are genuinely different builds.
#
# It inspects archives; it does not run Koda.
set -euo pipefail

die() {
    echo "verify-release: $*" >&2
    exit 1
}

version="${1:?usage: verify-release.sh <version> <dist-dir>}"
dist="${2:?usage: verify-release.sh <version> <dist-dir>}"

[[ -d "$dist" ]] || die "dist directory not found: $dist"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
checker="$script_dir/check-binary.sh"
[[ -x "$checker" ]] || die "check-binary.sh not found or not executable: $checker"

targets=(linux-x86_64-gnu linux-x86_64-musl macos-x86_64 macos-arm64)

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

for target in "${targets[@]}"; do
    archive="$dist/koda-$version-$target.tar.gz"
    sidecar="$archive.sha256"
    [[ -f "$archive" ]] || die "missing archive: $(basename "$archive")"
    [[ -f "$sidecar" ]] || die "missing checksum: $(basename "$sidecar")"

    # Validate the sidecar. Its format is `<hex>  <basename>`.
    ( cd "$dist" && sha256sum -c "$(basename "$sidecar")" ) >/dev/null \
        || die "checksum failed for $(basename "$archive")"

    inner="$tmp/$target"
    mkdir -p "$inner"
    tar -xzf "$archive" -C "$inner" || die "could not unpack $(basename "$archive")"
    [[ -f "$inner/koda" ]] || die "$(basename "$archive") does not contain koda"
    "$checker" "$target" "$inner/koda"

    echo "verify-release: OK $target"
done

# The two Linux archives must not be copies of one another.
gnu="$dist/koda-$version-linux-x86_64-gnu.tar.gz"
musl="$dist/koda-$version-linux-x86_64-musl.tar.gz"
if cmp -s "$gnu" "$musl"; then
    die "the GNU and musl archives are byte-identical"
fi

echo "verify-release: all ${#targets[@]} artifacts verified for $version"
