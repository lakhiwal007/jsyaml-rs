# Contributing

## Porting workflow

1. Pick a function in js-yaml (`vendor/js-yaml-<version>/src`, fetched by
   `just sync-upstream`) and port it into the matching module (see
   [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)). Keep upstream's names and order, so
   later upstream releases can be diffed and applied.
2. `just conformance` shows the scoreboard; `CONFORMANCE_REPORT=1` lists each failure
   with expected vs actual.
3. When cases start passing, `just conformance-bless` adds them to
   `conformance/passing.txt`. Commit the longer list; it must never shrink.
4. `just js-test` builds both npm packages and runs, against each: a randomised
   differential of `parseEvents` (mutated fixture inputs vs js-yaml, events and error
   messages), the `load` fixture differential and js-yaml's own test suite.

Tools: `cargo run --release -p jsyaml --example parse_time -- <files>` times the parser;
`examples/stack_probe.rs` measures stack use per nesting level (re-check
`HARD_DEPTH_LIMIT` after changing parser functions' locals).

Done: `parser_scalar.ts`, `parser.ts`. Next: scalar tag resolvers in `src/tag/scalar`
→ `constructor.ts` (the `load::` cases) → YAML 1.1 tags, merge, aliases.

## Rules

- Error `reason`s and positions must match upstream exactly; JS builds the message.
- No panics on any input: the fuzz target (`just fuzz`) checks this.
- No `unsafe` (the workspace forbids it).
- Fixtures are generated: never edit `conformance/fixtures/*.json`. To add a case, add it
  to `conformance/custom-cases.json` and run `just sync-upstream`.
