// Seeds the cargo-fuzz corpora with every conformance input (fuzz/corpus is not committed).
import fs from 'node:fs'
import path from 'node:path'

const root = path.join(import.meta.dirname, '..')
const inputs = JSON.parse(fs.readFileSync(path.join(root, 'conformance/fixtures/events.json'), 'utf8')).map((c) => c.yaml)
for (const target of ['load', 'fast_path']) {
  const dir = path.join(root, 'fuzz/corpus', target)
  fs.mkdirSync(dir, { recursive: true })
  inputs.forEach((yaml, i) => fs.writeFileSync(path.join(dir, `seed-${i}`), yaml))
}
console.log(`seeded ${inputs.length} inputs per target`)
