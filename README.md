# jsyaml-rs

A port of [js-yaml](https://github.com/nodeca/js-yaml) 5.x to Rust, shipped as a
**drop-in replacement for `js-yaml` in Node.js**. The goal is the same output and the same
errors, faster on large inputs.

> **Status: complete port of `load`, `loadAll` and `parseEvents`.** All 1824
> conformance cases match js-yaml 5.5.0 (events, decoded scalars, values for three
> schemas, error messages), as does js-yaml's own test suite (1629 tests), and
> randomised differential tests over hundreds of thousands of mutated inputs. `dump`
> and everything else is js-yaml's own. Benchmarks: [bench/RESULTS.md](bench/RESULTS.md).

```sh
npm install jsyaml-node   # native addon; jsyaml-wasm for other platforms or browsers
```

```js
// const yaml = require('js-yaml');
const yaml = require('jsyaml-node');

const config = yaml.load(fs.readFileSync('config.yml', 'utf8'));
yaml.dump(config); // everything except load/loadAll/parseEvents is js-yaml's own
```

## Packages

| Crate | Published as | What it is |
|---|---|---|
| `crates/jsyaml` | crates.io `jsyaml` | The Rust loader: `parse_events`, `construct`, `load`, `load_all` |
| `crates/jsyaml-tape` | (internal) | Option mapping and the transports between Rust and JS, plus `js/decode.cjs` |
| `crates/jsyaml-node` | npm `jsyaml-node` | napi-rs addon, the main package |
| `crates/jsyaml-wasm` | npm `jsyaml-wasm` | WebAssembly fallback for platforms without a prebuilt addon, and browsers |
| `crates/jsyaml-cli` | crates.io `jsyaml-cli` | `jsyaml file.yml` → JSON |

## How it fits together

js-yaml 5 already splits loading into a parser (text → events) and a constructor
(events → values). The port follows that split, which gives three paths:

1. **Built-in schemas** (`CORE_SCHEMA`, `JSON_SCHEMA`, `YAML11_SCHEMA`, or `withTags` of
   built-in tags): Rust parses *and* constructs. If the result is plain JSON data, it
   crosses as a JSON string for V8's `JSON.parse`; otherwise (Map, Set, Date,
   Uint8Array, NaN, -0, aliased objects) as a compact value tape.
2. **Schemas with custom JS tags**: Rust parses, and js-yaml's own
   `constructFromEvents` builds the values from an event tape.
3. **Errors**: Rust returns `reason` + offset, and JS throws js-yaml's own
   `YAMLException.throwAt(...)`, so messages, snippets and `instanceof` all match.

Details: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Development

```sh
npm install              # js-yaml for the JS tests
just js-unit             # JS decoder tests (no Rust build needed)
just test                # Rust tests, including the conformance ratchet
just conformance         # scoreboard against js-yaml's recorded behaviour
just sync-upstream       # re-fetch js-yaml + yaml-test-suite, regenerate fixtures
just js-test             # build both packages; run js-yaml's own test suite against them
just bench-compare       # js-yaml vs yaml vs jsyaml-rs → bench/RESULTS.md
```

## License

MIT, including js-yaml's notice (see [LICENSE](LICENSE)).
