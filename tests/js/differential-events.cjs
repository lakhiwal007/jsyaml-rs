'use strict';
// Randomised differential test of parseEvents: mutates the fixture inputs (insert,
// delete, replace and duplicate characters YAML cares about) and checks the package
// produces exactly js-yaml's events and decoded scalars, or the same error.
// Usage: node tests/js/differential-events.cjs <package dir> [cases] [seed]

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const [pkgDir, count = '20000', seedArg = '1'] = process.argv.slice(2);
if (!pkgDir) {
	console.error('usage: node tests/js/differential-events.cjs <package dir> [cases] [seed]');
	process.exit(2);
}
const pkg = require(path.resolve(pkgDir));
const upstream = require(require.resolve('js-yaml', { paths: [path.resolve(pkgDir)] }));

const inputs = JSON.parse(fs.readFileSync(path.join(__dirname, '../../conformance/fixtures/events.json'), 'utf8')).map((c) => c.yaml);
const ALPHABET = [' ', ' ', '  ', '\n', '\r\n', '\r', '\t', '-', '- ', ':', ': ', '?', ',', '[', ']', '{', '}', '#', '&a', '*a', '!', '!!str', '|', '>', "'", '"', '\\', '%', '---', '...', 'x', 'é', '😀', '﻿', '\u0085', '1', '\\u00e9'];

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

function outcome(lib, input) {
	try {
		const events = lib.parseEvents(input, {});
		const scalars = events.filter((e) => e.type === 4).map((e) => upstream.getScalarValue(input, e));
		return { events, scalars };
	} catch (e) {
		if (e instanceof upstream.YAMLException) return { error: e.message };
		return { error: `${e.name}: ${e.message}`, native: true };
	}
}

let compared = 0, errors = 0, skipped = 0;
for (let i = 0; i < Number(count); i++) {
	const input = mutate(inputs[rand(inputs.length)]);
	// Rust strings can't hold lone surrogates, which mutations produce by splitting pairs.
	if (/[\uD800-\uDFFF]/.test(input.replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]/g, ''))) { skipped++; continue; }
	const expected = outcome(upstream, input);
	if (expected.native) { skipped++; continue; }
	const actual = outcome(pkg, input);
	try {
		assert.deepStrictEqual(actual, expected);
	} catch {
		console.error(`MISMATCH (case ${i}, seed ${seedArg}) for input ${JSON.stringify(input)}`);
		console.error('expected:', JSON.stringify(expected).slice(0, 800));
		console.error('  actual:', JSON.stringify(actual).slice(0, 800));
		process.exit(1);
	}
	compared++;
	if (expected.error) errors++;
}
console.log(`differential-events: ${compared} mutated inputs match js-yaml, ${errors} of them errors (${skipped} skipped: lone surrogates or non-YAML errors)`);
