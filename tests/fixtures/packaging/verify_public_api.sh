#!/usr/bin/env bash
# Regenerate the public API inventory and fail on unreviewed drift.
#
# Two surfaces are tracked, because they are two different published APIs: the
# base model a `default-features = false` consumer sees, and the full surface
# with every feature on. An item that moves between them is an API change even
# when neither snapshot's item count changes.
#
# There is no `cargo semver-checks` here on purpose. Semver comparison needs a
# published baseline to compare against, and candid-core has none until
# 0.1.0-beta.1 exists on crates.io; see docs/releasing.md.
#
# Usage:
#   verify_public_api.sh            # compare against the committed snapshots
#   verify_public_api.sh --write    # accept the current API as the new baseline
#
# `--write` is the reviewed-drift path: regenerate, read the diff, and commit it
# with the change that caused it.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${here}/../../.." && pwd)"

# shellcheck source=./release-tools.env
. "${here}/release-tools.env"

write=0
if [[ ${1-} == "--write" ]]; then
    write=1
elif [[ $# -gt 0 ]]; then
    echo "usage: $(basename "$0") [--write]" >&2
    exit 2
fi

if ! rustup toolchain list | grep -q "^${PUBLIC_API_TOOLCHAIN}"; then
    echo "missing pinned toolchain ${PUBLIC_API_TOOLCHAIN}" >&2
    echo "install it with: rustup toolchain install ${PUBLIC_API_TOOLCHAIN}" >&2
    exit 1
fi

installed_version="$(cargo public-api --version 2>/dev/null | awk '{print $2}' || true)"
if [[ "${installed_version}" != "${CARGO_PUBLIC_API_VERSION}" ]]; then
    echo "cargo-public-api ${CARGO_PUBLIC_API_VERSION} is required, found '${installed_version:-none}'" >&2
    echo "install it with: cargo install cargo-public-api --version ${CARGO_PUBLIC_API_VERSION} --locked" >&2
    exit 1
fi

# `-s` omits blanket implementations, which are noise from dependency traits and
# not part of this crate's API. Auto-trait impls (`Send`, `Sync`, `Unpin`) and
# derived impls (`Clone`, `Debug`) are deliberately kept: losing one of those is
# a breaking change and has to show up in the diff.
generate() {
    local features_flag="$1" destination="$2"
    ( cd "${repo_root}" \
      && cargo "+${PUBLIC_API_TOOLCHAIN}" public-api --color never -s "${features_flag}" ) \
        >"${destination}"
}

status=0
for surface in "base:--no-default-features" "all-features:--all-features"; do
    name="${surface%%:*}"
    flag="${surface#*:}"
    committed="${here}/public-api-${name}.txt"
    generated="$(mktemp)"

    generate "${flag}" "${generated}"

    if (( write )); then
        mv "${generated}" "${committed}"
        echo "wrote ${committed#"${repo_root}"/} ($(wc -l <"${committed}" | tr -d ' ') items)"
        continue
    fi

    if [[ ! -f "${committed}" ]]; then
        echo "no committed snapshot at ${committed#"${repo_root}"/}; run with --write" >&2
        rm -f "${generated}"
        status=1
        continue
    fi

    if diff -u "${committed}" "${generated}" >"${generated}.diff"; then
        echo "public API unchanged for the ${name} surface ($(wc -l <"${committed}" | tr -d ' ') items)"
    else
        echo "public API DRIFT on the ${name} surface:" >&2
        sed -e "s#${generated}#generated#" -e "s#${committed}#committed#" "${generated}.diff" >&2
        echo >&2
        echo "If the change is intended, re-run with --write and commit the snapshot." >&2
        status=1
    fi
    rm -f "${generated}" "${generated}.diff"
done

exit "${status}"
