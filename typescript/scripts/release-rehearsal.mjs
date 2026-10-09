// Selects which products the pre-merge release rehearsal packages (#1581). It reuses the component
// map for ownership and native release policy for product identities; ci-scope cannot own it
// because native release policy already imports ci-scope.
import { appendFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { componentMap, readChangedCiSelection, releasedComponentNamesOfPath } from './ci-scope.mjs';
import { productOfComponent } from './native-release-policy.mjs';

// Inputs every active product's packaging depends on: dependency and toolchain pins, notice and
// dist configuration, ownership data, packaging and verification scripts, and the workflows and
// actions that run the prepare pipeline. Bundled frontend sources are deliberately absent: their
// own PR checks build them, and a frontend edit alone must not select a rehearsal.
const SHARED_INPUTS = new Set([
  'rust/Cargo.lock',
  'rust/Cargo.toml',
  'rust/rust-toolchain.toml',
  'rust/about.toml',
  'rust/about.hbs',
  'dist-workspace.toml',
  'typescript/pnpm-lock.yaml',
  '.github/components.json',
  '.github/release-parity.json',
  '.github/workflows/native-release.yml',
  '.github/workflows/native-release-bundle.yml',
  '.github/workflows/native-release-prepare.yml',
  '.github/workflows/native-release-upgrade.yml',
  '.github/workflows/native-release-upgrade-prove.yml',
  '.github/workflows/release-rehearsal.yml',
]);
const SHARED_PATTERNS = [
  /^rust\/licenses\//,
  /^rust\/(?:crates\/)?[^/]+\/Cargo\.toml$/,
  /^\.github\/actions\/(?:inject-release-version|setup-tooling|apt-install|warm-xcrun)\//,
  /^scripts\/(?:build-native-artifact|native-cargo|native-bootstrap|run-native-verification|retry-command)\.sh$/,
  /^typescript\/scripts\/(?:native-application-schema|native-artifact-policy|native-bootstrap|native-release-policy|generate-native-bootstrap|release-policy|release-version-injection|release-versions|verify-native-(?:artifact|bootstrap|notices|installation|extension-upgrade|driver-upgrade)|release-upgrade|publication-gates|verify-colab-app-entries|packed-command|cargo-workspace)\.mjs$/,
];
// Extension manifests belong to one product, so component attribution picks the products.
const PRODUCT_MANIFEST = /^extensions\/(?:.*\/)?Cargo\.toml$/;

/** Released native products, from the component map and native release policy. */
export function activeProducts(map = componentMap()) {
  return map.components
    .filter(
      (component) =>
        component.package && component.release !== false && component.releaseStatus !== 'never'
    )
    .map((component) => productOfComponent(component.name))
    .sort();
}

/**
 * Products to rehearse for the changed paths: all active products for a shared release input (or an
 * empty path list, kept conservative), otherwise the products that own a changed extension manifest.
 */
export function selectReleaseRehearsal(paths, map = componentMap()) {
  const all = activeProducts(map);
  if (
    paths.length === 0 ||
    paths.some((path) => SHARED_INPUTS.has(path) || SHARED_PATTERNS.some((p) => p.test(path)))
  )
    return all;
  const selected = new Set();
  for (const path of paths) {
    if (!PRODUCT_MANIFEST.test(path)) continue;
    for (const name of releasedComponentNamesOfPath(path, map)) {
      const product = productOfComponent(name);
      if (all.includes(product)) selected.add(product);
    }
  }
  return [...selected].sort();
}

export function runReleaseRehearsal(args, { cwd, stdout, stderr, summaryFile }) {
  const report = (text) => {
    stderr.write(text);
    if (summaryFile) appendFileSync(summaryFile, text);
  };
  if (args.length === 1 && args[0] === 'all') {
    stdout.write(`products=${JSON.stringify(activeProducts())}\n`);
    return;
  }
  if (args.length !== 3 || args[0] !== 'select')
    throw new Error('Usage: release-rehearsal.mjs select <base> <head> | all');
  const { paths } = readChangedCiSelection(args[1], args[2], cwd);
  const products = selectReleaseRehearsal(paths);
  report(
    `### Release rehearsal selection\n\n${
      products.length
        ? `Rehearsing ${products.join(', ')} on the four native targets.`
        : 'No release input changed; no rehearsal selected.'
    }\n`
  );
  stdout.write(
    `release_rehearsal=${products.length > 0}\nrelease_rehearsal_products=${JSON.stringify(products)}\n`
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runReleaseRehearsal(process.argv.slice(2), {
      cwd: fileURLToPath(new URL('../../', import.meta.url)),
      stdout: process.stdout,
      stderr: process.stderr,
      summaryFile: process.env.GITHUB_STEP_SUMMARY,
    });
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
