# Conformance fixtures

`fixtures/*.json` record what js-yaml (the published npm build, version in
`UPSTREAM_VERSION`) does on every input. **Do not edit them by hand**; regenerate with
`just sync-upstream`.

| File | Contents | Runner |
|---|---|---|
| `events.json` | `parseEvents(yaml)`: event objects with UTF-16 offsets, or the error | `crates/jsyaml/tests/conformance.rs` |
| `load.json` | `loadAll(yaml, { schema })` for `core`, `json` and `yaml11`, or the error | `crates/jsyaml/tests/conformance.rs` |

Inputs:

- [yaml-test-suite](https://github.com/yaml/yaml-test-suite) at the commit js-yaml pins
  (`test/spec/yaml-test-suite.commit`), cases marked `skip` excluded;
- js-yaml's `benchmark/samples` up to 64 KiB;
- `custom-cases.json`: hand-written edge cases the suite lacks (aliased and recursive
  collections, `<<` merges, YAML 1.1 types, `-0`, key order, `__proto__`, non-ASCII
  offsets, CRLF, depth limit). Add a case there, then re-run `just sync-upstream`.

Values use a typed encoding (Maps, Sets, Dates, `Uint8Array`, `NaN`, `-0`, shared
objects); see the header of `scripts/extract-upstream-fixtures.mjs`. Errors are compared
by `reason` and UTF-16 `position`; the message and snippet come from upstream's own
`YAMLException` in the JS bindings, so they don't need separate checks.

`passing.txt` is a ratchet: listed cases must keep passing. After porting more of the
parser, run `just conformance-bless` and commit the longer list.

The JS packages are also run against js-yaml's own `node:test` suite
(`just js-test`, see `tests/js/upstream.mjs`), which covers options, custom tags and
error snippets that the fixtures can't.
