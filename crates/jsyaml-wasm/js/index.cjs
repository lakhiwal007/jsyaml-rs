'use strict';
// js-yaml-compatible entry point for the WebAssembly build.

const upstream = require('js-yaml');
const native = require('./jsyaml_wasm.js');
const { createApi } = require('./decode.cjs');

module.exports = createApi(upstream, native);
