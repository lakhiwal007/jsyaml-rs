'use strict';
// Loads the native addon: a local build (`jsyaml.node` next to this file, as produced by
// scripts/build-node.sh), else the per-platform package npm installed through
// optionalDependencies (`jsyaml-node-<platform>`, see scripts/platforms.json).

const path = require('node:path');

function libc() {
	if (process.platform !== 'linux') return '';
	// glibc reports its version; musl doesn't.
	const report = typeof process.report?.getReport === 'function' ? process.report.getReport() : null;
	return report?.header?.glibcVersionRuntime ? 'gnu' : 'musl';
}

function platformPackage() {
	const { platform, arch } = process;
	if (platform === 'win32') return `jsyaml-node-win32-${arch}-msvc`;
	if (platform === 'linux') return `jsyaml-node-linux-${arch}-${libc()}`;
	return `jsyaml-node-${platform}-${arch}`;
}

function load() {
	const errors = [];
	for (const candidate of [path.join(__dirname, 'jsyaml.node'), platformPackage()]) {
		try {
			return require(candidate);
		} catch (error) {
			if (error.code !== 'MODULE_NOT_FOUND') throw error;
			errors.push(candidate);
		}
	}
	throw new Error(
		`jsyaml-node: no native binary for ${process.platform}-${process.arch} (looked for ${errors.join(', ')}). ` +
		'Reinstall without --no-optional / --omit=optional, or use the WebAssembly build: npm install jsyaml-wasm',
	);
}

module.exports = load();
