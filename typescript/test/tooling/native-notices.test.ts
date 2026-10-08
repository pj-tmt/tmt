import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { releaseNoticeTargets, verifyNativeNotices } from '../../scripts/verify-native-notices.mjs';

const { assertDependencyNotices } = (await import(
  new URL('../../scripts/native-artifact-policy.mjs', import.meta.url).href
)) as { assertDependencyNotices(notices: string): void };

const roots: string[] = [];
const targets = [
  'aarch64-apple-darwin',
  'x86_64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-unknown-linux-musl',
];
const products = ['cli', 'squad', 'colab', 'remote', 'driver-herdr'];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fixture() {
  const root = mkdtempSync(path.join(tmpdir(), 'tmt-notices-&-'));
  roots.push(root);
  mkdirSync(path.join(root, '.github'));
  mkdirSync(path.join(root, 'scripts'));
  writeFileSync(
    path.join(root, '.github/components.json'),
    readFileSync(new URL('../../../.github/components.json', import.meta.url))
  );
  writeFileSync(
    path.join(root, 'dist-workspace.toml'),
    `[dist]\ntargets = ${JSON.stringify(targets)}\n`
  );
  writeExecutable(
    path.join(root, 'scripts/build-native-artifact.sh'),
    `#!/bin/sh
set -eu
test "$#" = 3
test "$1" = --rust-notices-only
test "$CI" = true
target=$2
product=$3
printf '%s %s\\n' "$product" "$target" >> calls.txt
mkdir -p rust/target/native-notices
inventory=rust/target/native-notices/THIRD-PARTY-NOTICES.txt
fault=$(cat fault.txt 2>/dev/null || true)
case "$fault" in
  "$product|$target|missing") exit 0 ;;
  "$product|$target|failure") printf 'generator failed\\n' >&2; exit 9 ;;
  "$product|$target|empty") : > "$inventory" ;;
  "$product|$target|placeholder") printf 'Copyright (c) <year> <copyright holders>\\n' > "$inventory" ;;
  *) printf 'Copyright (c) 2026 Example %s %s\\n' "$product" "$target" > "$inventory" ;;
esac
printf 'generator diagnostic\\n' >&2
`,
    0o700
  );
  return root;
}

describe('native dependency notice verification', () => {
  it('generates and retains every active product/target inventory without native builds', () => {
    const root = fixture();
    const results = verifyNativeNotices(root, { report: () => {} });
    const pairs = products.flatMap((product) => targets.map((target) => ({ product, target })));
    expect(results.map(({ product, target }) => ({ product, target }))).toEqual(pairs);
    expect(readFileSync(path.join(root, 'calls.txt'), 'utf8')).toBe(
      pairs.map(({ product, target }) => `${product} ${target}\n`).join('')
    );
    for (const { product, target } of pairs) {
      const file = path.join(root, `rust/target/native-notices/verified/${product}-${target}`);
      expect(readFileSync(`${file}.txt`, 'utf8')).toBe(
        `Copyright (c) 2026 Example ${product} ${target}\n`
      );
      expect(readFileSync(`${file}.log`, 'utf8')).toContain('generator diagnostic');
    }
  });

  it.each(products)(
    'rejects placeholder attribution in %s, including a later target',
    (product) => {
      const root = fixture();
      const target = targets[2];
      writeFileSync(path.join(root, 'fault.txt'), `${product}|${target}|placeholder`);
      expect(() => verifyNativeNotices(root, { report: () => {} })).toThrow(
        `Native notices failed for ${product} (${target}): Dependency notices contain placeholder attribution`
      );
    }
  );

  it.each([
    ['missing', 'ENOENT'],
    ['empty', 'Dependency notices are empty'],
    ['failure', 'generator failed'],
  ])('fails on %s output instead of accepting the previous inventory', (fault, diagnostic) => {
    const root = fixture();
    writeFileSync(path.join(root, 'fault.txt'), `squad|${targets[0]}|${fault}`);
    expect(() => verifyNativeNotices(root, { report: () => {} })).toThrow(diagnostic);
    const evidence = path.join(root, 'rust/target/native-notices/verified');
    expect(readFileSync(path.join(evidence, `squad-${targets[0]}-failure.log`), 'utf8')).toContain(
      diagnostic
    );
    expect(() => readFileSync(path.join(evidence, 'results.json'))).toThrow();
  });

  it('derives product activation from the component map rather than a fixed product list', () => {
    const root = fixture();
    const file = path.join(root, '.github/components.json');
    const map = JSON.parse(readFileSync(file, 'utf8'));
    map.components.squad.release = false;
    map.components['driver-herdr'].release = false;
    writeFileSync(file, JSON.stringify(map));
    const results = verifyNativeNotices(root, { report: () => {} });
    expect([...new Set(results.map(({ product }) => product))]).toEqual(['cli', 'colab', 'remote']);
  });

  it('refuses empty product discovery and unknown active product policies', () => {
    const root = fixture();
    const file = path.join(root, '.github/components.json');
    writeFileSync(
      file,
      JSON.stringify({ components: { helper: { owns: ['.'], release: false } } })
    );
    expect(() => verifyNativeNotices(root)).toThrow('No active native products');
    writeFileSync(
      file,
      JSON.stringify({ components: { helper: { owns: ['.'], package: 'helper' } } })
    );
    expect(() => verifyNativeNotices(root)).toThrow(
      'No native publication policy for component helper'
    );
  });

  it('reads changed release targets from TOML and rejects empty or duplicate declarations', () => {
    const root = fixture();
    expect(releaseNoticeTargets(root)).toEqual(targets);
    for (const list of [[], [targets[0], targets[0]], ['--help']]) {
      writeFileSync(
        path.join(root, 'dist-workspace.toml'),
        `[dist]\ntargets = ${JSON.stringify(list)}\n`
      );
      expect(() => releaseNoticeTargets(root)).toThrow('Release notice targets');
    }
  });

  it.each(['<year>', '<copyright holders>', ''])('shares the archive rejection for %s', (text) => {
    expect(() => assertDependencyNotices(text)).toThrow();
    expect(() => assertDependencyNotices('Copyright (c) 2026 Example')).not.toThrow();
  });
});
