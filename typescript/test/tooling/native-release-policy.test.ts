import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const { checkLatestTag, releaseFlags, releasePolicy } = (await import(
  pathToFileURL(path.join(repositoryRoot, 'scripts', 'native-release-policy.mjs')).href
)) as {
  checkLatestTag: (tag: string) => boolean;
  releaseFlags: (product: string) => string[];
  releasePolicy: (product: string) => { latest: boolean; prerelease: boolean };
};

describe('native release publication policy', () => {
  it('makes only the CLI the latest release', () => {
    expect(releaseFlags('cli')).toEqual(['--latest=true']);
    for (const extension of ['office', 'squad']) {
      expect(releasePolicy(extension).latest).toBe(false);
      expect(releaseFlags(extension)).toContain('--latest=false');
    }
    expect(() => releasePolicy('unknown')).toThrow();
  });

  it('publishes the prerelease flags the native updater accepts', () => {
    // Pinned on both sides: tmt upgrade accepts these through
    // Product::accepts_prerelease_flag (tmt-core), and
    // rust/crates/tmt-adapters/src/native_install/release_tests.rs pins the
    // same table. Change them together; neither side reads the other.
    expect(
      Object.fromEntries(
        ['cli', 'office', 'squad'].map((product) => [product, releasePolicy(product).prerelease])
      )
    ).toEqual({ cli: false, office: true, squad: true });
    expect(releaseFlags('cli')).not.toContain('--prerelease');
    for (const extension of ['office', 'squad']) {
      expect(releaseFlags(extension)).toContain('--prerelease');
    }
  });

  it('accepts only a CLI tag as the published latest release', () => {
    expect(checkLatestTag('v5.0.0-alpha.7')).toBe(true);
    for (const tag of [
      'tmt-office-v0.1.0-alpha.4',
      'tmt-squad-v0.1.0-alpha.2',
      'install',
      'vnext',
    ]) {
      expect(() => checkLatestTag(tag)).toThrow(/not a CLI release/);
    }
  });
});
