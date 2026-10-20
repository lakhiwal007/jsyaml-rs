# Changelog

## Unreleased

- Constructor ported (`constructor.ts`, all built-in tags and schemas): all 1824
  conformance cases and js-yaml's own suite (1629 tests) pass on both packages, and
  randomised differentials match js-yaml on 600k+ mutated inputs, including alias
  identity and cycles. Inputs Rust can't reproduce exactly (lone surrogates,
  time-zone-dependent `legacyMapTag` keys) fall back to js-yaml's own constructor.
- JSON fast path for plain data; ASCII fast path for the non-printable check.
- Fixed: the napi addon leaked every value / event tape (external `ArrayBuffer`s);
  tapes are now copied into Node-owned memory. Added `tests/js/memory.cjs`.
- Fixed: the decoder misplaced the root when a shared `Date` or `Uint8Array` came first.
- npm packaging per platform (`scripts/package-npm.mjs`, release workflow).
- Benchmarks rewritten: pinned real-world corpus, js-yaml 4 and 5, fresh process per
  run, shuffled rounds, median and IQR, output checksums.
- Builds with Rust 1.85 (workspace dependency features fixed for Cargo < 1.88).

- Parser ported (`parser.ts`, `parser_scalar.ts`): all 451 `events::` fixtures pass,
  including decoded scalar text, and a randomised differential against js-yaml
  (`tests/js/differential-events.cjs`) matches on 200k mutated inputs. Nesting is capped
  at 1000 levels to keep the native stack safe. Scalar events' `indent` is now `i32`.

- Workspace scaffold: core crate types (events, schema, values, errors, UTF-16 offsets),
  JSON / value / event transports, napi-rs and wasm bindings with a js-yaml-compatible
  API, CLI, conformance fixtures recorded from js-yaml 5.5.0, benchmarks.
