import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import type { ComponentMap } from '../../scripts/ci-scope.mjs';
import type { CutMetadata } from '../../scripts/release-cut.mjs';
import type { DraftRelease } from '../../scripts/release-draft-assets.mjs';

// Copy actual tooling once, changing only its private registry data. Synthetic product
// names never enter the real map or registry; every history/staging function stays intact.
let root: string;
let scope: typeof import('../../scripts/ci-scope.mjs');
let cut: typeof import('../../scripts/release-cut.mjs');
let upgrade: typeof import('../../scripts/release-upgrade.mjs');
let policy: {
  productOfTag(tag: string): string | undefined;
  archivePrefix(product: string): string;
  isProductReleased(map: ComponentMap, product: string): boolean;
  releasePolicy(product: string): {
    product: string;
    tagPrefix: string;
    prerelease: boolean;
    latest: boolean;
  };
};
const tooling = fileURLToPath(new URL('../../', import.meta.url));
beforeAll(async () => {
  root = mkdtempSync(path.join(tmpdir(), 'release-predecessor-'));
  const scripts = path.join(root, 'typescript/scripts');
  cpSync(path.join(tooling, 'scripts'), scripts, { recursive: true });
  symlinkSync(
    path.join(tooling, 'node_modules'),
    path.join(root, 'typescript/node_modules'),
    'dir'
  );
  const file = path.join(scripts, 'native-release-policy.mjs');
  const original = readFileSync(file, 'utf8');
  expect(original.split('const PRODUCTS = {')).toHaveLength(2);
  writeFileSync(
    file,
    original.replace(
      'const PRODUCTS = {',
      `const PRODUCTS = {
    'fixture-old': { tagPrefix: 'tmt-fixture-old-v', prerelease: true, latest: false, retired: true },
    'fixture-new': { tagPrefix: 'tmt-fixture-new-v', prerelease: true, latest: false },
    'fixture-active': { tagPrefix: 'tmt-fixture-active-v', prerelease: true, latest: false },
    'fixture-next': { tagPrefix: 'tmt-fixture-next-v', prerelease: true, latest: false },`
    )
  );
  const load = (name: string) => import(pathToFileURL(path.join(scripts, `${name}.mjs`)).href);
  scope = await load('ci-scope');
  cut = await load('release-cut');
  upgrade = await load('release-upgrade');
  policy = await load('native-release-policy');
  mkdirSync(path.join(root, '.github'));
  writeFileSync(
    path.join(root, '.github/components.json'),
    JSON.stringify({ components: definitions })
  );
});
afterAll(() => {
  if (root) rmSync(root, { recursive: true, force: true });
});

const sha = (n: number) => n.toString(16).padStart(40, '0');
const definitions = {
  'fixture-new': { owns: ['fixture/new'], package: 'tmt-fixture-new', predecessor: 'fixture-old' },
  'fixture-active': { owns: ['fixture/active'], package: 'tmt-fixture-active' },
  'fixture-next': {
    owns: ['fixture/next'],
    package: 'tmt-fixture-next',
    predecessor: 'fixture-new',
  },
};
const mapOf = (components = definitions) => scope.parseComponentMap(JSON.stringify({ components }));
const published = (product: string, n: number, at = 1) =>
  ({
    id: n,
    created_at: '2026-10-08T00:00:00Z',
    tag_name: `tmt-${product}-v0.1.0-alpha.${n}`,
    draft: false,
    target_commitish: sha(at),
  }) satisfies DraftRelease;
const draft = (product: string, n: number, at = 2) => ({
  ...published(product, n, at),
  draft: true,
});
function historyGit(releases: NonNullable<CutMetadata['releases']>, tags: string[] = []) {
  return (args: string[]) => {
    if (args[0] === 'tag')
      return tags.filter((tag) => tag.startsWith(args[2].slice(0, -1))).join('\n');
    if (args[0] === 'rev-parse') {
      const tag = args[2].slice('refs/tags/'.length, -'^{commit}'.length);
      const release = releases.find((r) => r.tag_name === tag);
      if (!release) throw new Error(`No literal tag fixture ${tag}`);
      return release.target_commitish ?? '';
    }
    if (args[0] === 'merge-base') {
      if (args[3] === 'refs/remotes/origin/main' || args[2] <= args[3]) return '';
      throw new Error('not an ancestor', { cause: { status: 1 } });
    }
    if (args[0] === 'log') return `${sha(4)}\0feat: successor feature\n\0\n`;
    if (args[0] === 'show') return sha(3);
    if (args[0] === 'diff') return 'fixture/new/feature.rs\0';
    throw new Error(`Unexpected Git call ${args}`);
  };
}

describe('component predecessor validation', () => {
  it('accepts exact retired identities without retaining their old component record', () => {
    expect(mapOf().components[0].predecessor).toBe('fixture-old');
    expect(policy.productOfTag('tmt-fixture-old-v0.1.0-alpha.19')).toBe('fixture-old');
    expect(policy.archivePrefix('fixture-old')).toBe('tmt-fixture-old');
    expect(policy.releasePolicy('fixture-old')).toEqual({
      product: 'fixture-old',
      tagPrefix: 'tmt-fixture-old-v',
      prerelease: true,
      latest: false,
    });
    expect(policy.isProductReleased(mapOf(), 'fixture-old')).toBe(false);
    expect(policy.isProductReleased(mapOf(), 'fixture-new')).toBe(true);
  });
  it('accepts an existing released packaged predecessor', () => {
    expect(
      mapOf({
        ...definitions,
        'fixture-new': { ...definitions['fixture-new'], predecessor: 'fixture-active' },
      }).components[0].predecessor
    ).toBe('fixture-active');
  });
  it.each([
    ['', 'needs a package and product key'],
    ['unknown', 'Unknown native product: unknown'],
    ['tmt-fixture-old', 'Unknown native product: tmt-fixture-old'],
    ['fixture-new', 'cannot be its own predecessor'],
    [null, 'needs a package and product key'],
    [7, 'needs a package and product key'],
  ] as const)('refuses invalid predecessor %s', (predecessor, message) => {
    expect(() =>
      scope.parseComponentMap(
        JSON.stringify({
          components: {
            ...definitions,
            'fixture-new': { ...definitions['fixture-new'], predecessor },
          },
        })
      )
    ).toThrow(message);
  });
  it('requires the declaring package and an unambiguous product identity', () => {
    expect(() =>
      scope.parseComponentMap(
        JSON.stringify({
          components: {
            ...definitions,
            'fixture-new': { ...definitions['fixture-new'], package: undefined },
          },
        })
      )
    ).toThrow('needs a package');
    expect(() =>
      scope.parseComponentMap(
        JSON.stringify({
          components: { ...definitions, 'tmt-fixture-new': { ...definitions['fixture-new'] } },
        })
      )
    ).toThrow('Ambiguous');
  });
  it.each([
    { release: false },
    { package: undefined },
    { release: false, releaseStatus: 'parked' },
    { release: false, releaseStatus: 'never' },
  ])('refuses an inactive or unpackaged predecessor %j', (change) => {
    expect(() =>
      scope.parseComponentMap(
        JSON.stringify({
          components: {
            ...definitions,
            'fixture-new': { ...definitions['fixture-new'], predecessor: 'fixture-active' },
            'fixture-active': { ...definitions['fixture-active'], ...change },
          },
        })
      )
    ).toThrow('released or retired');
  });
  it('refuses missing active predecessor evidence and multi-product cycles', () => {
    expect(() =>
      scope.parseComponentMap(
        JSON.stringify({
          components: {
            'fixture-new': { ...definitions['fixture-new'], predecessor: 'fixture-active' },
          },
        })
      )
    ).toThrow('missing');
    expect(() =>
      mapOf({
        ...definitions,
        'fixture-new': { ...definitions['fixture-new'], predecessor: 'fixture-next' },
      })
    ).toThrow('cycle');
  });
  it('retained retired records must stay inactive and cannot hide a cycle', () => {
    const old = { owns: ['fixture/old'], package: 'tmt-fixture-old' };
    expect(() =>
      scope.parseComponentMap(
        JSON.stringify({ components: { ...definitions, 'fixture-old': old } })
      )
    ).toThrow('release: false');
    expect(() =>
      scope.parseComponentMap(
        JSON.stringify({
          components: {
            ...definitions,
            'fixture-old': { ...old, release: false, predecessor: 'fixture-new' },
          },
        })
      )
    ).toThrow('cycle');
  });
});

describe('predecessor allocation boundaries', () => {
  it('inherits the published SHA/tag while every old and new draft/orphan allocation reserves numbers', () => {
    const releases = [
      published('fixture-old', 9),
      draft('fixture-old', 12),
      draft('fixture-new', 11, 3),
      { ...draft('fixture-old', 14), assets: [{ name: 'verification-failed.json' }] },
    ];
    const history = cut.releaseCutHistory({
      releases,
      product: 'fixture-new',
      cut: sha(4),
      git: historyGit(releases, ['tmt-fixture-new-v0.1.0-alpha.15']),
      map: mapOf(),
    });
    expect(history).toEqual({
      highestVersion: '0.1.0-alpha.15',
      previous: { tag: 'tmt-fixture-old-v0.1.0-alpha.9', sha: sha(1) },
      previousAllocated: { tag: 'tmt-fixture-new-v0.1.0-alpha.11', sha: sha(3) },
    });
    expect(cut.nextAlphaVersion(history.highestVersion!)).toBe('0.1.0-alpha.16');
    expect(
      cut.releaseCutHistory({
        releases,
        product: 'fixture-new',
        cut: sha(4),
        git: historyGit(releases),
        map: mapOf(),
      }).highestVersion
    ).toBe('0.1.0-alpha.14');
  });
  it('own published history takes over completely, ignoring higher predecessor reservations', () => {
    const releases = [
      published('fixture-old', 9),
      draft('fixture-old', 99),
      published('fixture-new', 16, 2),
      draft('fixture-new', 17, 3),
    ];
    expect(
      cut.releaseCutHistory({
        releases,
        product: 'fixture-new',
        cut: sha(4),
        git: historyGit(releases),
        map: mapOf(),
      })
    ).toEqual({
      highestVersion: '0.1.0-alpha.17',
      previous: { tag: 'tmt-fixture-new-v0.1.0-alpha.16', sha: sha(2) },
      previousAllocated: { tag: 'tmt-fixture-new-v0.1.0-alpha.17', sha: sha(3) },
    });
  });
  it('follows a valid unpublished chain but stops at its own published history', () => {
    const releases = [published('fixture-old', 9), draft('fixture-new', 12)];
    expect(
      cut.releaseCutHistory({
        releases,
        product: 'fixture-next',
        cut: sha(4),
        git: historyGit(releases),
        map: mapOf(),
      })
    ).toEqual({
      highestVersion: '0.1.0-alpha.12',
      previous: { tag: 'tmt-fixture-old-v0.1.0-alpha.9', sha: sha(1) },
      previousAllocated: { tag: 'tmt-fixture-new-v0.1.0-alpha.12', sha: sha(2) },
    });
  });
  it('a publication recheck excludes its own candidate tag before selecting the predecessor boundary', () => {
    const releases = [published('fixture-old', 9), published('fixture-new', 10, 4)];
    expect(
      cut.releaseCutHistory({
        releases,
        product: 'fixture-new',
        excludeTag: 'tmt-fixture-new-v0.1.0-alpha.10',
        cut: sha(4),
        git: historyGit(releases),
        map: mapOf(),
      }).previous
    ).toEqual({ tag: 'tmt-fixture-old-v0.1.0-alpha.9', sha: sha(1) });
  });
  it('inherited ancestry still refuses unavailable Git evidence', () => {
    const releases = [published('fixture-old', 9)];
    expect(() =>
      cut.releaseCutHistory({
        releases,
        product: 'fixture-new',
        cut: sha(4),
        map: mapOf(),
        git: (args) => {
          if (args[0] === 'merge-base')
            throw new Error('history unavailable', { cause: { status: 128 } });
          return historyGit(releases)(args);
        },
      })
    ).toThrow('history unavailable');
  });
  it('plans the next alpha without a bootstrap seed and never plans a retired product', async () => {
    const releases = [published('fixture-old', 9), draft('fixture-old', 12)];
    const map = scope.parseComponentMap(
      JSON.stringify({
        components: {
          'fixture-new': definitions['fixture-new'],
          'fixture-old': { owns: ['fixture/old'], package: 'tmt-fixture-old', release: false },
        },
      })
    );
    const result = await cut.planReleaseCuts({
      metadata: {
        schema: 1,
        repository: 'pj-tmt/tmt',
        cut: sha(4),
        draftVisibility: 'trusted',
        releases,
      },
      git: historyGit(releases),
      map,
      date: '2026-10-08',
    });
    expect(result.components).toHaveLength(1);
    expect(result.components[0]).toMatchObject({
      product: 'fixture-new',
      status: 'proposed',
      version: '0.1.0-alpha.13',
      tag: 'tmt-fixture-new-v0.1.0-alpha.13',
      previous: sha(1),
      previousTag: 'tmt-fixture-old-v0.1.0-alpha.9',
    });
    expect(result.components[0].notes).toContain('tmt-fixture-old-v0.1.0-alpha.9');
    await expect(
      cut.planReleaseCuts({
        metadata: { schema: 1, repository: 'pj-tmt/tmt', cut: sha(4) },
        map,
        git: historyGit(releases),
        versions: { 'fixture-old': '0.1.0-alpha.20' },
      })
    ).rejects.toThrow('unreleased');
  });
  it('retains the first-successor supporting CLI gate despite inherited history', async () => {
    const map = scope.parseComponentMap(
      JSON.stringify({
        components: { 'fixture-new': { ...definitions['fixture-new'], requiresCliSha: sha(2) } },
      })
    );
    const base = [published('fixture-old', 9)];
    const plan = (releases: NonNullable<CutMetadata['releases']>) =>
      cut.planReleaseCuts({
        metadata: {
          schema: 1,
          repository: 'pj-tmt/tmt',
          cut: sha(4),
          draftVisibility: 'trusted',
          releases,
        },
        map,
        git: historyGit(releases),
        date: '2026-10-08',
      });
    expect((await plan(base)).components[0].reason).toContain('published supporting CLI');
    expect(
      (
        await plan([
          ...base,
          { tag_name: 'v5.0.0-alpha.80', target_commitish: sha(1), draft: false },
        ])
      ).components[0].reason
    ).toContain('predates registration');
    expect(
      (
        await plan([
          ...base,
          { tag_name: 'v5.0.0-alpha.81', target_commitish: sha(3), draft: false },
        ])
      ).components[0].status
    ).toBe('proposed');
    expect((await plan([...base, published('fixture-new', 10, 2)])).components[0].status).toBe(
      'proposed'
    );
  });
});

describe('predecessor upgrade archives', () => {
  it('uses published predecessor history only when no own lower published release exists', () => {
    const releases = [
      published('fixture-old', 9),
      draft('fixture-old', 11),
      draft('fixture-new', 10),
    ];
    const select = (values = releases, product = 'fixture-new') =>
      upgrade.selectPrevious({
        releases: values,
        product,
        candidateTag: `tmt-${product}-v0.1.0-alpha.10`,
        map: mapOf(),
      })?.tag_name ?? null;
    expect(select()).toBe('tmt-fixture-old-v0.1.0-alpha.9');
    expect(select([draft('fixture-old', 9)])).toBeNull();
    expect(select([published('fixture-old', 10), published('fixture-old', 11)])).toBeNull();
    expect(select([...releases, published('fixture-new', 8)])).toBe(
      'tmt-fixture-new-v0.1.0-alpha.8'
    );
    expect(select([published('fixture-old', 9), published('fixture-new', 10)])).toBe(
      'tmt-fixture-old-v0.1.0-alpha.9'
    );
    expect(select(releases, 'fixture-next')).toBe('tmt-fixture-old-v0.1.0-alpha.9');
  });
  it('stages the old archive with its own identity and verifies retained digests before the existing verifier call', () => {
    const target = 'aarch64-apple-darwin';
    const directory = path.join(root, 'staged');
    let id = 1;
    const content = new Map<number, string>();
    const release = (product: string, tag: string, draft = false): DraftRelease => ({
      id: id++,
      created_at: '2026-10-08T00:00:00Z',
      tag_name: tag,
      target_commitish: sha(3),
      draft,
      assets: [`tmt-${product}-${target}.tar.gz`, 'dist-manifest.json'].map((name) => {
        const assetId = id++;
        const bytes = `${tag}:${name}`;
        content.set(assetId, bytes);
        return {
          id: assetId,
          name,
          digest: `sha256:${createHash('sha256').update(bytes).digest('hex')}`,
        };
      }),
    });
    const releases = [
      release('fixture-old', 'tmt-fixture-old-v0.1.0-alpha.9'),
      release('fixture-new', 'tmt-fixture-new-v0.1.0-alpha.10', true),
      release('cli', 'v5.0.0-alpha.81'),
    ];
    const plan = upgrade.fetchUpgrade({
      releases,
      product: 'fixture-new',
      tag: 'tmt-fixture-new-v0.1.0-alpha.10',
      directory,
      map: mapOf(),
      download: (asset, file) => writeFileSync(file, content.get(asset.id)!),
    });
    expect(plan.previous).toBe('tmt-fixture-old-v0.1.0-alpha.9');
    expect(Object.keys(plan.files).sort()).toEqual([
      `${target}/candidate/dist-manifest.json`,
      `${target}/candidate/tmt-fixture-new-${target}.tar.gz`,
      `${target}/driver/dist-manifest.json`,
      `${target}/driver/tmt-cli-${target}.tar.gz`,
      `${target}/previous/dist-manifest.json`,
      `${target}/previous/tmt-fixture-old-${target}.tar.gz`,
    ]);
    const calls: { script: string; args: string[] }[] = [];
    expect(
      upgrade.proveStaged({
        directory,
        product: 'fixture-new',
        tag: plan.tag,
        target,
        run: (script, args) => calls.push({ script, args }),
      })
    ).toEqual({ previous: plan.previous });
    expect(calls).toHaveLength(1);
    expect(calls[0].script).toBe('verify-native-extension-upgrade.mjs');
    expect(calls[0].args[calls[0].args.indexOf('--previous-archive') + 1]).toBe(
      path.join(directory, target, 'previous', `tmt-fixture-old-${target}.tar.gz`)
    );
    // This injected call proves staging/arguments, not cross-product runtime verifier acceptance.
    writeFileSync(
      path.join(directory, target, 'previous', `tmt-fixture-old-${target}.tar.gz`),
      'changed'
    );
    expect(() =>
      upgrade.proveStaged({
        directory,
        product: 'fixture-new',
        tag: plan.tag,
        target,
        run: () => {
          throw new Error('must not execute');
        },
      })
    ).toThrow('recorded digest');
  });
});
