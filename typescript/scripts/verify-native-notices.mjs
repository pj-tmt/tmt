import assert from 'node:assert/strict';
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isReleased, parseComponentMap } from './ci-scope.mjs';
import { productOfComponent } from './native-release-policy.mjs';
import { assertDependencyNotices } from './native-artifact-policy.mjs';
import { runPackedCommand } from './packed-command.mjs';

/** Use the release target declaration, without building cargo-dist or native products. */
export function releaseNoticeTargets(root, runner = runPackedCommand) {
  const targets = JSON.parse(
    runner(
      'python3',
      [
        '-c',
        'import json,sys,tomllib; print(json.dumps(tomllib.load(open(sys.argv[1], "rb"))["dist"]["targets"]))',
        resolve(root, 'dist-workspace.toml'),
      ],
      { cwd: root, env: process.env }
    )
  );
  assert(
    Array.isArray(targets) &&
      targets.length > 0 &&
      targets.every(
        (target) => typeof target === 'string' && /^[a-z0-9_]+(?:-[a-z0-9_]+){2,}$/.test(target)
      ) &&
      new Set(targets).size === targets.length,
    'Release notice targets must be a nonempty unique list of target triples'
  );
  return targets;
}

/** Preserve each generated inventory before the next builder invocation replaces it. */
export function verifyNativeNotices(
  root,
  { runner = runPackedCommand, report = console.log } = {}
) {
  root = resolve(root);
  const map = parseComponentMap(readFileSync(resolve(root, '.github/components.json'), 'utf8'));
  const products = map.components
    .filter((component) => component.package && isReleased(map, component.name))
    .map((component) => productOfComponent(component.name));
  assert(products.length > 0, 'No active native products to verify');
  const targets = releaseNoticeTargets(root, runner);
  const output = resolve(root, 'rust/target/native-notices');
  const inventory = resolve(output, 'THIRD-PARTY-NOTICES.txt');
  const evidence = resolve(output, 'verified');
  rmSync(evidence, { recursive: true, force: true });
  mkdirSync(evidence, { recursive: true });
  const results = [];
  for (const product of products) {
    for (const target of targets) {
      const name = `${product}-${target}`;
      // A successful command that omitted its output must not accept the previous product's file.
      rmSync(inventory, { force: true });
      const started = performance.now();
      try {
        const diagnostics = runner(
          'sh',
          [
            '-c',
            'exec "$@" 2>&1',
            'native-notices',
            resolve(root, 'scripts/build-native-artifact.sh'),
            '--rust-notices-only',
            target,
            product,
          ],
          { cwd: root, env: { ...process.env, CI: 'true' }, timeoutMs: 120_000 }
        );
        writeFileSync(resolve(evidence, `${name}.log`), diagnostics);
        const notices = readFileSync(inventory, 'utf8');
        copyFileSync(inventory, resolve(evidence, `${name}.txt`));
        assertDependencyNotices(notices);
      } catch (error) {
        writeFileSync(
          resolve(evidence, `${name}-failure.log`),
          `${error.stack}\n${error.cause?.stdout ?? ''}\n${error.cause?.stderr ?? ''}`
        );
        throw new Error(`Native notices failed for ${product} (${target}): ${error.message}`, {
          cause: error,
        });
      }
      const seconds = (performance.now() - started) / 1000;
      results.push({ product, target, seconds });
      report(`Verified dependency notices: ${product} (${target}) in ${seconds.toFixed(2)}s`);
    }
  }
  writeFileSync(resolve(evidence, 'results.json'), JSON.stringify(results, null, 2) + '\n');
  return results;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert(process.argv.length === 2, 'Usage: node typescript/scripts/verify-native-notices.mjs');
  verifyNativeNotices(fileURLToPath(new URL('../../', import.meta.url)));
}
