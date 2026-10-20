// Registers alias-hooks.mjs (see upstream.mjs).
import { register } from 'node:module'

register('./alias-hooks.mjs', import.meta.url, {
  data: { pkg: process.env.JSYAML_PKG, tests: process.env.JSYAML_UPSTREAM_TESTS }
})
