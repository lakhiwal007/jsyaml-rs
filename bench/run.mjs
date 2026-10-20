// Runs every implementation on every corpus file and writes RESULTS.md and results.json.
// Prerequisites: `just bench-compare` (builds packages, fetches the corpus, generates the
// synthetic inputs). Environment: ROUNDS (default 3), MIN_TIME ms per run (default 2000),
// ONLY (substring filter on file names).
//
// Methodology, to keep the comparison fair:
// - each (implementation, file) run is a fresh Node process: no shared JIT state or heap;
// - every round runs the implementations in a new random order per file (seeded), and
//   samples from all rounds are pooled, so drift and thermal effects spread evenly;
// - every implementation gets the same treatment: warm-up, gc() before each sample
//   (outside the timed region), batching of tiny inputs to beat timer resolution;
// - what is timed is the public API a user calls (loadAll / parseAllDocuments + toJS),
//   including string transfer into native code and building the JS result;
// - results report median and interquartile range; "≈" marks overlapping ranges;
// - each implementation's output is checksummed against js-yaml 5.5.0's.

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const here = import.meta.dirname
const ROUNDS = Number(process.env.ROUNDS || 3)
const MIN_TIME = Number(process.env.MIN_TIME || 2000)
const ONLY = process.env.ONLY || ''
const BASE4 = 'js-yaml 4.3.2'
const BASE5 = 'js-yaml 5.5.0'
const IMPLS = [BASE4, BASE5, 'yaml 2.9.1', 'jsyaml-rs wasm', 'jsyaml-rs napi']
const NATIVE = 'Rust crate alone'
const nativeBin = path.join(here, '../target/release', process.platform === 'win32' ? 'jsyaml-bench.exe' : 'jsyaml-bench')

const corpus = JSON.parse(fs.readFileSync(path.join(here, 'corpus.json'), 'utf8'))
  .filter((c) => c.file.includes(ONLY))

let seed = 20261020
const rand = (n) => { seed = (seed * 1103515245 + 12345) % 2 ** 31; return seed % n }
const shuffle = (xs) => { const a = [...xs]; for (let i = a.length - 1; i > 0; i--) { const j = rand(i + 1); [a[i], a[j]] = [a[j], a[i]] } return a }
const quantile = (xs, q) => { const s = [...xs].sort((a, b) => a - b); const i = (s.length - 1) * q; const lo = Math.floor(i); return s[lo] + (s[Math.ceil(i)] - s[lo]) * (i - lo) }

function run (impl, file, profile) {
  try {
    const stdout = impl === NATIVE
      ? execFileSync(nativeBin, [file, profile, String(MIN_TIME)], { encoding: 'utf8', maxBuffer: 1 << 26 })
      : execFileSync(process.execPath, ['--expose-gc', path.join(here, 'runners/js.mjs'), impl, file, profile, String(MIN_TIME)], { encoding: 'utf8', maxBuffer: 1 << 26, stdio: ['ignore', 'pipe', 'pipe'] })
    return JSON.parse(stdout)
  } catch (e) {
    const lines = String(e.stderr || e.message).split('\n')
    return { error: lines.find((l) => /^\w*(Error|Exception)\b/.test(l)) ?? lines.find(Boolean) }
  }
}

const results = {}
for (let round = 1; round <= ROUNDS; round++) {
  for (const c of corpus) {
    const file = path.join(here, 'data', c.file)
    for (const impl of shuffle([...IMPLS, NATIVE])) {
      const r = run(impl, file, c.profile)
      const key = `${c.file}\0${impl}`
      const acc = (results[key] ??= { samples: [], importMs: [], firstCallMs: [], rssMb: [], checksums: new Set(), errors: new Set() })
      if (r.error) { acc.errors.add(r.error); continue }
      acc.samples.push(...r.samples)
      if (r.importMs !== undefined) { acc.importMs.push(r.importMs); acc.firstCallMs.push(r.firstCallMs); acc.rssMb.push(r.rssMb) }
      if (r.checksum) acc.checksums.add(r.checksum)
    }
    process.stdout.write(`round ${round}/${ROUNDS}: ${c.file}\n`)
  }
}

const fmt = (ms) => ms >= 1000 ? `${(ms / 1000).toFixed(2)} s` : ms >= 10 ? `${ms.toFixed(0)} ms` : ms >= 0.1 ? `${ms.toFixed(2)} ms` : `${(ms * 1000).toFixed(1)} µs`
const sizeOf = (bytes) => bytes >= 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : `${(bytes / 1e3).toFixed(1)} KB`
const stat = (acc) => acc && acc.samples.length ? { med: quantile(acc.samples, 0.5), p25: quantile(acc.samples, 0.25), p75: quantile(acc.samples, 0.75) } : null
const ratio = (base, s) => {
  if (!base || !s) return 'n/a'
  const x = base.med / s.med
  const overlap = s.p25 <= base.p75 && base.p25 <= s.p75
  return `${overlap ? '≈' : ''}${x.toFixed(2)}×`
}

const raw = []
let table = ''
const speedups = { small: [], medium: [], large: [] } // napi vs js-yaml 5, real-world files only
for (const c of corpus) {
  const bytes = fs.statSync(path.join(here, 'data', c.file)).size
  const base4 = stat(results[`${c.file}\0${BASE4}`])
  const base5 = stat(results[`${c.file}\0${BASE5}`])
  const reference = [...(results[`${c.file}\0${BASE5}`]?.checksums ?? [])][0]
  table += `\n### ${c.file} (${sizeOf(bytes)}, ${c.kind}${c.profile === 'default' ? '' : `, ${c.profile}`})\n\n${c.description}\n\n`
  table += '| Implementation | Median | IQR | Throughput | vs js-yaml 4.3.2 | vs js-yaml 5.5.0 | Peak RSS | Output |\n|---|---:|---:|---:|---:|---:|---:|---|\n'
  for (const impl of [...IMPLS, NATIVE]) {
    const acc = results[`${c.file}\0${impl}`]
    const s = stat(acc)
    raw.push({ file: c.file, bytes, impl, ...s, samples: acc?.samples, importMs: acc?.importMs, firstCallMs: acc?.firstCallMs, rssMb: acc?.rssMb, errors: [...(acc?.errors ?? [])] })
    if (!s) {
      table += `| ${impl} | failed: ${[...acc.errors][0]?.slice(0, 90)} | | | | | | |\n`
      continue
    }
    const sums = [...acc.checksums]
    const check = impl === NATIVE ? '(Rust values)' : sums.length === 1 && sums[0] === reference ? 'same as 5.5.0' : impl === BASE4 ? 'differs (v4 defaults)' : `**differs**`
    const rss = acc.rssMb.length ? `${quantile(acc.rssMb, 0.5).toFixed(0)} MB` : ''
    table += `| ${impl} | ${fmt(s.med)} | ${fmt(s.p25)}–${fmt(s.p75)} | ${(bytes / 1e6 / (s.med / 1000)).toFixed(1)} MB/s | ${ratio(base4, s)} | ${ratio(base5, s)} | ${rss} | ${check} |\n`
    if (impl === 'jsyaml-rs napi' && c.kind === 'real-world' && base5) {
      speedups[bytes < 10e3 ? 'small' : bytes < 1e6 ? 'medium' : 'large'].push(base5.med / s.med)
    }
  }
}

const geomean = (xs) => xs.length ? Math.exp(xs.reduce((a, x) => a + Math.log(x), 0) / xs.length) : NaN
const startupRows = IMPLS.map((impl) => {
  const imports = corpus.flatMap((c) => results[`${c.file}\0${impl}`]?.importMs ?? [])
  const firsts = corpus.filter((c) => c.file.includes('compose')).flatMap((c) => results[`${c.file}\0${impl}`]?.firstCallMs ?? [])
  return `| ${impl} | ${imports.length ? fmt(quantile(imports, 0.5)) : 'n/a'} | ${firsts.length ? fmt(quantile(firsts, 0.5)) : 'n/a'} |`
}).join('\n')

const cpu = os.cpus()[0]?.model ?? 'unknown CPU'
const text = `# Benchmark results

Generated by \`just bench-compare\` on ${new Date().toISOString().slice(0, 10)}. Raw samples: \`results.json\`.

Environment: ${cpu}, ${os.cpus().length} cores, ${(os.totalmem() / 2 ** 30).toFixed(0)} GB RAM, ${process.platform} ${os.release()}, Node ${process.version}.
${ROUNDS} rounds × ≥${MIN_TIME} ms per (implementation, file), each in a fresh process, order shuffled per round. Times are per \`loadAll\` call: median, and the interquartile range of all pooled samples. "≈" means the two ranges overlap (difference within noise).

See [README.md](README.md) for the methodology and what each column means.

## Summary

jsyaml-rs napi vs js-yaml 5.5.0, geometric mean over real-world files:

| File size | Files | Speedup |
|---|---:|---:|
| < 10 KB | ${speedups.small.length} | ${geomean(speedups.small).toFixed(2)}× |
| 10 KB – 1 MB | ${speedups.medium.length} | ${geomean(speedups.medium).toFixed(2)}× |
| > 1 MB | ${speedups.large.length} | ${geomean(speedups.large).toFixed(2)}× |

Startup (median over all runs): importing the module, and the first call on a 0.8 KB file (cold JIT / addon initialisation):

| Implementation | Import | First call (compose.yaml) |
|---|---:|---:|
${startupRows}

## Per file
${table}`
fs.writeFileSync(path.join(here, 'RESULTS.md'), text)
fs.writeFileSync(path.join(here, 'results.json'), JSON.stringify({ environment: { cpu, cores: os.cpus().length, node: process.version, platform: process.platform, release: os.release(), rounds: ROUNDS, minTime: MIN_TIME }, results: raw }, null, 1))
console.log('wrote bench/RESULTS.md and bench/results.json')
