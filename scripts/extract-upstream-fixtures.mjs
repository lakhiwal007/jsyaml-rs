#!/usr/bin/env node
// Records what js-yaml does on every yaml-test-suite case and benchmark sample, so the
// Rust port can be checked without a JS runtime.
// Usage: node extract-upstream-fixtures.mjs <js-yaml npm dir> <js-yaml source dir> <yaml-test-suite dir>
//
// Writes conformance/fixtures/:
//   events.json  parseEvents(): the event objects (UTF-16 offsets) and the decoded text of
//                each scalar (getScalarValue), or the error
//   load.json    loadAll() per schema: values in the typed encoding below, or the error
// Inputs: yaml-test-suite, small js-yaml benchmark samples, conformance/custom-cases.json.
//
// Typed encoding of values (plain JSON can't carry them):
//   null, booleans, strings and finite numbers other than -0 are themselves
//   {"$number": "NaN" | "Infinity" | "-Infinity" | "-0"}
//   arrays are arrays
//   {"$object": [[key, value], ...]}   plain objects, in Object.keys order
//   {"$map": [[key, value], ...]}      Map
//   {"$set": [value, ...]}             Set
//   {"$binary": "<base64>"}            Uint8Array
//   {"$date": <ms since epoch>}        Date
//   {"$shared": i}                     an object reached more than once (aliases);
//                                      its value is the document's shared[i]
// Errors: {"reason": string, "position": UTF-16 offset | null}

import fs from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const [npmDir, srcDir, suiteDir] = process.argv.slice(2)
if (!suiteDir) {
  console.error('usage: extract-upstream-fixtures.mjs <js-yaml npm dir> <js-yaml source dir> <yaml-test-suite dir>')
  process.exit(2)
}

const yaml = await import(pathToFileURL(path.join(npmDir, 'dist/js-yaml.mjs')))
const outDir = path.join(import.meta.dirname, '../conformance/fixtures')
const MAX_SAMPLE_BYTES = 64 * 1024

const SCHEMAS = {
  core: yaml.CORE_SCHEMA,
  json: yaml.JSON_SCHEMA,
  yaml11: yaml.YAML11_SCHEMA
}

// Same marker unescaping as upstream test/spec/spec.test.mjs.
function unescapeFixtureText (text) {
  return text
    .replaceAll('␣', ' ')
    .replace(/—*»/g, '\t')
    .replaceAll('←', '\r')
    .replaceAll('⇔', '﻿')
    .replaceAll('↵', '')
    .replace(/∎\n$/u, '')
}

function suiteCases () {
  const cases = []
  const dir = path.join(suiteDir, 'src')
  for (const file of fs.readdirSync(dir).sort()) {
    if (path.extname(file) !== '.yaml') continue
    const id = path.basename(file, '.yaml')
    const fixtures = yaml.load(fs.readFileSync(path.join(dir, file), 'utf8'))
    // Later documents inherit earlier fields, except `fail` (as upstream does).
    for (let i = 1; i < fixtures.length; i++) {
      const current = fixtures[i]
      fixtures[i] = Object.assign({}, fixtures[i - 1], current)
      if (!Object.hasOwn(current, 'fail')) delete fixtures[i].fail
    }
    if (fixtures[0]?.skip) continue
    fixtures.forEach((fixture, i) => {
      const suffix = fixtures.length > 1 ? `/${String(i).padStart(2, '0')}` : ''
      cases.push({
        id: `suite/${id}${suffix}`,
        name: fixture.name || id,
        yaml: unescapeFixtureText(fixture.yaml),
        fail: Boolean(fixture.fail)
      })
    })
  }
  return cases
}

function sampleCases () {
  const dir = path.join(srcDir, 'benchmark/samples')
  return fs.readdirSync(dir).sort()
    .filter((file) => fs.statSync(path.join(dir, file)).size <= MAX_SAMPLE_BYTES)
    .map((file) => ({ id: `sample/${file}`, name: file, yaml: fs.readFileSync(path.join(dir, file), 'utf8') }))
}

function errorOf (e) {
  if (e instanceof yaml.YAMLException) return { reason: e.reason, position: e.mark?.position ?? null }
  return { reason: `${e.name}: ${e.message}`, position: null, native: true }
}

function encodeDocument (root) {
  // Pass 1: count how often each object is reached, to find shared ones.
  const seen = new Map()
  const visit = (v) => {
    if (v === null || typeof v !== 'object') return
    seen.set(v, (seen.get(v) || 0) + 1)
    if (seen.get(v) > 1) return
    if (Array.isArray(v) || v instanceof Set) for (const x of v) visit(x)
    else if (v instanceof Map) for (const [k, x] of v) { visit(k); visit(x) }
    else if (!(v instanceof Uint8Array) && !(v instanceof Date)) for (const k of Object.keys(v)) visit(v[k])
  }
  visit(root)

  const sharedIndex = new Map()
  const shared = []
  const encode = (v, top = false) => {
    if (v === null || typeof v === 'boolean' || typeof v === 'string') return v
    if (typeof v === 'number') {
      if (Number.isNaN(v)) return { $number: 'NaN' }
      if (v === Infinity) return { $number: 'Infinity' }
      if (v === -Infinity) return { $number: '-Infinity' }
      if (Object.is(v, -0)) return { $number: '-0' }
      return v
    }
    if (typeof v !== 'object') throw new Error(`unexpected ${typeof v} in loaded value`)
    if (!top && seen.get(v) > 1) {
      if (!sharedIndex.has(v)) {
        sharedIndex.set(v, shared.length)
        shared.push(null)
        shared[sharedIndex.get(v)] = encode(v, true)
      }
      return { $shared: sharedIndex.get(v) }
    }
    if (Array.isArray(v)) return v.map((x) => encode(x))
    if (v instanceof Set) return { $set: [...v].map((x) => encode(x)) }
    if (v instanceof Map) return { $map: [...v].map(([k, x]) => [encode(k), encode(x)]) }
    if (v instanceof Uint8Array) return { $binary: Buffer.from(v).toString('base64') }
    if (v instanceof Date) return { $date: v.getTime() }
    return { $object: Object.keys(v).map((k) => [k, encode(v[k])]) }
  }
  const value = seen.get(root) > 1 ? encode(root, false) : encode(root, true)
  return shared.length ? { value, shared } : { value }
}

// Hand-written edge cases the suite lacks (aliased collections, YAML 1.1 types, offsets).
function customCases () {
  const file = path.join(import.meta.dirname, '../conformance/custom-cases.json')
  return JSON.parse(fs.readFileSync(file, 'utf8')).map((c) => ({ ...c, name: c.id }))
}

const cases = [...suiteCases(), ...sampleCases(), ...customCases()]
const events = []
const loads = []

for (const c of cases) {
  const base = { id: c.id, name: c.name, yaml: c.yaml, ...(c.fail ? { fail: true } : {}) }
  try {
    const parsed = yaml.parseEvents(c.yaml, {})
    // Decoded text of each scalar event, in order (checks getScalarValue).
    const scalars = parsed.filter((e) => e.type === yaml.EVENT_ID.SCALAR).map((e) => yaml.getScalarValue(c.yaml, e))
    events.push({ ...base, events: parsed, scalars })
  } catch (e) {
    events.push({ ...base, error: errorOf(e) })
  }
  for (const [schema, value] of Object.entries(SCHEMAS)) {
    try {
      const documents = yaml.loadAll(c.yaml, { schema: value })
      loads.push({ ...base, schema, documents: documents.map(encodeDocument) })
    } catch (e) {
      loads.push({ ...base, schema, error: errorOf(e) })
    }
  }
}

fs.mkdirSync(outDir, { recursive: true })
fs.writeFileSync(path.join(outDir, 'events.json'), JSON.stringify(events, null, '\t') + '\n')
fs.writeFileSync(path.join(outDir, 'load.json'), JSON.stringify(loads, null, '\t') + '\n')
console.log(`events.json: ${events.length} cases, load.json: ${loads.length} cases (${Object.keys(SCHEMAS).join(', ')})`)
