// Writes the benchmark inputs to bench/data/. Deterministic (seeded), so runs compare.
// Shapes are the real-world YAML that is big enough for parse time to matter.

import fs from 'node:fs'
import path from 'node:path'

const out = path.join(import.meta.dirname, 'data')
fs.mkdirSync(out, { recursive: true })

let seed = 42
const rand = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31)
const pick = (xs) => xs[Math.floor(rand() * xs.length)]
const word = () => pick(['alpha', 'beta', 'gamma', 'delta', 'web', 'api', 'worker', 'cache', 'db', 'queue'])

function write (name, text) {
  fs.writeFileSync(path.join(out, name), text)
  console.log(`${name}: ${(text.length / 1e6).toFixed(1)} MB`)
}

// Kubernetes manifests: many small documents, block style, quoted strings.
function k8s (docs) {
  let s = ''
  for (let i = 0; i < docs; i++) {
    const name = `${word()}-${i}`
    s += `---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${name}
  namespace: ${word()}
  labels: {app: ${name}, tier: "${word()}", version: "v${i % 7}"}
spec:
  replicas: ${1 + (i % 5)}
  selector:
    matchLabels:
      app: ${name}
  template:
    spec:
      containers:
        - name: ${name}
          image: 'registry.example.com/${word()}:${i % 30}.${i % 9}'
          ports: [${8000 + (i % 100)}, 443]
          resources:
            limits: {cpu: "${(i % 4) + 0.5}", memory: ${128 * (1 + (i % 8))}Mi}
          env:
            - {name: MODE, value: production}
            - {name: FEATURE_${i % 13}, value: "true"}
`
  }
  return s
}

// OpenAPI-like: one deep document, long descriptions, folded scalars.
function openapi (paths) {
  let s = 'openapi: 3.1.0\ninfo:\n  title: Example\n  version: 1.0.0\npaths:\n'
  for (let i = 0; i < paths; i++) {
    s += `  /v1/${word()}/${i}:
    get:
      operationId: get${i}
      description: >
        Returns the ${word()} resource number ${i}. This text is folded
        across lines the way API descriptions usually are, and it is long
        enough to exercise the folded scalar path.
      parameters:
        - {name: id, in: path, required: true, schema: {type: integer, minimum: 0}}
        - name: filter
          in: query
          schema: {type: string, enum: [${word()}, ${word()}, ${word()}]}
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Item${i % 50}'}
        '404': {description: Not found}
`
  }
  return s
}

// Lockfile-like: a huge flat-ish mapping of short keys and quoted values.
function lockfile (packages) {
  let s = "lockfileVersion: '9.0'\npackages:\n"
  for (let i = 0; i < packages; i++) {
    s += `  '${word()}-${i}@${i % 10}.${i % 7}.${i % 3}':
    resolution: {integrity: sha512-${Buffer.from(String(i * 7919)).toString('base64')}${'x'.repeat(40)}}
    engines: {node: '>=18'}
    dependencies:
      ${word()}-${(i * 31) % packages}: ${i % 10}.0.0
      ${word()}-${(i * 17) % packages}: ^${i % 5}.1.0
`
  }
  return s
}

// docker-compose / CI style: anchors, aliases and merge keys.
function anchors (services) {
  let s = 'x-defaults: &defaults\n  restart: unless-stopped\n  logging: {driver: json-file, options: {max-size: 10m}}\n  environment: &env {TZ: UTC, LOG_LEVEL: info}\nservices:\n'
  for (let i = 0; i < services; i++) {
    s += `  svc${i}:
    <<: *defaults
    image: ${word()}:${i % 20}
    environment: *env
    depends_on: [svc${Math.max(0, i - 1)}]
`
  }
  return s
}

write('k8s.yaml', k8s(20_000))
write('openapi.yaml', openapi(12_000))
write('lockfile.yaml', lockfile(40_000))
write('anchors.yaml', anchors(30_000))
