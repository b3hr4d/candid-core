#!/usr/bin/env bash
# Build external consumers against the *packaged* crate, never against this
# repository.
#
# This is the check a repository-relative test cannot make. Every consumer built
# here points at a `.crate` archive unpacked into a fresh temporary directory,
# so anything the archive omits — a source file, the manifest text
# `ProducerInfo::current` reads at compile time, a module reached only through
# `#[path]` — fails here rather than in a downstream user's build. The CLI smoke
# likewise compiles a `.did` file this script writes itself, because every
# fixture under `tests/` is deliberately outside the archive.
#
# Usage: verify_packaged_consumers.sh [path/to/candid-core-<version>.crate]
#
# With no argument the sole archive in `target/package/` is used. Only cargo,
# tar, mktemp, python3, and a SHA-256 utility are invoked; nothing is downloaded
# beyond the ordinary crates.io resolution cargo performs for the generated
# consumer crates.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"

if [[ $# -ge 1 ]]; then
    crate_archive="$1"
else
    shopt -s nullglob
    candidates=("${repo_root}/target/package"/candid-core-*.crate)
    shopt -u nullglob
    if [[ ${#candidates[@]} -ne 1 ]]; then
        echo "expected exactly one candid-core-*.crate in target/package, found ${#candidates[@]}" >&2
        echo "run 'cargo package --locked' first, or pass the archive path explicitly" >&2
        exit 1
    fi
    crate_archive="${candidates[0]}"
fi

[[ -f "${crate_archive}" ]] || { echo "no such archive: ${crate_archive}" >&2; exit 1; }

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT

# One shared target directory for every consumer, so the dependency graph is
# resolved and compiled once instead of six times. It lives in the temporary
# directory, never in the repository's own target directory.
export CARGO_TARGET_DIR="${workdir}/target"

archive_sha256="$(sha256_of "${crate_archive}")"
archive_bytes="$(wc -c < "${crate_archive}" | tr -d ' ')"

echo "archive:  ${crate_archive}"
echo "bytes:    ${archive_bytes}"
echo "sha256:   ${archive_sha256}"

tar -xzf "${crate_archive}" -C "${workdir}"
package_dir="$(find "${workdir}" -maxdepth 1 -type d -name 'candid-core-*' -print -quit)"
[[ -n "${package_dir}" ]] || { echo "archive did not unpack to a candid-core-* directory" >&2; exit 1; }
echo "unpacked: ${package_dir}"

# A consumer that resolved back to the repository would silently defeat this
# whole script, so assert the unpacked package is genuinely outside it.
case "${package_dir}" in
    "${repo_root}"/*) echo "refusing to run: unpacked package is inside the repository" >&2; exit 1 ;;
esac

consumer_number=0
last_consumer_dir=""

# Generate a consumer crate that depends on the unpacked package by path and
# compile it. $1 is a label, $2 the dependency's feature spec, $3 the body of
# `main.rs`, and any remaining arguments are passed to cargo.
build_consumer() {
    local label="$1" feature_spec="$2" body="$3"
    shift 3
    consumer_number=$((consumer_number + 1))
    local dir="${workdir}/consumer-${consumer_number}-${label}"
    mkdir -p "${dir}/src"

    cat >"${dir}/Cargo.toml" <<EOF
[package]
name = "consumer-${label}"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
candid-core = { path = "${package_dir}", ${feature_spec} }

[workspace]
EOF
    printf '%s\n' "${body}" >"${dir}/src/main.rs"

    echo "--- consumer ${label} (${feature_spec}) ---"
    ( cd "${dir}" && cargo "$@" )
    last_consumer_dir="${dir}"
}

# --- 1. base: `default-features = false`, no Candid engine at all -----------
build_consumer base 'default-features = false' '
use candid_core::{
    artifact_id_with_limits, ArtifactKind, ContractDraft, Declaration, Limits, PrimitiveType,
    ProducerInfo, TypeNode,
};

fn main() {
    let contract = ContractDraft::new(
        vec![TypeNode::Primitive { primitive: PrimitiveType::Nat }],
        vec![Declaration { name: "Amount".to_string(), ty: 0 }],
        None,
    )
    .build_with_limits(&Limits::default())
    .expect("the base surface must build a Contract");
    assert!(contract.contract_id().starts_with("candid-core:contract:v1:sha256:"));

    let json = contract.to_json_pretty().expect("canonical serialization");
    let id = artifact_id_with_limits(
        ArtifactKind::ContractJsonV1,
        json.as_bytes(),
        &Limits::default(),
    )
    .expect("detached artifact identity");
    assert!(id.starts_with("candid-core:artifact:contract-json:v1:sha256:"));

    // The packaged manifest is what `ProducerInfo::current` reads at compile
    // time. If `include` dropped it, or Cargo normalized it into a shape the
    // reader does not understand, this build panics or these are empty.
    let producer = ProducerInfo::current();
    assert_eq!(producer.name, "candid-core");
    assert!(!producer.version.is_empty());
    assert!(!producer.candid_version.is_empty());
    assert!(!producer.candid_parser_version.is_empty());
    println!("base ok: {} {}", producer.name, producer.version);
}
' run --quiet

# The *published* manifest has to preserve the feature boundary, not just the
# repository manifest. `cargo tree` over the base consumer is the proof.
echo "--- base consumer dependency boundary ---"
base_tree="$(cd "${last_consumer_dir}" && cargo tree --edges normal,build --prefix none)"
for forbidden in candid candid_parser cap-std ic_principal; do
    if grep -Eq "^${forbidden} v" <<<"${base_tree}"; then
        echo "base consumer graph must not contain ${forbidden}" >&2
        exit 1
    fi
done
echo "base consumer graph excludes candid, candid_parser, cap-std, ic_principal"

# --- 2. compiler only: source compilation with no filesystem ----------------
build_consumer compiler 'default-features = false, features = ["compiler"]' '
use candid_core::{compile_did, compile_with_resolver, CompileOptions, MemoryResolver, RuntimeContext};

fn main() {
    let compiled = compile_did("service : { ping : () -> (text) query };")
        .expect("the compiler surface must compile a self-contained source");
    assert!(compiled.contract().interface_id().is_some());

    let mut resolver = MemoryResolver::new();
    resolver
        .insert("api/root.did", r#"import "types.did"; service : { read : () -> (Item) query };"#)
        .expect("insert the entry source");
    resolver
        .insert("api/types.did", "type Item = record { id : nat64; label : text };")
        .expect("insert the imported source");
    let bundle = compile_with_resolver(
        "api/root.did",
        &resolver,
        CompileOptions::default(),
        &RuntimeContext::default(),
    )
    .expect("imported compilation must work with no filesystem");
    assert!(bundle.source_info().is_some());
    println!("compiler ok: {}", compiled.contract().contract_id());
}
' run --quiet

# --- 3. the full native library, every feature on ---------------------------
build_consumer all-features 'features = ["compiler", "filesystem-compiler", "host-value"]' '
use candid_core::{compile_did, validate_host_value, HostValue, Limits};

fn main() {
    let compilation = compile_did("type Greeting = text; service : { echo : (Greeting) -> (Greeting) query };")
        .expect("compile");
    let contract = compilation.contract();
    let declaration = contract
        .declarations()
        .iter()
        .find(|declaration| declaration.name == "Greeting")
        .expect("the Greeting declaration");
    let selector = contract.bind_type(declaration.ty).expect("bind the declared type");
    validate_host_value(contract, &selector, &HostValue::text("hello"), &Limits::default())
        .expect("host value validation");
    println!("all-features ok: {}", contract.contract_id());
}
' run --quiet

# --- 4. the CLI, installed from the package, on a source it writes itself ---
echo "--- CLI install and smoke ---"
cli_root="${workdir}/cli"
cargo install --locked --path "${package_dir}" --root "${cli_root}" \
    --no-default-features --features filesystem-compiler
cli="${cli_root}/bin/candid-core"
[[ -x "${cli}" ]] || { echo "cargo install produced no candid-core binary" >&2; exit 1; }

smoke="${workdir}/smoke"
mkdir -p "${smoke}"
cat >"${smoke}/service.did" <<'DID'
type Item = record { id : nat; label : text };
service : {
  get : (nat) -> (Item) query;
}
DID

"${cli}" compile "${smoke}/service.did" >"${smoke}/compiled.json"
python3 - "${smoke}/compiled.json" "${smoke}/contract.json" <<'PY'
import json
import sys

document = json.load(open(sys.argv[1]))
assert document["ok"] is True, document
contract = document["contract"]
identities = contract["identities"]
assert identities["contract"].startswith("candid-core:contract:v1:sha256:"), identities
assert identities["interface"].startswith("candid-core:interface:v1:sha256:"), identities
assert document["source_info"] is not None, "compile must emit the provenance sidecar"
json.dump(contract, open(sys.argv[2], "w"))
print("CLI compile produced", identities["contract"])
PY
"${cli}" validate "${smoke}/contract.json" >/dev/null
echo "CLI compile + validate ok"

# --- 5. wasm32-unknown-unknown, the two surfaces a browser host builds ------
if ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    echo "wasm32-unknown-unknown is not installed; run 'rustup target add wasm32-unknown-unknown'" >&2
    exit 1
fi

build_consumer wasm-base 'default-features = false' '
use candid_core::{ContractDraft, Declaration, Limits, PrimitiveType, TypeNode};

fn main() {
    let contract = ContractDraft::new(
        vec![TypeNode::Primitive { primitive: PrimitiveType::Nat }],
        vec![Declaration { name: "Amount".to_string(), ty: 0 }],
        None,
    )
    .build_with_limits(&Limits::default())
    .expect("the base surface must build on wasm");
    let _ = contract.contract_id();
}
' check --quiet --target wasm32-unknown-unknown

build_consumer wasm-compiler 'default-features = false, features = ["compiler"]' '
use candid_core::compile_did;

fn main() {
    let compiled = compile_did("service : { ping : () -> () };").expect("the compiler surface on wasm");
    let _ = compiled.contract().contract_id();
}
' check --quiet --target wasm32-unknown-unknown

echo
echo "every packaged-consumer surface built from ${crate_archive}"
echo "sha256 ${archive_sha256}"
