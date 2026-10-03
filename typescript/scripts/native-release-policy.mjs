import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isReleased, parseComponentMap } from './ci-scope.mjs';

// How each native product's GitHub release is published. The CLI release is
// the repository's "latest" release, so
// `releases/latest/download/install.sh` always reaches the CLI installer.
// Extension releases share the repository and must never become latest.
// Alpha status lives in the version and title, not in GitHub's prerelease flag.
// `tmt upgrade` checks the flag with `Product::accepts_prerelease_flag` in
// tmt-core. The flags are pinned in test/tooling/native-release-policy.test.ts
// and in the Rust native_install/release_tests.rs; change all three together.

const PRODUCTS = {
  // Covers the reported pre-companion installation in #1454. Older receipt
  // readability is not a claim that every older release has an upgrade proof.
  cli: { tagPrefix: 'v', prerelease: false, latest: true, upgradeFloor: 'v5.0.0-alpha.36' },
  office: { tagPrefix: 'tmt-office-v', prerelease: true, latest: false },
  squad: { tagPrefix: 'tmt-squad-v', prerelease: true, latest: false },
  'driver-herdr': { tagPrefix: 'tmt-driver-herdr-v', prerelease: true, latest: false },
  remote: { tagPrefix: 'tmt-remote-v', prerelease: true, latest: false },
  colab: { tagPrefix: 'tmt-colab-v', prerelease: true, latest: false },
};

/** Component identities retain their ownership names; native workflows use product keys. */
export function productOfComponent(name) {
  const product = Object.keys(PRODUCTS).find(
    (product) => name === product || name === `tmt-${product}`
  );
  if (!product) throw new Error(`No native publication policy for component ${name}.`);
  return product;
}

/** Resolve product selection to the component map, including prefixed private components. */
export function componentOfProduct(map, product) {
  releasePolicy(product);
  const components = map.components.filter(
    ({ name }) => name === product || name === `tmt-${product}`
  );
  if (components.length !== 1) throw new Error(`Ambiguous or missing component for ${product}.`);
  return components[0];
}

export function isProductReleased(map, product) {
  return isReleased(map, componentOfProduct(map, product).name);
}

/** The publication settings for one product; unknown products are refused. */
export function releasePolicy(product) {
  const policy = PRODUCTS[product];
  if (!policy) throw new Error(`Unknown native product: ${product}`);
  // Publication markers are immutable wire data; proof policy is not a marker field.
  const { tagPrefix, prerelease, latest } = policy;
  return { product, tagPrefix, prerelease, latest };
}

/** The exact CLI source covered by the upgrade proof; other products prove only their previous release. */
export function upgradeSupportFloor(product) {
  releasePolicy(product);
  return PRODUCTS[product].upgradeFloor ?? null;
}

/** The archive name prefix of a product's bundle: `tmt-cli-<target>.tar.gz`, `tmt-office-...`. */
export function archivePrefix(product) {
  releasePolicy(product);
  return product === 'cli' ? 'tmt-cli' : `tmt-${product}`;
}

/** `gh release create` flags that apply the policy to a draft. */
export function releaseFlags(product) {
  const policy = releasePolicy(product);
  return [...(policy.prerelease ? ['--prerelease'] : []), `--latest=${policy.latest}`];
}

/**
 * `gh release edit` flags that publish a draft under the policy. The prerelease flag is always
 * set explicitly: publication must retain the product policy regardless of a draft's flags.
 */
export function publishFlags(product) {
  const policy = releasePolicy(product);
  return ['--draft=false', `--prerelease=${policy.prerelease}`, `--latest=${policy.latest}`];
}

/** The product whose tag this is (`v5.0.0-alpha.9`, `tmt-office-v0.1.0-alpha.4`), or undefined. */
export function productOfTag(tag) {
  return Object.entries(PRODUCTS).find(
    ([, policy]) =>
      tag.startsWith(policy.tagPrefix) && /^\d/.test(tag.slice(policy.tagPrefix.length))
  )?.[0];
}

/**
 * After publication: the repository's latest release must be a CLI tag, so the
 * one-line installer URL resolves to a CLI `install.sh`.
 */
export function checkLatestTag(tag) {
  if (productOfTag(tag) !== 'cli') {
    throw new Error(
      `The latest release ${tag} is not a CLI release; install.sh would not resolve.`
    );
  }
  return true;
}

// The component map also gates preparation, which bypasses draft planning.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [command, product, ...extra] = process.argv.slice(2);
    if (command !== 'require-released' || !product || extra.length) {
      throw new Error('Usage: native-release-policy.mjs require-released <product>');
    }
    const map = parseComponentMap(
      readFileSync(new URL('../../.github/components.json', import.meta.url), 'utf8')
    );
    if (!isProductReleased(map, product)) {
      throw new Error(`${product} is not released (release: false in .github/components.json).`);
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
