'use strict';
// js-yaml-compatible entry point for the native Node-API addon.

const upstream = require('js-yaml');
const native = require('./binding.cjs');
const { createApi } = require('./decode.cjs');

module.exports = createApi(upstream, native);
