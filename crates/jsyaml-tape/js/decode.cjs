'use strict';
// Shared by the napi-rs and wasm packages: turns the native results (see
// crates/jsyaml-tape/src/lib.rs) into exactly what js-yaml returns, and wraps
// js-yaml's API so `require('jsyaml-node')` is a drop-in for `require('js-yaml')`.

// Op codes of the value tape; keep in sync with crates/jsyaml-tape/src/values.rs.
const NULL = 0, TRUE = 1, FALSE = 2, NUMBER = 3, STRING = 4, ARRAY = 5, OBJECT = 6,
	MAP = 7, SET = 8, BINARY = 9, DATE = 10, SHARED = 11;
const EVENT_STRIDE = 11;

// Inputs shorter than this go straight to js-yaml: below some size, crossing into native
// code costs more than it saves. Tune with `just bench-compare`.
const NATIVE_MIN_LENGTH = 0;

// Built-in tags the native constructor implements, by js-yaml export name; keep in sync
// with `Tag::export_name` in crates/jsyaml/src/schema.rs.
const BUILTIN_TAGS = [
	'strTag', 'nullCoreTag', 'nullJsonTag', 'nullYaml11Tag', 'boolCoreTag', 'boolJsonTag',
	'boolYaml11Tag', 'intCoreTag', 'intJsonTag', 'intYaml11Tag', 'floatCoreTag',
	'floatJsonTag', 'floatYaml11Tag', 'mergeTag', 'binaryTag', 'timestampTag', 'seqTag',
	'omapTag', 'pairsTag', 'mapTag', 'realMapTag', 'legacyMapTag', 'setTag',
];

// Lone surrogates can't cross into Rust (strings become UTF-8), so such inputs go to
// js-yaml unchanged; it rejects them with its own error.
const LONE_SURROGATE = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?:^|[^\uD800-\uDBFF])[\uDC00-\uDFFF]/;
const isWellFormed = typeof String.prototype.isWellFormed === 'function'
	? (s) => s.isWellFormed()
	: (s) => !LONE_SURROGATE.test(s);

function setKey(object, key, value) {
	// Same as js-yaml's mapTag: never invoke the __proto__ setter.
	if (key === '__proto__') {
		Object.defineProperty(object, key, { value, enumerable: true, configurable: true, writable: true });
	} else {
		object[key] = value;
	}
}

function decodeValueTape({ ops, numbers, strings, bytes }) {
	let pc = 0;
	let shared;

	function str() {
		const start = ops[pc++];
		return strings.slice(start, start + ops[pc++]);
	}

	// Creates the value at `pc`. Containers are filled in place, so a shared container
	// can be created empty first (`shell`) and filled later.
	function fill(code, out) {
		const n = ops[pc++];
		switch (code) {
			case ARRAY: for (let i = 0; i < n; i++) out.push(value()); break;
			case OBJECT: for (let i = 0; i < n; i++) { const k = str(); setKey(out, k, value()); } break;
			case MAP: for (let i = 0; i < n; i++) { const k = value(); out.set(k, value()); } break;
			default: for (let i = 0; i < n; i++) out.add(value());
		}
		return out;
	}

	function value() {
		const code = ops[pc++];
		switch (code) {
			case NULL: return null;
			case TRUE: return true;
			case FALSE: return false;
			case NUMBER: return numbers[ops[pc++]];
			case STRING: return str();
			case ARRAY: return fill(code, []);
			case OBJECT: return fill(code, {});
			case MAP: return fill(code, new Map());
			case SET: return fill(code, new Set());
			case BINARY: {
				const start = ops[pc++];
				return bytes.slice(start, start + ops[pc++]);
			}
			case DATE: return new Date(numbers[ops[pc++]]);
			case SHARED: return shared[ops[pc++]];
			default: throw new Error(`jsyaml: corrupt value tape (op ${code} at ${pc - 1})`);
		}
	}

	const documents = new Array(ops[pc++]);
	for (let d = 0; d < documents.length; d++) {
		const n = ops[pc++];
		const offsets = ops.subarray(pc, pc + n);
		pc += n;
		const rootStart = pc;
		// Two passes over the shared values, so that references between them (alias chains,
		// cycles) never recurse: create every container empty, then fill them.
		shared = new Array(n);
		for (let i = 0; i < n; i++) {
			const code = ops[offsets[i]];
			if (code === ARRAY) shared[i] = [];
			else if (code === OBJECT) shared[i] = {};
			else if (code === MAP) shared[i] = new Map();
			else if (code === SET) shared[i] = new Set();
			else { pc = offsets[i]; shared[i] = value(); }
		}
		let end = rootStart;
		for (let i = 0; i < n; i++) {
			const code = ops[offsets[i]];
			pc = offsets[i] + 1;
			if (code >= ARRAY && code <= SET) fill(code, shared[i]);
			else pc = offsets[i] + (code === BINARY ? 3 : 2);
			end = Math.max(end, pc);
		}
		pc = rootStart;
		documents[d] = value();
		pc = Math.max(end, pc);
	}
	return documents;
}

// The napi addon returns tapes packed in one Node-owned buffer (f64s, then 32-bit words,
// then bytes; see crates/jsyaml-node/src/lib.rs). The wasm build returns typed arrays.
function alignedBuffer(buf, alignment) {
	if (buf.byteOffset % alignment === 0) return { buffer: buf.buffer, offset: buf.byteOffset };
	const copy = new Uint8Array(buf.byteLength);
	copy.set(buf);
	return { buffer: copy.buffer, offset: 0 };
}

function unpackLoaded(result) {
	if (result.buf === undefined || result.buf === null) return result;
	const { buffer, offset } = alignedBuffer(result.buf, 8);
	const n = result.numbersLength, m = result.opsLength;
	return {
		numbers: new Float64Array(buffer, offset, n),
		ops: new Uint32Array(buffer, offset + 8 * n, m),
		bytes: new Uint8Array(buffer, offset + 8 * n + 4 * m, result.bytesLength),
		strings: result.strings,
	};
}

function unpackEvents(result) {
	if (result.buf === undefined || result.buf === null) return result;
	const { buffer, offset } = alignedBuffer(result.buf, 4);
	return { records: new Int32Array(buffer, offset, result.recordsLength), directives: result.directives };
}

function decodeEventTape({ records, directives }) {
	const docDirectives = JSON.parse(directives);
	const r = records;
	const events = new Array(r.length / EVENT_STRIDE);
	for (let e = 0, o = 0; e < events.length; e++, o += EVENT_STRIDE) {
		switch (r[o]) {
			case 1:
				events[e] = { type: 1, explicitStart: r[o + 1] === 1, explicitEnd: r[o + 2] === 1, directives: docDirectives[r[o + 3]] };
				break;
			case 2: case 3:
				events[e] = { type: r[o], start: r[o + 1], anchorStart: r[o + 2], anchorEnd: r[o + 3], tagStart: r[o + 4], tagEnd: r[o + 5], style: r[o + 6] };
				break;
			case 4:
				events[e] = {
					type: 4, valueStart: r[o + 1], valueEnd: r[o + 2], anchorStart: r[o + 3], anchorEnd: r[o + 4],
					tagStart: r[o + 5], tagEnd: r[o + 6], style: r[o + 7], chomping: r[o + 8], indent: r[o + 9], fast: r[o + 10] === 1,
				};
				break;
			case 5:
				events[e] = { type: 5, anchorStart: r[o + 1], anchorEnd: r[o + 2] };
				break;
			default:
				events[e] = { type: 6 };
		}
	}
	return events;
}

/**
 * Builds the js-yaml-compatible API. `native` provides `loadRaw(source, options)` and
 * `eventsRaw(source, options)` (crates/jsyaml-node or crates/jsyaml-wasm).
 */
function createApi(upstream, native) {
	const { YAMLException, CORE_SCHEMA } = upstream;
	const builtinNames = new Map();
	for (const name of BUILTIN_TAGS) if (upstream[name]) builtinNames.set(upstream[name], name);

	// Export names of the schema's tags, or null if it has tags only JS can run.
	const schemaNames = new WeakMap();
	function describeSchema(schema) {
		if (schema === undefined || schema === CORE_SCHEMA) return [];
		if (!schemaNames.has(schema)) {
			const names = schema.tags.map((tag) => builtinNames.get(tag));
			schemaNames.set(schema, names.includes(undefined) ? null : names);
		}
		return schemaNames.get(schema);
	}

	function fail(result, source, filename) {
		if (result.errorPosition === undefined || result.errorPosition === null) {
			throw new YAMLException(result.errorReason);
		}
		YAMLException.throwAt(source, result.errorPosition, result.errorReason, filename);
	}

	function nativeOptions(options, tags) {
		return {
			tags,
			json: options.json,
			maxTotalMergeKeys: options.maxTotalMergeKeys,
			maxAliases: options.maxAliases,
			maxDepth: options.maxDepth,
		};
	}

	function parseEvents(input, options = {}) {
		const source = String(input);
		if (!isWellFormed(source)) return upstream.parseEvents(source, options);
		const result = native.eventsRaw(source, nativeOptions(options, []));
		if (result.errorReason !== undefined && result.errorReason !== null) fail(result, source, options.filename || '');
		return decodeEventTape(unpackEvents(result));
	}

	function loadDocuments(input, options = {}) {
		const source = String(input);
		if (source.length < NATIVE_MIN_LENGTH || !isWellFormed(source)) return upstream.loadAll(source, options);

		const tags = describeSchema(options.schema);
		if (tags !== null) {
			const result = native.loadRaw(source, nativeOptions(options, tags));
			if (result.fallback !== true) {
				if (result.errorReason !== undefined && result.errorReason !== null) fail(result, source, options.filename || '');
				return result.json !== undefined && result.json !== null ? JSON.parse(result.json) : decodeValueTape(unpackLoaded(result));
			}
			// Valid input the native constructor can't reproduce exactly (lone surrogates,
			// time-zone dependent keys): build it with upstream's constructor below.
		}

		// Custom JS tags (or a fallback): native parser, upstream constructor.
		const events = parseEvents(source, options);
		const { filename = '', schema, json, maxTotalMergeKeys, maxAliases } = options;
		// Only options that are set: an explicit `undefined` would override upstream's defaults.
		const ctor = { source, filename };
		if (schema !== undefined) ctor.schema = schema;
		if (json !== undefined) ctor.json = json;
		if (maxTotalMergeKeys !== undefined) ctor.maxTotalMergeKeys = maxTotalMergeKeys;
		if (maxAliases !== undefined) ctor.maxAliases = maxAliases;
		return upstream.constructFromEvents(events, ctor);
	}

	// Same signatures and messages as js-yaml's src/load.ts.
	function loadAll(input, iteratorOrOptions, options) {
		let iterator = null;
		if (typeof iteratorOrOptions === 'function') iterator = iteratorOrOptions;
		else if (iteratorOrOptions !== null && typeof iteratorOrOptions === 'object') options = iteratorOrOptions;

		const documents = loadDocuments(input, options);
		if (iterator === null) return documents;
		for (const document of documents) iterator(document);
	}

	function load(input, options) {
		const documents = loadDocuments(input, options);
		if (documents.length === 0) throw new YAMLException('expected a document, but the input is empty');
		if (documents.length === 1) return documents[0];
		throw new YAMLException('expected a single document in the stream, but found more');
	}

	return { ...upstream, load, loadAll, parseEvents };
}

module.exports = { createApi, decodeValueTape, decodeEventTape, NATIVE_MIN_LENGTH };
