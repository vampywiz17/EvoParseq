#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
version=0.10.4
checksum=3195d112344906f1ef5f634a7e1840d864044f9674ed62fdfd720ff2bbfc8bcb
relative="purecrypto-$version"
destination="$root/vendor/$relative"
patch="$root/patches/$relative-armv7.patch"
patch_hash="$(sha256sum "$patch" | cut -d ' ' -f 1)"

if [[ -e "$destination" ]]; then
    if [[ -f "$destination/.evoparseq-patch" ]] &&
        [[ "$(cat "$destination/.evoparseq-patch")" == "$patch_hash" ]]; then
        exit 0
    fi
    echo "Existing Android purecrypto source has a different patch. Move $destination aside and run again." >&2
    exit 1
fi

mkdir -p "$root/vendor"
staging="$(mktemp -d "$root/vendor/.purecrypto.XXXXXXXX")"
trap 'rm -rf -- "$staging"' EXIT
archive="$staging/$relative.crate"
curl --fail --silent --show-error --location --retry 3 --proto '=https' \
    --output "$archive" "https://static.crates.io/crates/purecrypto/$relative.crate"
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check
tar -xzf "$archive" -C "$staging"

# Run outside the parent checkout so git applies paths relative to this crate.
(
    cd "$staging/$relative"
    GIT_CEILING_DIRECTORIES="$staging" git apply --check "$patch"
    GIT_CEILING_DIRECTORIES="$staging" git apply "$patch"
)
printf '%s\n' "$patch_hash" > "$staging/$relative/.evoparseq-patch"
mv -T -- "$staging/$relative" "$destination"
