// Downloads the real-world corpus listed in corpus.json into data/corpus/, pinned to
// commits or tags, and verifies each file's SHA-256 (recorded on first download with
// --update). The files are not committed: they belong to their projects.

import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

const here = import.meta.dirname
const manifestPath = path.join(here, 'corpus.json')
const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'))
const update = process.argv.includes('--update')
let changed = false

for (const entry of manifest) {
  if (!entry.url) continue
  const file = path.join(here, 'data', entry.file)
  if (!fs.existsSync(file)) {
    fs.mkdirSync(path.dirname(file), { recursive: true })
    const response = await fetch(entry.url, { signal: AbortSignal.timeout(300_000) })
    if (!response.ok) throw new Error(`${entry.url}: HTTP ${response.status}`)
    fs.writeFileSync(file, Buffer.from(await response.arrayBuffer()))
  }
  const sha256 = createHash('sha256').update(fs.readFileSync(file)).digest('hex')
  if (!entry.sha256 || update) {
    if (entry.sha256 !== sha256) changed = true
    entry.sha256 = sha256
  } else if (entry.sha256 !== sha256) {
    throw new Error(`${entry.file}: SHA-256 mismatch (got ${sha256}); delete it to re-download`)
  }
  console.log(`${entry.file}: ${(fs.statSync(file).size / 1e3).toFixed(1)} KB ok`)
}
if (changed) fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n')
