# Developer tasks. Install with `cargo install just`, then run `just <task>`.

default: check

# Format, lint and test — what CI runs.
check: fmt-check lint test js-unit

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-features

# Print the conformance scoreboard with every failure.
conformance:
    CONFORMANCE_REPORT=1 cargo test -p jsyaml --test conformance -- --nocapture

# Record everything that passes now in conformance/passing.txt (review the diff).
conformance-bless:
    CONFORMANCE_BLESS=1 cargo test -p jsyaml --test conformance -- --nocapture

# Fetch js-yaml + yaml-test-suite and regenerate conformance fixtures (needs node, npm, git).
sync-upstream version="":
    scripts/sync-upstream.sh {{version}}

# Unit tests of the shared JS decoder; no Rust build needed.
js-unit:
    npm ci --no-audit --no-fund --silent
    node --test tests/js/decode.test.cjs

# Criterion micro-benchmarks of the Rust crate.
bench:
    cargo bench -p jsyaml

# Build the npm package into crates/jsyaml-wasm/pkg.
# Needs wasm-bindgen-cli (same version as Cargo.lock) and binaryen's wasm-opt.
wasm:
    scripts/build-wasm.sh

# Build the Node-API addon for this platform into crates/jsyaml-node/pkg.
node:
    scripts/build-node.sh

# Run the differentials and js-yaml's own test suite against both packages.
js-test: wasm node js-unit
    for pkg in wasm node; do \
        node tests/js/differential-events.cjs crates/jsyaml-$pkg/pkg 20000 && \
        node tests/js/differential-load.cjs crates/jsyaml-$pkg/pkg 20000 && \
        node tests/js/differential.cjs crates/jsyaml-$pkg/pkg && \
        node --expose-gc tests/js/memory.cjs crates/jsyaml-$pkg/pkg && \
        node tests/js/upstream.mjs crates/jsyaml-$pkg/pkg || exit 1; \
    done

# js-yaml 4 / 5 vs yaml vs jsyaml-rs (napi, wasm, native) on a pinned real-world corpus
# plus synthetic inputs; writes bench/RESULTS.md and bench/results.json. Takes ~1–1.5 h
# (the `yaml` package needs seconds per call on the large files).
# ROUNDS, MIN_TIME (ms) and ONLY (file filter) tune it.
bench-compare: wasm node
    cd bench && npm install --no-audit --no-fund --silent && node fetch-corpus.mjs && node gen-data.mjs
    rm -rf bench/pkg-wasm bench/pkg-node
    cp -R crates/jsyaml-wasm/pkg bench/pkg-wasm && cp -R crates/jsyaml-node/pkg bench/pkg-node
    cargo build --release -p jsyaml-bench
    if command -v caffeinate >/dev/null; then caffeinate -dims node --expose-gc bench/run.mjs; else node --expose-gc bench/run.mjs; fi

# Needs nightly and `cargo install cargo-fuzz`. Targets: load (no panics, valid error
# offsets) and fast_path (the JSON fast path agrees with the full constructor).
fuzz seconds="60":
    node scripts/fuzz-seed.mjs
    cargo +nightly fuzz run load fuzz/corpus/load -- -max_total_time={{seconds}}
    cargo +nightly fuzz run fast_path fuzz/corpus/fast_path -- -max_total_time={{seconds}}

# Needs `cargo install cargo-deny`.
deny:
    cargo deny check

doc:
    cargo doc --workspace --no-deps --open
