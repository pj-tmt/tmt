#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const DEPENDENCY_PREFIX = 'tmt-e2e-native-archive-v1-';
const NATIVE_BASE =
  'rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073';
const RECIPE_FILES = [
  'typescript/scripts/e2e-dependency-cache.mjs',
  'typescript/test/e2e/dependency-cache.Dockerfile',
  'typescript/test/e2e/Dockerfile',
];

/** Source bytes and ordinary modules are excluded; auto-target paths retain discovery. */
export function dependencyCacheKey(
  root,
  files = execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' })
    .split('\0')
    .filter(Boolean)
) {
  const hash = createHash('sha256').update('Linux/amd64;dev;debug=0;incremental=0\0');
  const inputs = files
    .filter(
      (file) =>
        /(?:^|\/)Cargo\.(?:toml|lock)$/.test(file) ||
        /(?:^|\/)(?:rust-toolchain\.toml|\.cargo\/config(?:\.toml)?)$/.test(file) ||
        RECIPE_FILES.includes(file)
    )
    .sort();
  for (const required of [...RECIPE_FILES, 'rust/Cargo.lock', 'rust/rust-toolchain.toml']) {
    if (!inputs.includes(required)) throw new Error(`Missing E2E dependency input: ${required}`);
  }
  for (const file of inputs)
    hash
      .update(file + '\0')
      .update(fs.readFileSync(path.join(root, file)))
      .update('\0');
  const manifests = inputs.filter((file) => file.endsWith('/Cargo.toml'));
  const autoTargets = files
    .filter(
      (file) =>
        file.endsWith('.rs') &&
        manifests.some((manifest) => {
          const relative = path.posix.relative(path.posix.dirname(manifest), file);
          return /^(?:build\.rs|src\/(?:main|lib)\.rs|(?:src\/bin|examples|tests|benches)\/(?:[^/]+\.rs|[^/]+\/main\.rs))$/.test(
            relative
          );
        })
    )
    .sort();
  for (const file of autoTargets) hash.update(file + '\0');
  return DEPENDENCY_PREFIX + hash.digest('hex');
}

/** Actions owns archive decoding. Refusal is decided before the real build. */
export function inspectDependencyCache(bundle, key) {
  try {
    if (!new RegExp(`^${DEPENDENCY_PREFIX}[a-f0-9]{64}$`).test(key))
      throw new Error('invalid dependency key');
    if (!fs.lstatSync(bundle).isDirectory()) throw new Error('bundle is not a directory');
    if (fs.readFileSync(path.join(bundle, 'cache-key'), 'utf8') !== key + '\n')
      throw new Error('dependency key differs');
    if (fs.readdirSync(bundle).sort().join(',') !== 'cache-key,registry,target')
      throw new Error('unexpected payload locations');
    for (const dir of ['target', 'target/debug', 'target/debug/deps', 'registry']) {
      const location = path.join(bundle, dir);
      if (!fs.lstatSync(location).isDirectory() || fs.readdirSync(location).length === 0)
        throw new Error(`missing or empty ${dir}`);
    }
    if (fs.readdirSync(path.join(bundle, 'target')).join(',') !== 'debug')
      throw new Error('target contains non-dev scratch');
    for (const name of fs.readdirSync(path.join(bundle, 'target/debug/deps'))) {
      const info = fs.lstatSync(path.join(bundle, 'target/debug/deps', name));
      if (/^tmt_adapters-[a-f0-9]+$/.test(name) && info.isFile() && info.mode & 0o111)
        throw new Error('dummy adapter executable retained');
    }
    return { usable: true, reason: 'exact dependency archive' };
  } catch (error) {
    return { usable: false, reason: error instanceof Error ? error.message : String(error) };
  }
}

/** Insert only dependency bytes, keeping every source COPY and real RUN intact. */
export function renderDependencyDockerfile(text) {
  const anchor = 'RUN cargo build --locked --example tmux-probe';
  if (
    !text.startsWith(`FROM ${NATIVE_BASE} AS native-tests\n`) ||
    text.split(anchor).length !== 2 ||
    !text.includes('ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0\n' + anchor)
  )
    throw new Error('E2E native Dockerfile anchor changed');
  return text.replace(
    anchor,
    'COPY --from=tmtdeps /target/ /native/rust/target/\nCOPY --from=tmtdeps /registry/ /usr/local/cargo/registry/\n' +
      anchor
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, bundle, key] = process.argv.slice(2);
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  if (command === 'key') console.log(`key=${dependencyCacheKey(root)}`);
  else if (command === 'stamp') {
    fs.writeFileSync(path.join(bundle, 'cache-key'), key + '\n', { flag: 'wx' });
    const result = inspectDependencyCache(bundle, key);
    if (!result.usable) throw new Error(result.reason);
  } else if (command === 'admit') {
    const result = inspectDependencyCache(bundle, key);
    console.error(`E2E dependency cache: ${result.reason}`);
    console.log(`usable=${result.usable}`);
  } else throw new Error('Expected key, stamp or admit');
}
