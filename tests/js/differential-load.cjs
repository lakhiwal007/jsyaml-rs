'use strict';
// Randomised differential test of loadAll: mutates the fixture inputs and checks the
// package returns exactly js-yaml's documents (deep equality plus the same object
// identity structure, so aliases and cycles match) or the same error, across schemas
// and options.
// Usage: node tests/js/differential-load.cjs <package dir> [cases] [seed]

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const [pkgDir, count = '20000', seedArg = '1'] = process.argv.slice(2);
if (!pkgDir) {
	console.error('usage: node tests/js/differential-load.cjs <package dir> [cases] [seed]');
	process.exit(2);
}
const pkg = require(path.resolve(pkgDir));
const y = require(require.resolve('js-yaml', { paths: [path.resolve(pkgDir)] }));

const OPTIONS = [
	{},
	{ schema: y.JSON_SCHEMA },
	{ schema: y.YAML11_SCHEMA },
	{ schema: y.FAILSAFE_SCHEMA },
	{ schema: y.CORE_SCHEMA.withTags(y.mergeTag, y.realMapTag) },
	{ schema: y.YAML11_SCHEMA.withTags(y.legacyMapTag) },
	{ schema: y.YAML11_SCHEMA.withTags(y.realMapTag) },
	{ json: true },
	{ schema: y.YAML11_SCHEMA, maxAliases: 2, maxTotalMergeKeys: 3 },
];

const inputs = JSON.parse(fs.readFileSync(path.join(__dirname, '../../conformance/fixtures/load.json'), 'utf8')).map((c) => c.yaml);
const ALPHABET = [' ', '  ', '\n', '\r\n', '\t', '- ', ': ', '? ', ',', '[', ']', '{', '}', '#', '&a ', '*a', '&b ', '*b', '<<: ', '<<: *a', '!!set ', '!!omap ', '!!pairs ', '!!map ', '!!seq ', '!!str ', '!!int ', '!!float ', '!!binary ', '!!timestamp ', '!!bool ', '!!null ', '|', '>', "'", '"', '\\', '~', 'null', 'true', 'yes', 'off', '1', '0x1F', '0o17', '-0', '1e3', '.inf', '.nan', '1_000', '1:20', '2001-12-14', '2001-12-14 21:59:43.10 -5', 'é', '😀', '\\u00e9', '\\uD83D\\uDE00', '__proto__', 'x'];

let seed = Number(seedArg) >>> 0 || 1;
function rand(n) {
	seed ^= seed << 13; seed >>>= 0; seed ^= seed >>> 17; seed ^= seed << 5; seed >>>= 0;
	return seed % n;
}

function mutate(s) {
	const rounds = 1 + rand(4);
	for (let r = 0; r < rounds; r++) {
		const at = rand(s.length + 1);
		const piece = ALPHABET[rand(ALPHABET.length)];
		switch (rand(4)) {
			case 0: s = s.slice(0, at) + piece + s.slice(at); break;
			case 1: s = s.slice(0, at) + s.slice(at + 1 + rand(3)); break;
			case 2: s = s.slice(0, at) + piece + s.slice(at + 1); break;
			default: { const len = rand(12); s = s.slice(0, at) + s.slice(at, at + len) + s.slice(at); }
		}
	}
	return s;
}

// Object identity structure: each object gets the index of its first visit.
function identityShape(value) {
	const ids = new Map();
	const out = [];
	const walk = (v) => {
		if (v === null || typeof v !== 'object') return;
		if (ids.has(v)) { out.push(ids.get(v)); return; }
		ids.set(v, ids.size);
		out.push(-1);
		if (Array.isArray(v) || v instanceof Set) for (const x of v) walk(x);
		else if (v instanceof Map) for (const [k, x] of v) { walk(k); walk(x); }
		else if (!(v instanceof Uint8Array) && !(v instanceof Date)) for (const k of Object.keys(v)) walk(v[k]);
	};
	walk(value);
	return out;
}

function outcome(lib, input, options) {
	try {
		const documents = lib.loadAll(input, options);
		return { documents, shape: identityShape(documents) };
	} catch (e) {
		if (e instanceof y.YAMLException) return { error: e.message };
		return { error: `${e.name}: ${e.message}`, native: true };
	}
}

let compared = 0, errors = 0, skipped = 0;
for (let i = 0; i < Number(count); i++) {
	const input = mutate(inputs[rand(inputs.length)]);
	const options = OPTIONS[rand(OPTIONS.length)];
	const expected = outcome(y, input, options);
	if (expected.native) { skipped++; continue; }
	const actual = outcome(pkg, input, options);
	try {
		assert.deepStrictEqual(actual, expected);
	} catch {
		console.error(`MISMATCH (case ${i}, seed ${seedArg}, options ${OPTIONS.indexOf(options)}) for input ${JSON.stringify(input)}`);
		console.error('expected:', require('node:util').inspect(expected, { depth: 6 }).slice(0, 1500));
		console.error('  actual:', require('node:util').inspect(actual, { depth: 6 }).slice(0, 1500));
		process.exit(1);
	}
	compared++;
	if (expected.error) errors++;
}
console.log(`differential-load: ${compared} mutated inputs match js-yaml, ${errors} of them errors (${skipped} skipped: non-YAML errors in js-yaml)`);
