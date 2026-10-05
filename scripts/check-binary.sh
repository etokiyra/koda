#!/usr/bin/env bash
# Assert that a release binary matches its target's architecture and linkage.
#
# Usage: check-binary.sh <target-name> <path-to-binary>
#
# The target names are the release artifact suffixes:
#   linux-x86_64-gnu   linux-x86_64-musl   macos-x86_64   macos-arm64
#
# This inspects the produced binary; it does not run it.
set -euo pipefail

die() {
    echo "check-binary: $*" >&2
    exit 1
}

name="${1:?usage: check-binary.sh <target-name> <binary>}"
path="${2:?usage: check-binary.sh <target-name> <binary>}"

[[ -f "$path" ]] || die "binary not found: $path"
[[ -x "$path" ]] || die "binary is not executable: $path"

command -v file >/dev/null 2>&1 || die "the \`file\` tool is required"
info="$(file -b "$path")"

case "$name" in
    linux-x86_64-gnu)
        [[ "$info" == *"ELF 64-bit"* ]] || die "not a 64-bit ELF: $info"
        [[ "$info" == *"x86-64"* ]] || die "not x86-64: $info"
        [[ "$info" == *"dynamically linked"* ]] || die "gnu build is not dynamically linked (glibc): $info"
        ;;
    linux-x86_64-musl)
        [[ "$info" == *"ELF 64-bit"* ]] || die "not a 64-bit ELF: $info"
        [[ "$info" == *"x86-64"* ]] || die "not x86-64: $info"
        if [[ "$info" != *"statically linked"* && "$info" != *"static-pie linked"* ]]; then
            die "musl build is not statically linked: $info"
        fi
        ;;
    macos-x86_64)
        [[ "$info" == *"Mach-O 64-bit"* ]] || die "not a 64-bit Mach-O: $info"
        [[ "$info" == *"x86_64"* ]] || die "not x86_64: $info"
        [[ "$info" != *"arm64"* ]] || die "x86_64 target produced an arm64 binary: $info"
        ;;
    macos-arm64)
        [[ "$info" == *"Mach-O 64-bit"* ]] || die "not a 64-bit Mach-O: $info"
        [[ "$info" == *"arm64"* ]] || die "not arm64: $info"
        ;;
    *)
        die "unknown target name: $name"
        ;;
esac

echo "check-binary: OK $name -> $info"
