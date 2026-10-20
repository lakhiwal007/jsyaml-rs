#!/usr/bin/env bash
# Builds the Node-API addon for the current platform into crates/jsyaml-node/pkg (or $1).
# Publishing per-platform packages is done in CI with @napi-rs/cli (see the release workflow).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/crates/jsyaml-node/pkg}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"

cargo build -p jsyaml-node --profile node-release
case "$(uname -s)" in
  Darwin) lib="libjsyaml_node.dylib" ;;
  Linux) lib="libjsyaml_node.so" ;;
  *) lib="jsyaml_node.dll" ;;
esac

rm -rf "$out" && mkdir -p "$out"
cp "$target_dir/node-release/$lib" "$out/jsyaml.node"
cp "$root/crates/jsyaml-node/js/index.cjs" "$root/crates/jsyaml-node/js/index.mjs" "$root/crates/jsyaml-node/js/binding.cjs" "$root/crates/jsyaml-tape/js/decode.cjs" "$root/crates/jsyaml-tape/js/index.d.ts" "$out/"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
cat > "$out/package.json" <<JSON
{
  "name": "jsyaml-node",
  "version": "$version",
  "description": "Drop-in js-yaml with a native YAML loader built from Rust with NAPI-RS",
  "license": "MIT",
  "type": "commonjs",
  "main": "index.cjs",
  "types": "index.d.ts",
  "exports": {
    ".": { "types": "./index.d.ts", "import": "./index.mjs", "require": "./index.cjs" }
  },
  "dependencies": { "js-yaml": "$(cat "$root/conformance/UPSTREAM_VERSION")" },
  "engines": { "node": ">=18" }
}
JSON
echo "built $out ($(wc -c < "$out/jsyaml.node") bytes addon)"
