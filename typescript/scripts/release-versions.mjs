// Versions and published releases of the native products, in one place: the upgrade proof and the
// publication gates both need "the newest published release below or above this one".
import { productOfTag, releasePolicy } from './native-release-policy.mjs';

const VERSION = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

function parseVersion(text) {
  const match = VERSION.exec(text);
  if (!match) throw new Error(`${text} is not a version.`);
  return {
    core: match.slice(1, 4).map(Number),
    pre: match[4] === undefined ? [] : match[4].split('.'),
  };
}

/** Semantic Versioning precedence: -1, 0 or 1. A pre-release is lower than its release. */
export function compareVersions(left, right) {
  const a = parseVersion(left);
  const b = parseVersion(right);
  for (let index = 0; index < 3; index += 1) {
    if (a.core[index] !== b.core[index]) return a.core[index] < b.core[index] ? -1 : 1;
  }
  if (a.pre.length === 0 || b.pre.length === 0) {
    return a.pre.length === b.pre.length ? 0 : a.pre.length === 0 ? 1 : -1;
  }
  for (let index = 0; index < Math.min(a.pre.length, b.pre.length); index += 1) {
    const [x, y] = [a.pre[index], b.pre[index]];
    if (x === y) continue;
    const numeric = [/^\d+$/.test(x), /^\d+$/.test(y)];
    if (numeric[0] && numeric[1]) return Number(x) < Number(y) ? -1 : 1;
    if (numeric[0] !== numeric[1]) return numeric[0] ? -1 : 1;
    return x < y ? -1 : 1;
  }
  return Math.sign(a.pre.length - b.pre.length);
}

const ALPHA = /^\d+\.\d+\.\d+-alpha\.\d+$/;

/**
 * Whether a version is an alpha release, `X.Y.Z-alpha.N`: the one channel the pipeline publishes
 * by itself. A stable version or any other pre-release label is the owner's to publish.
 */
export function isAlphaVersion(version) {
  return ALPHA.test(version);
}

/** The version a product's tag names: `v5.0.0-alpha.9` and `tmt-office-v0.1.0-alpha.4`. */
export function versionOfTag(tag, product) {
  if (productOfTag(tag) !== product) throw new Error(`${tag} is not a ${product} tag.`);
  return tag.slice(releasePolicy(product).tagPrefix.length);
}

/** The published (not draft) releases of a product, newest version first. */
export function publishedReleases(releases, product) {
  return releases
    .filter((release) => release.draft !== true && productOfTag(release.tag_name) === product)
    .map((release) => ({ release, version: versionOfTag(release.tag_name, product) }))
    .sort((left, right) => compareVersions(right.version, left.version))
    .map(({ release }) => release);
}
