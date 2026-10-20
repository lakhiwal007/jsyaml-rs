'use strict';
// Tests decode.cjs (shared by both packages) against a fake native module, so it runs
// without building Rust: `npm test` after `npm install`.

const { describe, it } = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const upstream = require('js-yaml');
const { createApi, decodeValueTape, decodeEventTape } = require('../../crates/jsyaml-tape/js/decode.cjs');

const STRIDE = 11;

// Encodes upstream event objects the way crates/jsyaml-tape/src/events.rs does.
function encodeEvents(events) {
	const records = new Int32Array(events.length * STRIDE);
	const directives = [];
	events.forEach((e, i) => {
		const r = records.subarray(i * STRIDE, (i + 1) * STRIDE);
		r[0] = e.type;
		if (e.type === 1) r.set([+e.explicitStart, +e.explicitEnd, directives.push(e.directives) - 1], 1);
		else if (e.type === 2 || e.type === 3) r.set([e.start, e.anchorStart, e.anchorEnd, e.tagStart, e.tagEnd, e.style], 1);
		else if (e.type === 4) {
			r.set([e.valueStart, e.valueEnd, e.anchorStart, e.anchorEnd, e.tagStart, e.tagEnd, e.style, e.chomping, e.indent, +e.fast], 1);
		} else if (e.type === 5) r.set([e.anchorStart, e.anchorEnd], 1);
	});
	return { records, directives: JSON.stringify(directives) };
}

// A native module that answers with upstream's results in transport form.
const fakeNative = {
	calls: [],
	loadRaw(source, options) {
		this.calls.push(['loadRaw', options.tags]);
		try {
			return { json: JSON.stringify(upstream.loadAll(source)) };
		} catch (e) {
			return { errorReason: e.reason, errorPosition: e.mark ? e.mark.position : null };
		}
	},
	eventsRaw(source) {
		this.calls.push(['eventsRaw']);
		try {
			return encodeEvents(upstream.parseEvents(source, {}));
		} catch (e) {
			return { errorReason: e.reason, errorPosition: e.mark ? e.mark.position : null };
		}
	},
};
const api = createApi(upstream, fakeNative);

describe('event tape', () => {
	const fixtures = JSON.parse(fs.readFileSync(path.join(__dirname, '../../conformance/fixtures/events.json'), 'utf8'));
	it(`round-trips upstream events for ${fixtures.length} fixture inputs`, () => {
		for (const f of fixtures) {
			if (!f.events) continue;
			assert.deepStrictEqual(decodeEventTape(encodeEvents(f.events)), f.events, f.id);
		}
	});
});

// Builds a value tape from the fixtures' typed encoding, the way
// crates/jsyaml-tape/src/values.rs builds one from Rust values.
function encodeValues(documents) {
	const ops = [documents.length], numbers = [], bytes = [];
	let strings = '';
	const str = (s) => { ops.push(strings.length, s.length); strings += s; };
	const num = (code, n) => { ops.push(code, numbers.length); numbers.push(n); };
	function value(v) {
		if (v === null) return ops.push(0);
		if (v === true) return ops.push(1);
		if (v === false) return ops.push(2);
		if (typeof v === 'number') return num(3, v);
		if (typeof v === 'string') return ops.push(4), str(v);
		if (Array.isArray(v)) return ops.push(5, v.length), v.forEach(value);
		if ('$number' in v) return num(3, Number(v.$number));
		if ('$object' in v) return ops.push(6, v.$object.length), v.$object.forEach(([k, x]) => (str(k), value(x)));
		if ('$map' in v) return ops.push(7, v.$map.length), v.$map.forEach(([k, x]) => (value(k), value(x)));
		if ('$set' in v) return ops.push(8, v.$set.length), v.$set.forEach(value);
		if ('$binary' in v) {
			const b = Buffer.from(v.$binary, 'base64');
			ops.push(9, bytes.length, b.length);
			return bytes.push(...b);
		}
		if ('$date' in v) return num(10, v.$date);
		if ('$shared' in v) return ops.push(11, v.$shared);
		throw new Error(`unknown encoding ${JSON.stringify(v)}`);
	}
	for (const doc of documents) {
		const shared = doc.shared || [];
		ops.push(shared.length);
		const table = ops.length;
		ops.push(...shared.map(() => 0));
		value(doc.value);
		shared.forEach((v, i) => { ops[table + i] = ops.length; value(v); });
	}
	return { ops: Uint32Array.from(ops), numbers: Float64Array.from(numbers), strings, bytes: Uint8Array.from(bytes) };
}

describe('value tape', () => {
	const fixtures = JSON.parse(fs.readFileSync(path.join(__dirname, '../../conformance/fixtures/load.json'), 'utf8'));
	const schemas = { core: upstream.CORE_SCHEMA, json: upstream.JSON_SCHEMA, yaml11: upstream.YAML11_SCHEMA };

	it(`decodes to exactly upstream's values for ${fixtures.length} fixture loads`, () => {
		for (const f of fixtures) {
			if (!f.documents) continue;
			const expected = upstream.loadAll(f.yaml, { schema: schemas[f.schema] });
			assert.deepStrictEqual(decodeValueTape(encodeValues(f.documents)), expected, `${f.schema} ${f.id}`);
		}
	});

	it('preserves identity of aliased objects, including cycles', () => {
		const byId = (id) => fixtures.find((f) => f.id === id && f.schema === 'core').documents;
		const [shared] = decodeValueTape(encodeValues(byId('custom/alias/collection')));
		assert.strictEqual(shared.a, shared.b);
		const [cycle] = decodeValueTape(encodeValues(byId('custom/alias/recursive')));
		assert.strictEqual(cycle[0], cycle);
	});

	it('defines __proto__ as an own property', () => {
		const [doc] = decodeValueTape(encodeValues([{ value: { $object: [['__proto__', { $object: [['polluted', true]] }]] } }]));
		assert.ok(Object.hasOwn(doc, '__proto__'));
		assert.strictEqual(Object.getPrototypeOf(doc), Object.prototype);
		assert.strictEqual({}.polluted, undefined);
	});
});

describe('createApi', () => {
	it('loads through the JSON path with the core schema', () => {
		fakeNative.calls.length = 0;
		assert.deepStrictEqual(api.load('a: [1, b]'), { a: [1, 'b'] });
		assert.deepStrictEqual(fakeNative.calls, [['loadRaw', []]]);
	});

	it('passes built-in schemas to native code by tag name', () => {
		fakeNative.calls.length = 0;
		api.load('a: 1', { schema: upstream.CORE_SCHEMA.withTags(upstream.mergeTag) });
		assert.deepStrictEqual(fakeNative.calls[0][1].at(-1), 'mergeTag');
	});

	it('sends custom tags down the events path', () => {
		fakeNative.calls.length = 0;
		const schema = upstream.CORE_SCHEMA.withTags(upstream.defineScalarTag('!upper', { resolve: (s) => s.toUpperCase() }));
		assert.deepStrictEqual(api.load('x: !upper hi', { schema }), { x: 'HI' });
		assert.deepStrictEqual(fakeNative.calls, [['eventsRaw']]);
	});

	it("throws js-yaml's own YAMLException with the same message", () => {
		for (const input of ['a: 1\na: 2', 'é€: [1, 2\n', 'a:\n\tb: 1', '']) {
			let expected, actual;
			try { upstream.load(input, { filename: 'f.yml' }); } catch (e) { expected = e; }
			try { api.load(input, { filename: 'f.yml' }); } catch (e) { actual = e; }
			assert.ok(actual instanceof upstream.YAMLException, input);
			assert.strictEqual(actual.message, expected.message, input);
		}
	});

	it('keeps the deprecated loadAll iterator form', () => {
		const seen = [];
		assert.strictEqual(api.loadAll('--- 1\n--- 2', (d) => seen.push(d)), undefined);
		assert.deepStrictEqual(seen, [1, 2]);
		assert.throws(() => api.load('--- 1\n--- 2'), /expected a single document/);
	});

	it('re-exports the rest of js-yaml unchanged', () => {
		assert.strictEqual(api.dump, upstream.dump);
		assert.strictEqual(api.YAMLException, upstream.YAMLException);
	});
});
