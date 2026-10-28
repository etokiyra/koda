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
#   * the GNU and musl archives are genuinely different builds;
#   * one `koda` .deb and one `koda` .rpm exist, contain `/usr/bin/koda`, and
#     declare their expected package name, version, architecture and libc
#     dependency.
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

# Native Linux packages. They are built by the `packages` job; the release must
# not ship without them.
shopt -s nullglob
debs=("$dist"/koda_"$version"-*_amd64.deb)
rpms=("$dist"/koda-"$version"-*.x86_64.rpm)
shopt -u nullglob

[[ ${#debs[@]} -eq 1 ]] || die "expected one amd64 .deb, found ${#debs[@]}"
[[ ${#rpms[@]} -eq 1 ]] || die "expected one x86_64 .rpm, found ${#rpms[@]}"

command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb is required to verify the .deb"
command -v rpm >/dev/null 2>&1 || die "rpm is required to verify the .rpm"

deb="${debs[0]}"
rpm="${rpms[0]}"
[[ "$(dpkg-deb --field "$deb" Package)" == "koda" ]] || die "unexpected .deb package name"
[[ "$(dpkg-deb --field "$deb" Version)" == "$version-1" ]] || die "unexpected .deb version"
[[ "$(dpkg-deb --field "$deb" Architecture)" == "amd64" ]] || die "unexpected .deb architecture"
dpkg-deb --contents "$deb" | grep -qE '\./usr/bin/koda$' || die ".deb does not contain /usr/bin/koda"
dpkg-deb --field "$deb" Depends | grep -q 'libc6' || die ".deb has no libc6 dependency"

read -r rpm_name rpm_version rpm_release rpm_arch <<< \
    "$(rpm -qp --qf '%{NAME} %{VERSION} %{RELEASE} %{ARCH}\n' "$rpm")"
[[ "$rpm_name" == "koda" ]] || die "unexpected .rpm package name: $rpm_name"
[[ "$rpm_version" == "$version" ]] || die "unexpected .rpm version: $rpm_version"
[[ "$rpm_release" == "1" ]] || die "unexpected .rpm release: $rpm_release"
[[ "$rpm_arch" == "x86_64" ]] || die "unexpected .rpm architecture: $rpm_arch"
rpm -qpl "$rpm" | grep -qx '/usr/bin/koda' || die ".rpm does not contain /usr/bin/koda"
rpm -qpR "$rpm" | grep -q 'libc\.so\.6' || die ".rpm has no libc dependency"

echo "verify-release: OK linux packages ($(basename "$deb"), $(basename "$rpm"))"

echo "verify-release: all ${#targets[@]} artifacts and both Linux packages verified for $version"
