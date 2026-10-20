'use strict';
// Loads every fixture input with a built package and with js-yaml, and compares values
// (deepStrictEqual, plus alias identity) or error messages.
// Usage: node tests/js/differential.cjs <package dir>

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const pkgDir = process.argv[2];
if (!pkgDir) {
	console.error('usage: node tests/js/differential.cjs <package dir>');
	process.exit(2);
}
const pkg = require(path.resolve(pkgDir));
// The package's own js-yaml (its dependency) so schema objects are the ones it knows.
const upstream = require(require.resolve('js-yaml', { paths: [path.resolve(pkgDir)] }));

const fixtures = JSON.parse(fs.readFileSync(path.join(__dirname, '../../conformance/fixtures/load.json'), 'utf8'));
const schemas = { core: upstream.CORE_SCHEMA, json: upstream.JSON_SCHEMA, yaml11: upstream.YAML11_SCHEMA };

function outcome(lib, f) {
	try {
		return { value: lib.loadAll(f.yaml, { schema: schemas[f.schema], filename: 'input.yml' }) };
	} catch (e) {
		return { error: e instanceof upstream.YAMLException ? e.message : `${e.name}: ${e.message}` };
	}
}

let pass = 0;
const failures = [];
for (const f of fixtures) {
	const expected = outcome(upstream, f);
	const actual = outcome(pkg, f);
	try {
		assert.deepStrictEqual(actual, expected);
		pass++;
	} catch (e) {
		failures.push(`${f.schema} ${f.id}\n${e.message.split('\n').slice(0, 12).join('\n')}`);
	}
}
console.log(`differential: ${pass} / ${fixtures.length} match js-yaml`);
if (process.env.DIFF_REPORT) for (const f of failures) console.log(`\nFAIL ${f}`);
process.exit(failures.length ? 1 : 0);
