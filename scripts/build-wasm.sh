#!/usr/bin/env bash
# Builds the npm package for Node into crates/jsyaml-wasm/pkg (or $1).
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen-cli matching Cargo.lock, wasm-opt (binaryen).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/crates/jsyaml-wasm/pkg}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"

cargo build -p jsyaml-wasm --target wasm32-unknown-unknown --profile wasm-release
rm -rf "$out"
wasm-bindgen "$target_dir/wasm32-unknown-unknown/wasm-release/jsyaml_wasm.wasm" \
  --target nodejs --out-dir "$out" --no-typescript
wasm-opt -Oz --strip-debug --strip-producers \
  --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
  "$out/jsyaml_wasm_bg.wasm" -o "$out/jsyaml_wasm_bg.wasm"

cp "$root/crates/jsyaml-wasm/js/index.cjs" "$root/crates/jsyaml-wasm/js/index.mjs" "$root/crates/jsyaml-tape/js/decode.cjs" "$root/crates/jsyaml-tape/js/index.d.ts" "$out/"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
cat > "$out/package.json" <<JSON
{
  "name": "jsyaml-wasm",
  "version": "$version",
  "description": "Drop-in js-yaml with a YAML loader compiled from Rust to WebAssembly",
  "license": "MIT",
  "type": "commonjs",
  "main": "index.cjs",
  "types": "index.d.ts",
  "exports": {
    ".": { "types": "./index.d.ts", "import": "./index.mjs", "require": "./index.cjs" }
  },
  "files": ["index.cjs", "index.mjs", "decode.cjs", "index.d.ts", "jsyaml_wasm.js", "jsyaml_wasm_bg.wasm"],
  "dependencies": { "js-yaml": "$(cat "$root/conformance/UPSTREAM_VERSION")" },
  "engines": { "node": ">=18" }
}
JSON
echo "built $out ($(wc -c < "$out/jsyaml_wasm_bg.wasm") bytes wasm, $(gzip -9c "$out/jsyaml_wasm_bg.wasm" | wc -c) gzipped)"
