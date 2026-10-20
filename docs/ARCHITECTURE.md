# Architecture

## Upstream map

| js-yaml 5.5 | Lines | Port | Status |
|---|---:|---|---|
| `src/parser/parser.ts` | 1495 | `crates/jsyaml/src/parser/mod.rs` | done: 451/451 fixtures, 200k+ random inputs match |
| `src/parser/parser_scalar.ts` | 311 | `crates/jsyaml/src/parser/scalar.rs` | done (lone surrogate escapes → U+FFFD) |
| `src/parser/events.ts` | 150 | `crates/jsyaml/src/event.rs` | done |
| `src/parser/constructor.ts` | 489 | `crates/jsyaml/src/construct/mod.rs` | done |
| `src/tag/**` | ~900 | `crates/jsyaml/src/construct/{scalars,containers}.rs` | done |
| `src/schema.ts` | 360 | `crates/jsyaml/src/schema.rs` | done |
| `src/load.ts` | 118 | `crates/jsyaml/src/lib.rs`, `crates/jsyaml-tape/js/decode.cjs` | done |
| `src/common/exception.ts`, `snippet.ts` | 200 | not ported: JS uses upstream's `YAMLException` | — |
| `src/dump.ts`, `src/ast/**` | ~1700 | not ported: re-exported from js-yaml | — |

`dump` stays in JS on purpose. A Rust `dump` has to read a JS object graph through
N-API one property at a time, which is the slow direction; benchmark before porting it.

## Constructor

- The full constructor (`construct/mod.rs`) mirrors upstream's frame stack. JS object
  identity (aliases, `Map`/`Set` keys, merge sources) is modelled with an arena of nodes
  addressed by id; a finished document becomes a `Value` tree, and nodes reached more
  than once go to `Document::shared`. Each document's arena is freed when it ends.
- The fast path (`construct/fast.rs`) writes JSON directly from the events for plain
  data. It bails out (to the full constructor, on the same events) on aliases, merges,
  non-JSON values, collection keys and every error, so it never decides an outcome.
- Inputs Rust can't reproduce exactly return `Error::is_unsupported`; the bindings then
  build the result with upstream's own `constructFromEvents` from the native events.
  That covers lone-surrogate escapes and `legacyMapTag` keys stringified through
  `Date.prototype.toString`. Input strings that contain lone surrogates themselves go to
  js-yaml unchanged (they can't cross into Rust as UTF-8).

## Known differences from upstream

- None in what `jsyaml-node` / `jsyaml-wasm` return (see the fallbacks above). The Rust
  API reports the two unsupported cases as errors, and `get_scalar_value` decodes lone
  surrogate escapes to U+FFFD.
- Nesting deeper than `HARD_DEPTH_LIMIT` (1000) fails even with a larger `maxDepth`,
  so a hostile input can't overflow the native stack and abort Node. Upstream throws a
  `RangeError` somewhere past ~3000 levels instead.

## Offsets

Rust works in byte offsets. Everything that crosses into JS (event records, error
positions) is converted to UTF-16 code units with `jsyaml::utf16::Utf16Cursor`, which is
free for ASCII input and incremental otherwise.

## Transports (`crates/jsyaml-tape`)

Building JS values from Rust one N-API call at a time is what makes most native addons
slower than expected (papaparse-rs: 6× faster natively, 1.8–3× through napi). So results
cross in bulk:

| Result | Form | Built in JS by |
|---|---|---|
| JSON-safe documents | JSON string | `JSON.parse` |
| Anything else | `ValueTape`: `u32` ops + `f64` numbers + one string + one byte buffer | `decodeValueTape` |
| Events (custom tags) | `EventTape`: 11 `i32`s per event + directives JSON | `decodeEventTape` → upstream `constructFromEvents` |

Typed arrays are handed over without copying (`TypedArray::new(Vec)` in napi-rs). Strings
inside the value tape are `substring`s of one big string.

Aliased objects keep their JS identity: the constructor stores each aliased collection,
`Date` or `Uint8Array` once in `Document::shared`, and the decoder creates each container
before decoding its children so cycles (`&a [*a]`) resolve.

## Schema routing (`decode.cjs`)

`describeSchema` maps each tag object in `options.schema.tags` to its js-yaml export name
(`strTag`, `mergeTag`, ...). If all are built-in, Rust gets the names and does
everything; if any is a user-defined tag, the events path is used. Schema objects are
compared by identity, so the ESM entry (`index.mjs`) wraps js-yaml's ESM build and the
CJS entry its CJS build.

## Small inputs

`NATIVE_MIN_LENGTH` in `decode.cjs` sends short inputs straight to js-yaml, where the
crossing costs more than it saves. It is 0 until benchmarks pick a threshold.

## Distribution

Same layout as lightningcss and oxc-parser. `scripts/package-npm.mjs` assembles, from the
per-target binaries the release workflow builds on native runners:

- `jsyaml-node-<platform>` for each entry of `scripts/platforms.json` (macOS arm64/x64,
  Linux x64/arm64 for glibc and musl, Windows x64/arm64): just `jsyaml.node`, restricted
  with `os` / `cpu` / `libc` so npm installs only the matching one;
- `jsyaml-node`: the JS entry points, `optionalDependencies` on every platform package,
  and a dependency on the exact js-yaml version the port matches. `binding.cjs` loads a
  local build or the platform package, and otherwise fails with a message pointing at
  `jsyaml-wasm`;
- `jsyaml-wasm`: the WebAssembly build, for any other platform and for browsers.

The workflow smoke-tests the Linux package and publishes the platform packages before the
main one. Locally, `node scripts/package-npm.mjs --artifacts <dir> --allow-missing` builds
a subset for testing with `npm pack`.

## Native memory

The addon returns the value and event tapes as one buffer copied into Node-owned memory
(`napi_create_buffer_copy`), not as external `ArrayBuffer`s over Rust allocations: those
were never freed (RSS grew ~6 KB per call while V8's heap stayed flat).
`tests/js/memory.cjs` guards against this.
