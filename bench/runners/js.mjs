// Times one implementation on one file, in its own process (so JIT state, heap and peak
// RSS are its own). Run with --expose-gc.
// Usage: node --expose-gc js.mjs <impl> <file> <profile> <minTimeMs>
// Prints {"importMs","firstCallMs","batch","samples":[ms per call...],"rssMb","checksum"}.

import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'

const require = createRequire(import.meta.url)
const [impl, file, profile, minTimeArg] = process.argv.slice(2)
const MIN_TIME = Number(minTimeArg || 2000)
const MIN_SAMPLES = 10
const MAX_SAMPLES = 500
const MIN_SAMPLE_MS = 5 // tiny inputs are timed in batches of calls of at least this long
const bench = path.join(import.meta.dirname, '..')
const now = () => Number(process.hrtime.bigint()) / 1e6

// Each implementation's equivalent of "load every document with this profile".
// `default`: the library's own defaults. `yaml11-merge`: YAML 1.1 types and `<<` merges.
const IMPLS = {
  'js-yaml 4.3.2': () => {
    const y = require('js-yaml-4')
    // js-yaml 4's default schema already has YAML 1.1 types and merges.
    return (s) => y.loadAll(s)
  },
  'js-yaml 5.5.0': () => {
    const y = require('js-yaml')
    const options = profile === 'yaml11-merge' ? { schema: y.YAML11_SCHEMA, maxTotalMergeKeys: -1 } : {}
    return (s) => y.loadAll(s, options)
  },
  'yaml 2.9.1': () => {
    const { parseAllDocuments } = require('yaml')
    const options = profile === 'yaml11-merge' ? { version: '1.1', merge: true } : {}
    return (s) => parseAllDocuments(s, options).map((d) => {
      if (d.errors.length) throw d.errors[0]
      return d.toJS({ maxAliasCount: -1 })
    })
  },
  'jsyaml-rs napi': () => jsyamlRs(path.join(bench, 'pkg-node')),
  'jsyaml-rs wasm': () => jsyamlRs(path.join(bench, 'pkg-wasm')),
}

function jsyamlRs (dir) {
  const m = require(dir)
  const options = profile === 'yaml11-merge' ? { schema: m.YAML11_SCHEMA, maxTotalMergeKeys: -1 } : {}
  return (s) => m.loadAll(s, options)
}

const input = fs.readFileSync(file, 'utf8')

let t = now()
const load = IMPLS[impl]()
const importMs = now() - t

t = now()
let result = load(input)
const firstCallMs = now() - t
const checksum = checksumOf(result)

// Warm up for ~10% of the budget, and pick a batch size for tiny inputs.
let warm = 0
let calls = 0
while (warm < MIN_TIME / 10 || calls < 3) {
  t = now()
  result = load(input)
  warm += now() - t
  calls++
  if (warm > MIN_TIME) break
}
const perCall = warm / calls
const batch = Math.max(1, Math.ceil(MIN_SAMPLE_MS / Math.max(perCall, 1e-3)))

const samples = []
let total = 0
while ((total < MIN_TIME || samples.length < MIN_SAMPLES) && samples.length < MAX_SAMPLES) {
  result = null
  globalThis.gc?.() // same treatment for every implementation, outside the timed region
  t = now()
  for (let i = 0; i < batch; i++) result = load(input)
  const ms = now() - t
  samples.push(ms / batch)
  total += ms
  // Very slow cases: stop after 5 samples once well over budget (5, so one stalled
  // sample can't move the median or the interquartile range much).
  if (samples.length >= 5 && total > MIN_TIME * 3) break
}

console.log(JSON.stringify({
  importMs,
  firstCallMs,
  batch,
  samples,
  rssMb: process.resourceUsage().maxRSS / 1024,
  checksum
}))

// Order-sensitive digest of the loaded value, with Maps, Sets, Dates and binaries.
function checksumOf (value) {
  try {
    const text = JSON.stringify(value, function (key, v) {
      const raw = this[key]
      if (raw instanceof Date) return { $date: raw.getTime() }
      if (v instanceof Map) return { $map: [...v] }
      if (v instanceof Set) return { $set: [...v] }
      if (v instanceof Uint8Array) return { $binary: [...v] }
      if (typeof v === 'number' && !Number.isFinite(v)) return { $number: String(v) }
      return v
    })
    let h = 0x811c9dc5
    for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 0x01000193) >>> 0
    return h.toString(16)
  } catch {
    return 'n/a (cyclic)'
  }
}
