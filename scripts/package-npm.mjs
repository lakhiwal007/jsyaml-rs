#!/usr/bin/env node
// Assembles the npm packages for publishing:
//   <out>/jsyaml-node             JS entry points + optionalDependencies on every platform
//   <out>/jsyaml-node-<platform>  one native binary each (os / cpu / libc restricted)
//   <out>/jsyaml-wasm             the WebAssembly build
// Usage: node scripts/package-npm.mjs --artifacts <dir> [--wasm <dir>] [--out <dir>] [--allow-missing]
//   <dir>/<rust target>/jsyaml.node, one per entry of scripts/platforms.json.
// --allow-missing packages only the platforms present (for local testing; never publish that).

import fs from 'node:fs'
import path from 'node:path'
import { parseArgs } from 'node:util'

const root = path.join(import.meta.dirname, '..')
const { values: args } = parseArgs({
  options: {
    artifacts: { type: 'string' },
    wasm: { type: 'string', default: path.join(root, 'crates/jsyaml-wasm/pkg') },
    out: { type: 'string', default: path.join(root, 'dist/npm') },
    'allow-missing': { type: 'boolean', default: false }
  }
})
if (!args.artifacts) {
  console.error('usage: node scripts/package-npm.mjs --artifacts <dir> [--wasm <dir>] [--out <dir>] [--allow-missing]')
  process.exit(2)
}

const version = /^version = "(.*)"/m.exec(fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8'))[1]
const jsYamlVersion = fs.readFileSync(path.join(root, 'conformance/UPSTREAM_VERSION'), 'utf8').trim()
const platforms = JSON.parse(fs.readFileSync(path.join(root, 'scripts/platforms.json'), 'utf8'))
const repository = { type: 'git', url: 'git+https://github.com/lakhiwal007/jsyaml-rs.git' }
const common = { version, license: 'MIT', repository, engines: { node: '>=18' } }

const write = (dir, file, content) => fs.writeFileSync(path.join(dir, file), typeof content === 'string' ? content : JSON.stringify(content, null, 2) + '\n')
const copy = (from, dir, name = path.basename(from)) => fs.copyFileSync(from, path.join(dir, name))
const fresh = (name) => { const dir = path.join(args.out, name); fs.rmSync(dir, { recursive: true, force: true }); fs.mkdirSync(dir, { recursive: true }); return dir }

// Per-platform binaries.
const present = []
for (const p of platforms) {
  const binary = path.join(args.artifacts, p.target, 'jsyaml.node')
  if (!fs.existsSync(binary)) {
    if (args['allow-missing']) continue
    throw new Error(`missing ${binary} (pass --allow-missing for a local test build)`)
  }
  const name = `jsyaml-node-${p.package}`
  const dir = fresh(name)
  copy(binary, dir)
  copy(path.join(root, 'LICENSE'), dir)
  write(dir, 'README.md', `# ${name}\n\nThe \`${p.target}\` binary of [jsyaml-node](https://www.npmjs.com/package/jsyaml-node). Install \`jsyaml-node\` instead; it pulls this in.\n`)
  write(dir, 'package.json', {
    name,
    ...common,
    description: `jsyaml-node native binary for ${p.target}`,
    main: 'jsyaml.node',
    files: ['jsyaml.node', 'LICENSE', 'README.md'],
    os: [p.os],
    cpu: [p.cpu],
    ...(p.libc ? { libc: [p.libc] } : {})
  })
  present.push(name)
}

// The main package.
const node = fresh('jsyaml-node')
for (const f of ['index.cjs', 'index.mjs', 'binding.cjs']) copy(path.join(root, 'crates/jsyaml-node/js', f), node)
for (const f of ['decode.cjs', 'index.d.ts']) copy(path.join(root, 'crates/jsyaml-tape/js', f), node)
copy(path.join(root, 'LICENSE'), node)
copy(path.join(root, 'README.md'), node)
write(node, 'package.json', {
  name: 'jsyaml-node',
  ...common,
  description: 'Drop-in js-yaml with a native YAML loader built from Rust with NAPI-RS',
  keywords: ['yaml', 'js-yaml', 'parser', 'napi', 'rust'],
  type: 'commonjs',
  main: 'index.cjs',
  types: 'index.d.ts',
  exports: { '.': { types: './index.d.ts', import: './index.mjs', require: './index.cjs' } },
  files: ['index.cjs', 'index.mjs', 'binding.cjs', 'decode.cjs', 'index.d.ts', 'LICENSE', 'README.md'],
  dependencies: { 'js-yaml': jsYamlVersion },
  optionalDependencies: Object.fromEntries(platforms.map((p) => [`jsyaml-node-${p.package}`, version]))
})

// The WebAssembly package (already laid out by scripts/build-wasm.sh).
if (fs.existsSync(path.join(args.wasm, 'package.json'))) {
  const wasm = fresh('jsyaml-wasm')
  for (const f of fs.readdirSync(args.wasm)) if (f !== '.gitignore') copy(path.join(args.wasm, f), wasm)
  copy(path.join(root, 'LICENSE'), wasm)
  copy(path.join(root, 'README.md'), wasm)
  const pkg = JSON.parse(fs.readFileSync(path.join(wasm, 'package.json'), 'utf8'))
  write(wasm, 'package.json', { ...pkg, ...common, files: [...pkg.files, 'LICENSE', 'README.md'] })
  present.push('jsyaml-wasm')
}

console.log(`packaged ${version} into ${args.out}: jsyaml-node, ${present.join(', ')}`)
