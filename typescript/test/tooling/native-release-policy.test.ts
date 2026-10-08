import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const {
  checkLatestTag,
  productOfTag,
  productOfComponent,
  publishFlags,
  releaseFlags,
  releasePolicy,
  isProductReleased,
  isProductRetired,
  archivePrefix,
  upgradeSupportFloor,
} = (await import(
  pathToFileURL(path.join(repositoryRoot, 'scripts', 'native-release-policy.mjs')).href
)) as {
  checkLatestTag: (tag: string) => boolean;
  productOfComponent: (name: string) => string;
  productOfTag: (tag: string) => string | undefined;
  publishFlags: (product: string) => string[];
  releaseFlags: (product: string) => string[];
  releasePolicy: (product: string) => {
    latest: boolean;
    prerelease: boolean;
  };
  isProductReleased: (
    map: { components: { name: string; release?: boolean }[] },
    product: string
  ) => boolean;
  isProductRetired: (product: string) => boolean;
  archivePrefix: (product: string) => string;
  upgradeSupportFloor: (product: string) => string | null;
};

describe('native release publication policy', () => {
  it('keeps Ops release-blocked until CLI registration while retaining Squad history', () => {
    const map = JSON.parse(
      readFileSync(path.join(repositoryRoot, '../.github/components.json'), 'utf8')
    );
    expect(map.components.ops).toMatchObject({
      package: 'tmt-ops',
      predecessor: 'squad',
      release: false,
    });
    for (const key of ['requiresCliSha', 'bootstrapSha', 'initialVersion'])
      expect(map.components.ops).not.toHaveProperty(key);
    expect(map.components).not.toHaveProperty('squad');
    expect(isProductRetired('squad')).toBe(true);
    expect(isProductRetired('ops')).toBe(false);
    expect(archivePrefix('squad')).toBe('tmt-squad');
    expect(archivePrefix('ops')).toBe('tmt-ops');
  });

  it('resolves prefixed components without permitting missing or ambiguous activation evidence', () => {
    expect(
      isProductReleased({ components: [{ name: 'tmt-colab', release: false }] }, 'colab')
    ).toBe(false);
    expect(isProductReleased({ components: [{ name: 'colab', release: true }] }, 'colab')).toBe(
      true
    );
    expect(() => isProductReleased({ components: [] }, 'colab')).toThrow('Ambiguous or missing');
    expect(() =>
      isProductReleased(
        {
          components: [
            { name: 'colab', release: true },
            { name: 'tmt-colab', release: false },
          ],
        },
        'colab'
      )
    ).toThrow('Ambiguous or missing');
  });
  it.each(['cli', 'ops', 'squad', 'office', 'driver-herdr', 'remote', 'colab'])(
    'gates %s through the component release policy',
    (product) => {
      const result = spawnSync(
        process.execPath,
        [
          path.join(repositoryRoot, 'scripts', 'native-release-policy.mjs'),
          'require-released',
          product,
        ],
        { cwd: os.tmpdir(), encoding: 'utf8', timeout: 10_000, maxBuffer: 64 * 1024 }
      );
      expect(result.error).toBeUndefined();
      expect(result.status).toBe(['office', 'ops', 'squad'].includes(product) ? 1 : 0);
      expect(result.stdout).toBe('');
      expect(result.stderr).toBe(
        ['office', 'ops', 'squad'].includes(product)
          ? `${product} is not released (release: false in .github/components.json).\n`
          : ''
      );
    }
  );

  it('declares a single CLI upgrade support floor while extensions retain last-published proof', () => {
    expect(upgradeSupportFloor('cli')).toBe('v5.0.0-alpha.36');
    expect(releasePolicy('cli')).toEqual({
      product: 'cli',
      tagPrefix: 'v',
      prerelease: false,
      latest: true,
    });
    for (const product of ['office', 'ops', 'squad', 'driver-herdr', 'remote', 'colab'])
      expect(upgradeSupportFloor(product)).toBeNull();
    expect(() => upgradeSupportFloor('unknown')).toThrow('Unknown native product');
  });

  it('maps ownership names to workflow products without accepting arbitrary prefixes', () => {
    expect(productOfComponent('tmt-remote')).toBe('remote');
    expect(productOfComponent('tmt-colab')).toBe('colab');
    expect(productOfComponent('driver-herdr')).toBe('driver-herdr');
    expect(() => productOfComponent('colab-client')).toThrow('No native publication policy');
    expect(productOfTag('tmt-remote-v0.1.0-alpha.1')).toBe('remote');
  });
  it('makes only the CLI the latest release', () => {
    expect(releaseFlags('cli')).toEqual(['--latest=true']);
    for (const extension of ['office', 'ops', 'squad', 'driver-herdr', 'remote', 'colab']) {
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
        ['cli', 'office', 'ops', 'squad', 'driver-herdr', 'remote', 'colab'].map((product) => [
          product,
          releasePolicy(product).prerelease,
        ])
      )
    ).toEqual({
      cli: false,
      office: true,
      ops: true,
      squad: true,
      'driver-herdr': true,
      remote: true,
      colab: true,
    });
    expect(releaseFlags('cli')).not.toContain('--prerelease');
    for (const extension of ['office', 'ops', 'squad', 'driver-herdr', 'remote', 'colab']) {
      expect(releaseFlags(extension)).toContain('--prerelease');
    }
  });

  it('publishes a draft with every flag explicit, since each draft starts with component publication policy', () => {
    expect(publishFlags('cli')).toEqual(['--draft=false', '--prerelease=false', '--latest=true']);
    for (const extension of ['office', 'ops', 'squad', 'driver-herdr', 'remote', 'colab']) {
      expect(publishFlags(extension)).toEqual([
        '--draft=false',
        '--prerelease=true',
        '--latest=false',
      ]);
    }
    expect(() => publishFlags('unknown')).toThrow();
  });

  it('accepts only a CLI tag as the published latest release', () => {
    expect(checkLatestTag('v5.0.0-alpha.7')).toBe(true);
    for (const tag of [
      'tmt-office-v0.1.0-alpha.4',
      'tmt-ops-v0.1.0-alpha.2',
      'tmt-squad-v0.1.0-alpha.2',
      'tmt-driver-herdr-v99.0.0-alpha.1',
      'tmt-remote-v0.1.0-alpha.1',
      'tmt-colab-v0.1.0-alpha.1',
      'install',
      'vnext',
    ]) {
      expect(() => checkLatestTag(tag)).toThrow(/not a CLI release/);
    }
  });

  it('names the product a release tag belongs to, and only for tags the policy publishes', () => {
    expect(productOfTag('v5.0.0-alpha.9')).toBe('cli');
    expect(productOfTag('tmt-office-v0.1.0-alpha.4')).toBe('office');
    expect(productOfTag('tmt-ops-v0.1.0-alpha.2')).toBe('ops');
    expect(productOfTag('tmt-squad-v0.1.0-alpha.2')).toBe('squad');
    expect(productOfTag('tmt-driver-herdr-v0.1.0-alpha.1')).toBe('driver-herdr');
    expect(productOfTag('tmt-colab-v0.1.0-alpha.1')).toBe('colab');
    for (const tag of [
      'install',
      'vnext',
      'tmt-office-vnext',
      'tmt-relay-v1.0.0',
      'v',
      '5.0.0',
      '',
    ]) {
      expect(productOfTag(tag), tag).toBeUndefined();
    }
  });
});
