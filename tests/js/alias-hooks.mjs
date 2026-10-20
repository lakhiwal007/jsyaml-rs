// Module resolution hook: `'js-yaml'` imported from js-yaml's test files resolves to the
// package under test. Everything else, including the package's own import of js-yaml,
// resolves normally.
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

let pkgEntry, testsDir

export function initialize ({ pkg, tests }) {
  pkgEntry = pathToFileURL(path.join(pkg, 'index.mjs')).href
  testsDir = tests + path.sep
}

export function resolve (specifier, context, next) {
  const parent = context.parentURL?.startsWith('file:') ? fileURLToPath(context.parentURL) : ''
  if (specifier === 'js-yaml' && parent.startsWith(testsDir)) return { url: pkgEntry, shortCircuit: true }
  return next(specifier, context)
}
