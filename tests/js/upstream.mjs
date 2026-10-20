// Runs js-yaml's own test suite (node:test) against a built package, by resolving the
// tests' `import ... from 'js-yaml'` to the package instead.
// Usage: node tests/js/upstream.mjs <package dir>   (e.g. crates/jsyaml-node/pkg)
//
// Needs `just sync-upstream` (vendor/js-yaml-<version>, vendor/yaml-test-suite).

import { execFileSync, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const root = path.join(import.meta.dirname, '../..')
const pkg = path.resolve(process.argv[2] ?? '')
if (!process.argv[2] || !fs.existsSync(path.join(pkg, 'index.mjs'))) {
  console.error('usage: node tests/js/upstream.mjs <package dir containing index.mjs>')
  process.exit(2)
}

const version = fs.readFileSync(path.join(root, 'conformance/UPSTREAM_VERSION'), 'utf8').trim()
const upstream = path.join(root, `vendor/js-yaml-${version}`)
if (!fs.existsSync(upstream)) {
  console.error(`missing ${upstream}; run: just sync-upstream`)
  process.exit(2)
}

// The spec test reads the suite from test/spec/yaml-test-suite.
const suiteLink = path.join(upstream, 'test/spec/yaml-test-suite')
// On Windows a junction, which unlike a directory symlink needs no special rights.
if (!fs.existsSync(suiteLink)) {
  fs.symlinkSync(path.join(root, 'vendor/yaml-test-suite'), suiteLink, process.platform === 'win32' ? 'junction' : 'dir')
}

// Test-only devDependencies of js-yaml (@streamparser/json, ...).
if (!fs.existsSync(path.join(upstream, 'node_modules'))) {
  execFileSync('npm', ['ci', '--ignore-scripts', '--no-audit', '--no-fund', '--silent'], { cwd: upstream, stdio: 'inherit', shell: process.platform === 'win32' })
}

// dist.test.mjs checks js-yaml's own build artefacts, which don't apply here.
const tests = [
  ...fs.globSync('test/core/**/*.test.mjs', { cwd: upstream }).filter((f) => !f.endsWith('dist.test.mjs')),
  ...fs.globSync('test/spec/*.test.mjs', { cwd: upstream })
].sort()

const result = spawnSync(process.execPath, [
  // A file: URL, not a path: on Windows Node would read `C:` as a URL scheme.
  '--import', pathToFileURL(path.join(import.meta.dirname, 'alias-register.mjs')).href,
  '--test', '--test-reporter', process.env.TEST_REPORTER || 'dot',
  ...tests
], {
  cwd: upstream,
  stdio: 'inherit',
  env: { ...process.env, JSYAML_PKG: pkg, JSYAML_UPSTREAM_TESTS: path.join(upstream, 'test') }
})
process.exit(result.status ?? 1)
