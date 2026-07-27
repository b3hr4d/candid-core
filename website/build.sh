#!/usr/bin/env sh
# Builds the browser bridge and drops it next to the page that loads it.
#
# One cargo invocation and one copy. There is no bundler, no npm install, and no
# binding generator: `website/wasm` exports a raw C ABI and `app/js/candid-core.js`
# is the hand-written glue for it.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

cargo build \
  --manifest-path "$root/wasm/Cargo.toml" \
  --target wasm32-unknown-unknown \
  --release

cp "$root/wasm/target/wasm32-unknown-unknown/release/candid_core_web.wasm" \
  "$root/app/candid_core_web.wasm"

size=$(wc -c <"$root/app/candid_core_web.wasm")
echo "built website/app/candid_core_web.wasm (${size} bytes)"
echo "serve it with:  python3 -m http.server --directory $root/app 8080"
