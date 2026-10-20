'use strict';
// Memory regression test: repeated loads must not grow native memory. The napi addon
// once leaked every value tape (external ArrayBuffers whose finalizers never freed the
// Rust allocation); V8's heap stayed flat while RSS grew ~6 KB per call.
// Usage: node --expose-gc tests/js/memory.cjs <package dir>

const path = require('node:path');

const pkgDir = process.argv[2];
if (!pkgDir || typeof globalThis.gc !== 'function') {
	console.error('usage: node --expose-gc tests/js/memory.cjs <package dir>');
	process.exit(2);
}
const pkg = require(path.resolve(pkgDir));

// One input per transport: JSON fast path, value tape (aliases), event tape (custom tag).
const plain = 'services:\n' + Array.from({ length: 40 }, (_, i) => `  s${i}: {image: "app:${i}", ports: [${i}, 443]}`).join('\n') + '\n';
const aliased = 'base: &b {restart: always, env: [A, B, C]}\n' + Array.from({ length: 40 }, (_, i) => `s${i}: {<<: *b, image: app${i}, shared: *b}`).join('\n') + '\n';
const schema = pkg.CORE_SCHEMA.withTags(pkg.mergeTag, pkg.defineScalarTag('!upper', { resolve: (s) => s.toUpperCase() }));
const custom = 'a: !upper x\n' + plain;
const cases = [
	['json fast path', () => pkg.loadAll(plain)],
	['value tape', () => pkg.loadAll(aliased, { schema: pkg.CORE_SCHEMA.withTags(pkg.mergeTag) })],
	['event tape', () => pkg.loadAll(custom, { schema })],
	['parseEvents', () => pkg.parseEvents(plain)],
];

const CALLS = 30000;
const LIMIT_MB = 40;
const rss = () => process.memoryUsage().rss / 1e6;
let failed = false;
for (const [name, run] of cases) {
	for (let i = 0; i < 2000; i++) run(); // warm up: let the heap reach its working size
	gc();
	const before = rss();
	for (let i = 0; i < CALLS; i++) run();
	gc();
	const growth = rss() - before;
	const ok = growth < LIMIT_MB;
	failed ||= !ok;
	console.log(`${ok ? 'ok' : 'FAIL'}  ${name}: RSS ${growth >= 0 ? '+' : ''}${growth.toFixed(1)} MB over ${CALLS} calls (limit ${LIMIT_MB} MB)`);
}
process.exit(failed ? 1 : 0);
