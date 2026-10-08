import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import type { ComponentMap } from '../../scripts/ci-scope.mjs';
import type { CutMetadata } from '../../scripts/release-cut.mjs';
import type { DraftRelease } from '../../scripts/release-draft-assets.mjs';
import { writeExecutable } from '../support/executable-fixture.mjs';

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
  root = realpathSync(mkdtempSync(path.join(tmpdir(), 'release-predecessor-')));
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
const mapOf = (components: Record<string, object> = definitions) =>
  scope.parseComponentMap(JSON.stringify({ components }));
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
  it('preserves immutable activation fields without planning a retired historical product', async () => {
    const text = JSON.stringify({
      components: {
        'fixture-old': { owns: ['fixture/old'], package: 'tmt-fixture-old', release: true },
      },
    });
    expect(() => scope.parseComponentMap(text)).toThrow('release: false');
    // This literal immutable-source fixture represents the product before retirement.
    const historical = scope.parseComponentMap(text, { historical: true });
    expect(historical.components[0].release).toBe(true);
    const calls: string[][] = [];
    const result = await cut.planReleaseCuts({
      metadata: {
        schema: 1,
        repository: 'pj-tmt/tmt',
        cut: sha(4),
        draftVisibility: 'trusted',
        releases: [],
      },
      map: historical,
      date: '2026-10-08',
      git: (args) => {
        calls.push(args);
        if (args[0] === 'merge-base') return '';
        throw new Error('retired product must not read allocation or notes');
      },
    });
    expect(result.components).toEqual([]);
    expect(calls).toEqual([['merge-base', '--is-ancestor', sha(4), 'refs/remotes/origin/main']]);
  });

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
  it('stages both CLI drivers and the old archive with their own identities before the verifier call', () => {
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
      release('cli', 'v5.0.0-alpha.80'),
    ];
    const map = mapOf({
      ...definitions,
      'fixture-new': { ...definitions['fixture-new'], requiresCliSha: sha(3) },
    });
    const plan = upgrade.fetchUpgrade({
      releases,
      product: 'fixture-new',
      tag: 'tmt-fixture-new-v0.1.0-alpha.10',
      directory,
      map,
      observeCli: (release) =>
        release.tag_name === 'v5.0.0-alpha.81'
          ? { sha: sha(4), status: 'ahead' }
          : { sha: sha(1), status: 'behind' },
      download: (asset, file) => writeFileSync(file, content.get(asset.id)!),
    });
    expect(plan.previous).toBe('tmt-fixture-old-v0.1.0-alpha.9');
    expect(Object.keys(plan.files).sort()).toEqual(
      [
        `${target}/candidate/dist-manifest.json`,
        `${target}/candidate/tmt-fixture-new-${target}.tar.gz`,
        `${target}/driver/dist-manifest.json`,
        `${target}/driver/tmt-cli-${target}.tar.gz`,
        `${target}/previous/dist-manifest.json`,
        `${target}/previous/tmt-fixture-old-${target}.tar.gz`,
        `${target}/previous-driver/dist-manifest.json`,
        `${target}/previous-driver/tmt-cli-${target}.tar.gz`,
      ].sort()
    );
    const calls: { script: string; args: string[] }[] = [];
    expect(
      upgrade.proveStaged({
        directory,
        product: 'fixture-new',
        tag: plan.tag,
        target,
        map,
        run: (script, args) => calls.push({ script, args }),
      })
    ).toEqual({ previous: plan.previous });
    expect(calls).toHaveLength(1);
    expect(calls[0].script).toBe('verify-native-extension-upgrade.mjs');
    expect(calls[0].args[calls[0].args.indexOf('--previous-archive') + 1]).toBe(
      path.join(directory, target, 'previous', `tmt-fixture-old-${target}.tar.gz`)
    );
    expect(calls[0].args.slice(-6)).toEqual([
      '--previous-product',
      'fixture-old',
      '--previous-driver-archive',
      path.join(directory, target, 'previous-driver', `tmt-cli-${target}.tar.gz`),
      '--previous-driver-manifest',
      path.join(directory, target, 'previous-driver', 'dist-manifest.json'),
    ]);
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
        map,
        run: () => {
          throw new Error('must not execute');
        },
      })
    ).toThrow('recorded digest');
  });
});

describe('two published CLI drivers for a predecessor', () => {
  const targets = [
    'aarch64-apple-darwin',
    'aarch64-unknown-linux-musl',
    'x86_64-apple-darwin',
    'x86_64-unknown-linux-musl',
  ];
  const candidateTag = 'tmt-fixture-new-v0.1.0-alpha.10';
  const registration = sha(3);
  const crossDefinitions = () => ({
    ...definitions,
    'fixture-new': { ...definitions['fixture-new'], requiresCliSha: registration },
  });
  type Observation = { sha: string; status: 'ahead' | 'behind' | 'identical' | 'diverged' };
  const observations: Record<string, Observation> = {
    'v5.0.0-alpha.84': { sha: sha(8), status: 'ahead' },
    'v5.0.0-alpha.83': { sha: registration, status: 'identical' },
    'v5.0.0-alpha.82': { sha: sha(6), status: 'diverged' },
    'v5.0.0-alpha.81': { sha: sha(2), status: 'behind' },
    'v5.0.0-alpha.80': { sha: sha(1), status: 'behind' },
  };
  function fixture() {
    const directory = mkdtempSync(path.join(root, 'two-drivers-'));
    const contents = new Map<number, string>();
    let id = 1;
    const release = (product: string, tag: string, draft = false): DraftRelease => ({
      id: id++,
      tag_name: tag,
      draft,
      // Deliberately wrong for ancestry: only a resolved published tag is evidence.
      target_commitish: sha(99),
      created_at: '2026-10-08T00:00:00Z',
      assets: [...targets.map((t) => `tmt-${product}-${t}.tar.gz`), 'dist-manifest.json'].map(
        (name) => {
          const assetId = id++;
          const bytes = `${tag}:${name}`;
          contents.set(assetId, bytes);
          return {
            id: assetId,
            name,
            digest: `sha256:${createHash('sha256').update(bytes).digest('hex')}`,
          };
        }
      ),
    });
    const releases = [
      release('fixture-old', 'tmt-fixture-old-v0.1.0-alpha.9'),
      release('fixture-new', candidateTag, true),
      release('cli', 'v5.0.0-alpha.90', true),
      ...Object.keys(observations).map((tag) => release('cli', tag)),
    ];
    const observed: string[] = [];
    const downloads: string[] = [];
    const observeCli = (r: DraftRelease, at: string): Observation => {
      expect(at).toBe(registration);
      observed.push(r.tag_name);
      return observations[r.tag_name];
    };
    const download = (asset: NonNullable<DraftRelease['assets']>[number], file: string) => {
      downloads.push(file);
      writeFileSync(file, contents.get(asset.id)!);
    };
    const input = {
      releases,
      directory,
      product: 'fixture-new',
      tag: candidateTag,
      map: mapOf(crossDefinitions()),
      observeCli,
      download,
    };
    return { input, contents, observed, downloads, release };
  }
  it('chooses the newest strict ancestor, skips drafts/divergence, and captures all four target pairs', () => {
    const f = fixture();
    const plan = upgrade.fetchUpgrade(f.input);
    expect(plan).toMatchObject({
      previous: 'tmt-fixture-old-v0.1.0-alpha.9',
      driver: 'v5.0.0-alpha.84',
      previousDriver: 'v5.0.0-alpha.81',
      driverSha: sha(8),
      previousDriverSha: sha(2),
      requiresCliSha: registration,
    });
    expect(f.observed).toEqual([
      'v5.0.0-alpha.84',
      'v5.0.0-alpha.83',
      'v5.0.0-alpha.82',
      'v5.0.0-alpha.81',
    ]);
    expect(Object.keys(plan.files)).toHaveLength(32);
    for (const target of targets) {
      const calls: { script: string; args: string[] }[] = [];
      upgrade.proveStaged({
        ...f.input,
        target,
        run: (script, args) => calls.push({ script, args }),
      });
      expect(calls).toEqual([
        {
          script: 'verify-native-extension-upgrade.mjs',
          args: [
            '--archive',
            path.join(f.input.directory, target, 'candidate', `tmt-fixture-new-${target}.tar.gz`),
            '--manifest',
            path.join(f.input.directory, target, 'candidate', 'dist-manifest.json'),
            '--previous-archive',
            path.join(f.input.directory, target, 'previous', `tmt-fixture-old-${target}.tar.gz`),
            '--previous-manifest',
            path.join(f.input.directory, target, 'previous', 'dist-manifest.json'),
            '--target',
            target,
            '--product',
            'fixture-new',
            '--driver-archive',
            path.join(f.input.directory, target, 'driver', `tmt-cli-${target}.tar.gz`),
            '--driver-manifest',
            path.join(f.input.directory, target, 'driver', 'dist-manifest.json'),
            '--previous-product',
            'fixture-old',
            '--previous-driver-archive',
            path.join(f.input.directory, target, 'previous-driver', `tmt-cli-${target}.tar.gz`),
            '--previous-driver-manifest',
            path.join(f.input.directory, target, 'previous-driver', 'dist-manifest.json'),
          ],
        },
      ]);
    }
  });
  it.each([
    ['missing registration', 'exact requiresCliSha'],
    ['missing observation', 'published CLI tag ancestry'],
    ['unknown status', 'Unknown CLI ancestry'],
    ['unknown SHA', 'Unknown CLI ancestry'],
    ['array SHA', 'Unknown CLI ancestry'],
    ['latest behind', 'does not contain registration'],
    ['latest diverged', 'does not contain registration'],
    ['no ancestor', 'strict pre-registration ancestor'],
    ['REST error', 'permission unavailable'],
    ['no published CLI', 'needs a published CLI release'],
    ['unreviewed predecessor chain', 'immediate predecessor'],
  ])('refuses %s before any download', (kind, message) => {
    const f = fixture();
    const input = { ...f.input };
    if (kind === 'missing registration') input.map = mapOf();
    if (kind === 'missing observation') delete (input as Partial<typeof input>).observeCli;
    if (kind === 'unknown status')
      input.observeCli = () => ({ sha: sha(8), status: 'unknown' }) as unknown as Observation;
    if (kind === 'unknown SHA') input.observeCli = () => ({ sha: 'not-a-commit', status: 'ahead' });
    if (kind === 'array SHA')
      input.observeCli = () => ({ sha: [sha(8)], status: 'ahead' }) as unknown as Observation;
    if (kind === 'latest behind') input.observeCli = () => ({ sha: sha(2), status: 'behind' });
    if (kind === 'latest diverged') input.observeCli = () => ({ sha: sha(6), status: 'diverged' });
    if (kind === 'no ancestor') input.observeCli = () => ({ sha: sha(8), status: 'ahead' });
    if (kind === 'REST error')
      input.observeCli = () => {
        throw new Error('permission unavailable');
      };
    if (kind === 'no published CLI')
      input.releases = input.releases.filter((release) => !release.tag_name.startsWith('v'));
    if (kind === 'unreviewed predecessor chain') {
      input.product = 'fixture-next';
      input.tag = 'tmt-fixture-next-v0.1.0-alpha.10';
      input.releases = [
        ...input.releases,
        {
          ...draft('fixture-next', 10),
          assets: input.releases[1].assets!.map((asset) => ({
            ...asset,
            name: asset.name.replace('tmt-fixture-new-', 'tmt-fixture-next-'),
          })),
        },
      ];
    }
    expect(() => upgrade.fetchUpgrade(input)).toThrow(message);
    expect(f.downloads).toEqual([]);
  });
  it('accepts the registration commit itself as the new driver, never as the old driver', () => {
    const f = fixture();
    f.input.observeCli = (release) =>
      release.tag_name === 'v5.0.0-alpha.84'
        ? { sha: registration, status: 'identical' }
        : observations[release.tag_name];
    expect(upgrade.fetchUpgrade(f.input)).toMatchObject({
      driverSha: registration,
      previousDriver: 'v5.0.0-alpha.81',
      previousDriverSha: sha(2),
    });
  });
  it('own published history uses the unchanged one-driver path without ancestry reads', () => {
    const f = fixture();
    f.input.releases.push(f.release('fixture-new', 'tmt-fixture-new-v0.1.0-alpha.8'));
    f.input.observeCli = () => {
      throw new Error('same-product must not read ancestry');
    };
    const plan = upgrade.fetchUpgrade(f.input);
    expect(plan.previous).toBe('tmt-fixture-new-v0.1.0-alpha.8');
    expect(Object.keys(plan).sort()).toEqual(
      ['product', 'tag', 'previous', 'floor', 'driver', 'files'].sort()
    );
    expect(Object.keys(plan.files)).toHaveLength(24);
    const calls: string[][] = [];
    upgrade.proveStaged({ ...f.input, target: targets[0], run: (_, args) => calls.push(args) });
    expect(calls).toHaveLength(1);
    expect(calls[0]).not.toContain('--previous-product');
    expect(calls[0]).not.toContain('--previous-driver-archive');
    expect(calls[0]).toHaveLength(16);
    plan.driverSha = sha(8);
    writeFileSync(path.join(f.input.directory, 'plan.json'), JSON.stringify(plan));
    expect(() =>
      upgrade.proveStaged({
        ...f.input,
        target: targets[0],
        run: () => {
          throw new Error('must not run');
        },
      })
    ).toThrow('Same-product plan must not carry');
  });
  it('no published predecessor retains the original no-op and makes no ancestry or download calls', () => {
    const f = fixture();
    f.input.releases = f.input.releases.filter(
      (release) => !release.tag_name.startsWith('tmt-fixture-old-')
    );
    f.input.observeCli = () => {
      throw new Error('no previous must not read ancestry');
    };
    const plan = upgrade.fetchUpgrade(f.input);
    expect(plan).toEqual({
      product: 'fixture-new',
      tag: candidateTag,
      previous: null,
      floor: null,
      driver: null,
      files: {},
    });
    expect(f.downloads).toEqual([]);
    expect(
      upgrade.proveStaged({
        ...f.input,
        target: targets[0],
        run: () => {
          throw new Error('must not run');
        },
      })
    ).toEqual({ previous: null });
  });
  it.each(['driver', 'previous-driver'] as const)(
    'refuses either tampered %s before execution',
    (kind) => {
      const f = fixture();
      upgrade.fetchUpgrade(f.input);
      writeFileSync(
        path.join(f.input.directory, targets[0], kind, `tmt-cli-${targets[0]}.tar.gz`),
        'tampered'
      );
      const executed: string[] = [];
      expect(() =>
        upgrade.proveStaged({
          ...f.input,
          target: targets[0],
          run: (script) => executed.push(script),
        })
      ).toThrow('recorded digest');
      expect(executed).toEqual([]);
    }
  );
  it.each(['previousDriver', 'previousDriverSha', 'driverSha', 'requiresCliSha'] as const)(
    'refuses missing %s provenance',
    (field) => {
      const f = fixture();
      const plan = upgrade.fetchUpgrade(f.input);
      delete plan[field];
      writeFileSync(path.join(f.input.directory, 'plan.json'), JSON.stringify(plan));
      expect(() =>
        upgrade.proveStaged({
          ...f.input,
          target: targets[0],
          run: () => {
            throw new Error('must not run');
          },
        })
      ).toThrow('registration provenance');
    }
  );
  it.each(['requiresCliSha', 'previousDriverSha', 'previousDriver'] as const)(
    'refuses inconsistent %s provenance',
    (field) => {
      const f = fixture();
      const plan = upgrade.fetchUpgrade(f.input);
      plan[field] =
        field === 'requiresCliSha'
          ? sha(4)
          : field === 'previousDriverSha'
            ? plan.driverSha
            : plan.driver!;
      writeFileSync(path.join(f.input.directory, 'plan.json'), JSON.stringify(plan));
      expect(() =>
        upgrade.proveStaged({
          ...f.input,
          target: targets[0],
          run: () => {
            throw new Error('must not run');
          },
        })
      ).toThrow('registration provenance');
    }
  );
  it.each(['driverSha', 'previousDriverSha', 'requiresCliSha'] as const)(
    'refuses non-string %s provenance even when it stringifies to a SHA',
    (field) => {
      const f = fixture();
      const plan = upgrade.fetchUpgrade(f.input);
      writeFileSync(
        path.join(f.input.directory, 'plan.json'),
        JSON.stringify({ ...plan, [field]: [plan[field]] })
      );
      expect(() =>
        upgrade.proveStaged({
          ...f.input,
          target: targets[0],
          run: () => {
            throw new Error('must not run');
          },
        })
      ).toThrow('registration provenance');
    }
  );
  it.each(['target', 'digest'] as const)(
    'refuses missing old driver %s without fallback',
    (kind) => {
      const f = fixture();
      f.input.releases = f.input.releases.map((release) =>
        release.tag_name !== 'v5.0.0-alpha.81'
          ? release
          : {
              ...release,
              assets:
                kind === 'target'
                  ? release.assets!.filter((asset) => !asset.name.includes(targets[0]))
                  : release.assets!.map((asset) =>
                      asset.name.includes(targets[0]) ? { id: asset.id, name: asset.name } : asset
                    ),
            }
      );
      expect(() => upgrade.fetchUpgrade(f.input)).toThrow(
        kind === 'target' ? 'has no tmt-cli-' : 'no usable digest'
      );
      expect(f.observed).not.toContain('v5.0.0-alpha.80');
    }
  );
  it('requires a recorded previous-driver digest even when its bytes are still present', () => {
    const f = fixture();
    const plan = upgrade.fetchUpgrade(f.input);
    delete plan.files[`${targets[0]}/previous-driver/dist-manifest.json`];
    writeFileSync(path.join(f.input.directory, 'plan.json'), JSON.stringify(plan));
    expect(() =>
      upgrade.proveStaged({
        ...f.input,
        target: targets[0],
        run: () => {
          throw new Error('must not run');
        },
      })
    ).toThrow('no recorded digest');
  });
  it('binds the actual fetch CLI caller to REST tag resolution and registration comparison', () => {
    const f = fixture();
    const bin = path.join(f.input.directory, 'bin');
    mkdirSync(bin);
    const registry = path.join(root, '.github/components.json');
    const original = readFileSync(registry);
    const log = path.join(f.input.directory, 'rest.jsonl');
    const responses: Record<string, unknown> = {
      'repos/fixture/repository/releases': [f.input.releases],
    };
    for (const [tag, observation] of Object.entries(observations)) {
      responses[`repos/fixture/repository/commits/${tag}`] = {
        sha: observation.sha,
        files: [{ patch: 'unused commit patch' }],
      };
      responses[`repos/fixture/repository/compare/${registration}...${observation.sha}`] = {
        status: observation.status,
        base_commit: { sha: registration },
        merge_base_commit: {
          sha: observation.status === 'behind' ? observation.sha : registration,
        },
        files: [{ patch: 'unused comparison patch' }],
      };
    }
    const fixtureFile = path.join(f.input.directory, 'responses.json');
    writeFileSync(
      fixtureFile,
      JSON.stringify({ responses, assets: Object.fromEntries(f.contents), log })
    );
    writeExecutable(
      path.join(bin, 'gh'),
      `#!${process.execPath}
const fs = require('node:fs');
const fixture = JSON.parse(fs.readFileSync(process.env.TMT_ANCESTRY_FIXTURE, 'utf8'));
const args = process.argv.slice(2); const endpoint = args.find(a => a.startsWith('repos/'));
fs.appendFileSync(fixture.log, JSON.stringify(args) + '\\n');
if (endpoint?.includes('/releases/assets/')) {
  const value = fixture.assets[endpoint.split('/').at(-1)];
  if (value === undefined) process.exit(91); process.stdout.write(value);
} else {
  if (!Object.hasOwn(fixture.responses, endpoint)) process.exit(92);
  const response = fixture.responses[endpoint];
  const filter = args[args.indexOf('--jq') + 1];
  if (endpoint?.includes('/commits/')) {
    if (!args.includes('--jq') || filter !== '{sha: .sha}') process.exit(93);
    process.stdout.write(JSON.stringify({sha: response.sha}));
  } else if (endpoint?.includes('/compare/')) {
    if (!args.includes('--jq') || filter !== '{status: .status, base_commit: {sha: .base_commit.sha}, merge_base_commit: {sha: .merge_base_commit.sha}}') process.exit(94);
    process.stdout.write(JSON.stringify({status: response.status, base_commit: {sha: response.base_commit.sha}, merge_base_commit: {sha: response.merge_base_commit.sha}}));
  } else process.stdout.write(JSON.stringify(response));
}
`
    );
    try {
      writeFileSync(registry, JSON.stringify({ components: crossDefinitions() }));
      const result = spawnSync(
        process.execPath,
        [
          path.join(root, 'typescript/scripts/release-upgrade.mjs'),
          'fetch',
          '--product',
          'fixture-new',
          '--tag',
          candidateTag,
          '--directory',
          path.join(f.input.directory, 'caller-staged'),
        ],
        {
          env: {
            ...process.env,
            PATH: bin,
            GITHUB_REPOSITORY: 'fixture/repository',
            TMT_ANCESTRY_FIXTURE: fixtureFile,
          },
          encoding: 'utf8',
          timeout: 10_000,
        }
      );
      expect(result.error).toBeUndefined();
      expect(result.status, result.stderr).toBe(0);
      const plan = JSON.parse(
        readFileSync(path.join(f.input.directory, 'caller-staged/plan.json'), 'utf8')
      );
      expect(plan).toMatchObject({
        previousDriver: 'v5.0.0-alpha.81',
        previousDriverSha: sha(2),
        driverSha: sha(8),
        requiresCliSha: registration,
      });
      const args = readFileSync(log, 'utf8')
        .trim()
        .split('\n')
        .map((line) => JSON.parse(line) as string[]);
      expect(args.filter((a) => a[1].includes('/commits/')).map((a) => a[1])).toEqual([
        'repos/fixture/repository/commits/v5.0.0-alpha.84',
        'repos/fixture/repository/commits/v5.0.0-alpha.83',
        'repos/fixture/repository/commits/v5.0.0-alpha.82',
        'repos/fixture/repository/commits/v5.0.0-alpha.81',
      ]);
      expect(args.filter((a) => a[1].includes('/compare/')).map((a) => a[1])).toEqual([
        `repos/fixture/repository/compare/${registration}...${sha(8)}`,
        `repos/fixture/repository/compare/${registration}...${registration}`,
        `repos/fixture/repository/compare/${registration}...${sha(6)}`,
        `repos/fixture/repository/compare/${registration}...${sha(2)}`,
      ]);
      expect(Object.keys(plan.files)).toHaveLength(32);
    } finally {
      writeFileSync(registry, original);
    }
  });
});
