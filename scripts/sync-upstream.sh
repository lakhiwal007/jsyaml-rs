#!/usr/bin/env bash
# Fetches js-yaml (source + published npm build) and the yaml-test-suite commit it pins,
# then regenerates conformance fixtures.
# Usage: scripts/sync-upstream.sh [version]   (default: contents of conformance/UPSTREAM_VERSION)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
version="${1:-$(cat "$root/conformance/UPSTREAM_VERSION")}"
src="$root/vendor/js-yaml-$version"
npm_dir="$root/vendor/js-yaml-$version-npm"
suite="$root/vendor/yaml-test-suite"
mkdir -p "$root/vendor"

# Source tree: tests, benchmark samples and the pinned yaml-test-suite commit.
if [ ! -d "$src" ]; then
  git clone --quiet --depth 1 --branch "$version" https://github.com/nodeca/js-yaml.git "$src"
fi

# The published package: the reference implementation the fixtures are recorded from.
if [ ! -d "$npm_dir" ]; then
  tmp="$(mktemp -d)"
  (cd "$tmp" && npm pack --silent "js-yaml@$version" >/dev/null)
  mkdir -p "$npm_dir" && tar -xzf "$tmp"/js-yaml-*.tgz -C "$npm_dir" --strip-components 1
  rm -rf "$tmp"
fi

commit="$(tr -d '[:space:]' < "$src/test/spec/yaml-test-suite.commit")"
if [ "$(git -C "$suite" rev-parse HEAD 2>/dev/null || true)" != "$commit" ]; then
  rm -rf "$suite" && mkdir -p "$suite"
  git -C "$suite" init --quiet
  git -C "$suite" fetch --quiet --depth 1 https://github.com/yaml/yaml-test-suite.git "$commit"
  git -C "$suite" checkout --quiet FETCH_HEAD
fi

node "$root/scripts/extract-upstream-fixtures.mjs" "$npm_dir" "$src" "$suite"
echo "$version" > "$root/conformance/UPSTREAM_VERSION"
echo "Fixtures regenerated from js-yaml $version. Run: just conformance"
