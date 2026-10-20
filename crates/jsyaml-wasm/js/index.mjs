// ESM entry point. Wraps js-yaml's ESM build (not the CJS one) so that schemas, tags and
// YAMLException are the same objects the caller imports from 'js-yaml'.
import { createRequire } from 'node:module';
import * as upstream from 'js-yaml';

const require = createRequire(import.meta.url);
const { createApi } = require('./decode.cjs');
const api = createApi(upstream, require('./jsyaml_wasm.js'));

export * from 'js-yaml';
export const { load, loadAll, parseEvents } = api;
