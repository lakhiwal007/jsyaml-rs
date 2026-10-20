# Benchmarks

`just bench-compare` builds both JS packages and the native runner, downloads the
corpus, generates the synthetic inputs and writes [`RESULTS.md`](RESULTS.md) plus every
raw sample in `results.json`.

## What is compared

| Implementation | What it is |
|---|---|
| js-yaml 4.3.2 | The js-yaml most projects run (by far the most downloaded version). Its default schema also resolves timestamps and `<<`, so its output can differ from 5.x. |
| js-yaml 5.5.0 | The version jsyaml-rs ports; outputs must be identical. |
| yaml 2.9.1 | The other popular pure-JS loader (`parseAllDocuments` + `toJS`). |
| jsyaml-rs wasm | The WebAssembly package, in Node. |
| jsyaml-rs napi | The native addon: what users of `jsyaml-node` get. |
| Rust crate alone | `jsyaml::load_all` into Rust values. No JS caller gets this; it is the ceiling, shown to make the cost of crossing into JS visible. |

## Inputs

`corpus.json` lists them. Real-world files are downloaded from their projects, pinned to a
commit or tag and checked by SHA-256 (`fetch-corpus.mjs`): an OpenAPI description
(10 MB), Kubernetes' OpenAPI spec (JSON, 4 MB), a CRD, a pnpm lockfile, Kubernetes
manifests, CI and compose files down to 0.8 KB. js-yaml's own benchmark samples are
included as chosen by js-yaml's authors. The synthetic files from `gen-data.mjs` were
written for this project and are reported separately: treat them with suspicion.

## Method

- Every (implementation, file) pair runs in a fresh Node process.
- Each run imports the library, times the first call, warms up, then samples until
  `MIN_TIME` (2 s) and at least 10 samples; very slow cases stop after 5 samples.
- `gc()` runs before every sample, outside the timed region, for every implementation.
- Inputs that take under 5 ms are timed in batches of calls, so the timer's resolution
  doesn't dominate; the reported time is per call.
- `ROUNDS` (3) rounds run the implementations in a new shuffled order per file, and their
  samples are pooled.
- The timed call is the public API (`loadAll`), so string transfer into native code and
  building the JS result are included.
- Reported: median and interquartile range per call; speedups are ratios of medians, and
  `≈` marks results whose interquartile ranges overlap the baseline's.
- Every implementation's output is checksummed and compared with js-yaml 5.5.0's.
- `NATIVE_MIN_LENGTH` (the small-input shortcut to js-yaml in `decode.cjs`) is 0, so small
  files measure the native path, not js-yaml.

- `just bench-compare` runs under `caffeinate` on macOS: an idle machine that sleeps or
  throttles produced single samples of 300+ s in an earlier run.

## Limits

- One machine and one Node version per run; laptops throttle. Re-run on your hardware.
- Peak RSS is the whole process's, including Node itself.
- Startup costs (import, first call) are shown separately; they dominate short-lived CLIs.
