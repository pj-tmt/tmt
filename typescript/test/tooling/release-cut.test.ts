import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vite-plus/test';
import history from '../fixtures/release-cut-history.json' with { type: 'json' };
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import { versionOfTag } from '../../scripts/release-versions.mjs';
import { readCutMetadata } from '../../scripts/release-cut-read.mjs';
import {
  attributeCutCommits,
  nextAlphaVersion,
  parseReleaseCommits,
  planReleaseCuts,
  readCutRange,
  releaseCutHistory,
  renderCutNotes,
  renderCutSummary,
  type CutCommit,
  type CutMetadata,
} from '../../scripts/release-cut.mjs';

const sha = (n: number) => n.toString(16).padStart(40, 'a');
const definitions = {
  cli: { owns: ['.'], excludes: ['extensions', 'shared'], package: 'tmt-cli' },
  ops: {
    owns: ['extensions/ops'],
    package: 'tmt-ops',
    requiresCliSha: sha(0),
    selectedBy: ['tests/ops.*'],
  },
  private: { owns: ['shared'], release: false, releaseConsumers: ['ops'] },
  never: {
    owns: ['extensions/never'],
    release: false,
    releaseStatus: 'never',
    package: 'tmt-never',
  },
  parked: {
    owns: ['extensions/parked'],
    release: false,
    releaseStatus: 'parked',
    package: 'tmt-parked',
  },
};
const map = parseComponentMap(JSON.stringify({ components: definitions }));
const commit = (message = 'feat: add feature', files = ['rust/file.rs'], n = 1): CutCommit => ({
  sha: sha(n),
  message,
  files,
});
const render = (commits: CutCommit[]) =>
  renderCutNotes({
    commits,
    repository: 'pj-tmt/tmt',
    version: '5.0.0-alpha.47',
    previousTag: 'v5.0.0-alpha.46',
    tag: 'v5.0.0-alpha.47',
    date: '2026-10-03',
  });

describe('direct component-map cut attribution', () => {
  it('uses release roots and declared consumers without turning CI selection into release attribution', () => {
    const commits = [
      commit('feat: CLI', ['rust/x', 'extensions/parked/x']),
      commit('feat: private leaf', ['shared/a', 'shared/b'], 2),
      commit('fix: selected test', ['tests/ops.test'], 3),
      commit('feat: parked only', ['extensions/parked/a'], 4),
    ];
    // CI selects Ops for the test; release membership still follows the CLI root.
    expect(attributeCutCommits(commits, map, 'cli').map((c) => c.sha)).toEqual([sha(1), sha(3)]);
    expect(attributeCutCommits(commits, map, 'ops').map((c) => c.sha)).toEqual([sha(2)]);
  });
  it('rejects unknown, never-shipped or non-private consumption instead of a second attribution list', () => {
    for (const consumers of [['missing'], ['never']]) {
      expect(() =>
        parseComponentMap(
          JSON.stringify({
            components: {
              ...definitions,
              never: {
                owns: ['tooling'],
                package: 'private-tool',
                release: false,
                releaseStatus: 'never',
              },
              private: { ...definitions.private, releaseConsumers: consumers },
            },
          })
        )
      ).toThrow('Invalid release consumer');
    }
    expect(() =>
      parseComponentMap(
        JSON.stringify({
          components: { ...definitions, private: { ...definitions.private, release: true } },
        })
      )
    ).toThrow('Invalid release consumer');
  });
  it('preserves CLI plus Ops for style/invoke, and Ops only for CLI-excluded TUI', async () => {
    const baseline = {
      cli: {
        owns: ['.'],
        excludes: ['extensions/ops', 'rust/crates/tmt-tui'],
        package: 'tmt-cli',
      },
      ops: { owns: ['extensions/ops'], package: 'tmt-ops' },
      tui: { owns: ['rust/crates/tmt-tui'], release: false, releaseConsumers: ['ops'] },
    };
    const extended = parseComponentMap(
      JSON.stringify({
        components: {
          ...baseline,
          style: {
            owns: ['rust/crates/tmt-cli-style'],
            release: false,
            releaseConsumers: ['ops'],
          },
          invoke: { owns: ['rust/crates/tmt-invoke'], release: false, releaseConsumers: ['ops'] },
        },
      })
    );
    const commits = [
      commit('feat: style change', ['rust/crates/tmt-cli-style/src/lib.rs']),
      commit('fix: invocation change', ['rust/crates/tmt-invoke/src/lib.rs'], 2),
      commit('feat: TUI change', ['rust/crates/tmt-tui/src/lib.rs'], 3),
    ];
    const cli = await render(attributeCutCommits(commits, extended, 'cli'));
    expect(cli.commits).toEqual([sha(1), sha(2)]);
    expect(cli).toEqual(
      await render(
        attributeCutCommits(
          commits,
          parseComponentMap(JSON.stringify({ components: baseline })),
          'cli'
        )
      )
    );
    const ops = await render(attributeCutCommits(commits, extended, 'ops'));
    expect(ops.commits).toEqual([sha(1), sha(2), sha(3)]);
    expect(ops.notes).toContain('style change');
    expect(ops.notes).toContain('invocation change');
    expect(ops.notes).toContain('TUI change');
  });
});

describe('conventional cut notes', () => {
  it('lists features, fixes, performance and all breaking types, with references and escaped subjects', async () => {
    const result = await render([
      commit('feat(cli): add <thing> (#42)'),
      commit('fix: repair', ['rust/x'], 2),
      commit('perf: speed up', ['rust/x'], 3),
      commit('docs: explain', ['rust/x'], 4),
      commit('chore!: remove old setting\n\nBREAKING CHANGE: old setting removed', ['rust/x'], 5),
    ]);
    expect(result.commits).toEqual([sha(1), sha(2), sha(3), sha(5)].sort());
    expect(result.breaking).toBe(true);
    expect(result.notes).toContain('&lt;thing&gt;');
    expect(result.notes).toContain('https://github.com/pj-tmt/tmt/issues/42');
    expect(result.notes).toContain('old setting removed');
    expect(result.notes).not.toContain('explain');
  });
  it('expands nested messages once per source SHA, including bang and footer breaking notes', async () => {
    const result = await render([
      commit(
        'chore: aggregate\n\nfix(cli): first fix\n\nBEGIN_NESTED_COMMIT\nfeat(ops)!: nested feature\n\nBREAKING CHANGE: changed shape\nEND_NESTED_COMMIT'
      ),
    ]);
    expect(result.commits).toEqual([sha(1)]);
    expect(result.notes).toContain('first fix');
    expect(result.notes).toContain('nested feature');
    expect(result.notes).toContain('changed shape');
    expect(() => parseReleaseCommits([commit('feat: x\n\nBEGIN_NESTED_COMMIT\nfix: y')])).toThrow(
      'Unclosed nested'
    );
  });
  it('keeps standalone conventional reverts visible and never treats Release-As as version authorization', async () => {
    const result = await render([
      commit('revert: revert previous feature\n\nThis reverts commit ' + sha(9) + '.'),
      commit('chore: change version\n\nRelease-As: 9.0.0', ['rust/x'], 2),
    ]);
    expect(result.commits).toEqual([sha(1)]);
    expect(result.notes).toContain('revert previous feature');
    expect(result.notes).not.toContain('9.0.0');
    expect(nextAlphaVersion('5.0.0-alpha.46')).toBe('5.0.0-alpha.47');
  });
  it('lists a non-conventional subject under Other changes instead of dropping it', async () => {
    const result = await render([
      commit('Keep Remote door origins stable and add status (#1605)'),
      commit('Ops: capitalized type (#7)', ['rust/x'], 2),
      commit('docs: explain', ['rust/x'], 3),
      commit('Subject only\n\nfeat: body text is not a subject', ['rust/x'], 4),
    ]);
    expect(result.commits).toEqual([sha(1), sha(2), sha(4)].sort());
    expect(result.breaking).toBe(false);
    expect(result.notes).toContain('### Other changes');
    expect(result.notes).toContain('Keep Remote door origins stable and add status');
    expect(result.notes).toContain('/commit/' + sha(2));
    expect(result.notes).not.toContain('explain');
    expect(result.notes).not.toContain('body text');
  });
  it('keeps Other changes after the conventional groups and conventional types unchanged', async () => {
    const result = await render([
      commit('Plain title'),
      commit('feat: real feature', ['rust/x'], 2),
    ]);
    expect(result.notes.indexOf('### Features')).toBeGreaterThanOrEqual(0);
    expect(result.notes.indexOf('### Features')).toBeLessThan(
      result.notes.indexOf('### Other changes')
    );
  });
  it('rejects notes that introduce an out-of-range commit link through a subject', async () => {
    await expect(
      render([commit(`feat: see [other](https://github.com/pj-tmt/tmt/commit/${sha(9)})`)])
    ).rejects.toThrow('exactly the releasable');
  });
  it.each([
    '5.0.0',
    '5.1.0-beta.1',
    '5.0.0-alpha.01',
    '05.0.0-alpha.1',
    '5.0.0-alpha.9007199254740991',
  ])('requires an owner for %s', (version) => {
    expect(() => nextAlphaVersion(version)).toThrow('owner-dispatched');
  });
});

describe('historical shadow comparison (fixture-only release PR parents)', () => {
  it.each(history.cases)('reproduces $tag at the immutable historical cut', async (fixture) => {
    const componentMap = parseComponentMap(JSON.stringify(fixture.map), { historical: true });
    expect(nextAlphaVersion(versionOfTag(fixture.previousTag, fixture.product))).toBe(
      versionOfTag(fixture.tag, fixture.product)
    );
    const result = await renderCutNotes({
      commits: attributeCutCommits(fixture.commits, componentMap, fixture.product),
      repository: history.provenance.repository,
      version: versionOfTag(fixture.tag, fixture.product),
      previousTag: fixture.previousTag,
      tag: fixture.tag,
      date: fixture.date,
    });
    expect(result.notes).toBe(fixture.publishedNotes.trim());
    const expected = [...fixture.publishedNotes.matchAll(/\/commit\/([a-f0-9]{40})\)/g)]
      .map((m) => m[1])
      .sort();
    expect(result.commits).toEqual(expected);
    expect(fixture.commits.some((c) => c.sha === fixture.previousCut)).toBe(false);
    expect(fixture.commits.some((c) => c.sha === fixture.cut)).toBe(true);
  });
  it('records the public releases and source-map snapshots without a production ancestry adapter', () => {
    expect(history.cases.map((c) => c.releasePr)).toEqual([1302, 1343, 1270]);
    expect(history.provenance.cutDefinition).toContain('fixture only');
    const source = readFileSync(new URL('../../scripts/release-cut.mjs', import.meta.url), 'utf8');
    expect(source).not.toMatch(/from.+release-please|\^1/);
  });
});

function planningFixture() {
  const metadata: CutMetadata = {
    schema: 1,
    repository: 'pj-tmt/tmt',
    cut: sha(2),
    draftVisibility: 'trusted',
    releases: [
      { tag_name: 'v5.0.0-alpha.46', draft: false },
      { tag_name: 'tmt-ops-v0.1.0-alpha.13', draft: false },
    ],
  };
  const git = vi.fn((args: string[]) => {
    if (args[0] === 'merge-base') return '';
    if (args[0] === 'tag') return '';
    if (args[0] === 'rev-parse') return sha(0);
    if (args[0] === 'log') return `${sha(1)}\0feat: feature\n\0\n`;
    if (args[0] === 'show') return `${sha(0)} ${sha(8)}`;
    if (args[0] === 'diff') return 'rust/file.rs\0';
    throw new Error(`Unexpected git ${args}`);
  });
  return { metadata, git, map, date: '2026-10-03' };
}
describe('immutable plans and independent cuts', () => {
  it('accepts an owner-selected advancing stable/core version without authorizing publication', async () => {
    const fixture = planningFixture();
    for (const version of ['5.0.0', '5.1.0-alpha.0', '6.0.0']) {
      const result = await planReleaseCuts({ ...fixture, versions: { cli: version } });
      expect(result.components[0]).toMatchObject({
        status: 'proposed',
        version,
        tag: `v${version}`,
        authorization: 'owner-required',
      });
    }
    fixture.metadata.releases = [{ tag_name: 'v5.0.0', draft: false }];
    expect(
      (await planReleaseCuts({ ...fixture, versions: { cli: '5.1.0-alpha.0' } })).components[0]
    ).toMatchObject({ status: 'proposed', authorization: 'owner-required' });
  });
  it('refuses non-advancing, ambiguous and parked explicit versions', async () => {
    const fixture = planningFixture();
    for (const version of ['5.0.0-alpha.46', '5.0.0-alpha.1', '4.9.0']) {
      expect(
        (await planReleaseCuts({ ...fixture, versions: { cli: version } })).components[0].reason
      ).toContain('must advance');
    }
    for (const version of ['05.0.0', '5.0.0-alpha.01', '5.0.0-rc.1', '5.0.0+build', 'main'])
      await expect(planReleaseCuts({ ...fixture, versions: { cli: version } })).rejects.toThrow(
        'canonical'
      );
    await expect(planReleaseCuts({ ...fixture, versions: { parked: '1.0.0' } })).rejects.toThrow(
      'Unknown native product'
    );
  });
  it('computes a cut/version/notes with ordinary tag ancestry, without a mutation client', async () => {
    const fixture = planningFixture();
    const result = await planReleaseCuts(fixture);
    expect(result.mode).toBe('plan');
    expect(result.components.map((c) => [c.product, c.status])).toEqual([
      ['cli', 'proposed'],
      ['ops', 'no-releasable-commits'],
    ]);
    expect(result.components[0]).toMatchObject({
      cut: sha(2),
      previous: sha(0),
      tag: 'v5.0.0-alpha.47',
      commits: [sha(1)],
    });
    expect(renderCutSummary(result)).toContain('Creates no drafts, tags or dispatches');
    const diffs = fixture.git.mock.calls.filter(([args]) => args[0] === 'diff');
    expect(diffs.every(([args]) => args.at(-2) === sha(0))).toBe(true);
  });
  it('proposes a cut for a non-conventional commit that touches a released component only', async () => {
    const fixture = planningFixture();
    const original = fixture.git.getMockImplementation()!;
    fixture.git.mockImplementation((args) =>
      args[0] === 'log'
        ? `${sha(1)}\0Keep Remote door origins stable (#1605)\n\0\n`
        : args[0] === 'diff'
          ? 'extensions/ops/door.rs\0'
          : original(args)
    );
    const result = await planReleaseCuts(fixture);
    expect(result.components.map((c) => [c.product, c.status])).toEqual([
      ['cli', 'no-releasable-commits'],
      ['ops', 'proposed'],
    ]);
    expect(result.components[1]).toMatchObject({
      commits: [sha(1)],
      tag: 'tmt-ops-v0.1.0-alpha.14',
    });
    expect(result.components[1].notes).toContain('Other changes');
    fixture.git.mockImplementation((args) =>
      args[0] === 'log'
        ? `${sha(1)}\0docs: explain door\n\0\n`
        : args[0] === 'diff'
          ? 'extensions/ops/door.md\0'
          : original(args)
    );
    expect((await planReleaseCuts(fixture)).components.map((c) => c.status)).toEqual([
      'no-releasable-commits',
      'no-releasable-commits',
    ]);
  });
  it('does not refuse a new cut for an existing unpublished draft', async () => {
    const fixture = planningFixture();
    fixture.metadata.releases!.push({
      tag_name: 'v5.0.0-alpha.47',
      draft: true,
      target_commitish: sha(0),
    });
    const result = await planReleaseCuts(fixture);
    expect(result.components[0]).toMatchObject({
      status: 'proposed',
      tag: 'v5.0.0-alpha.48',
      previousTag: 'v5.0.0-alpha.46',
    });
  });
  it('counts orphan Git tags as well as drafts before allocating an alpha number', async () => {
    const fixture = planningFixture();
    const original = fixture.git.getMockImplementation()!;
    fixture.git.mockImplementation((args) =>
      args[0] === 'tag' ? 'v5.0.0-alpha.49' : original(args)
    );
    const result = await planReleaseCuts(fixture);
    expect(result.components[0]).toMatchObject({
      status: 'proposed',
      tag: 'v5.0.0-alpha.50',
      previousTag: 'v5.0.0-alpha.46',
    });
  });
  it('retains unpublished breaking changes in later notes and authorization', async () => {
    const fixture = planningFixture();
    fixture.metadata.releases!.push({
      tag_name: 'v5.0.0-alpha.47',
      draft: true,
      target_commitish: sha(1),
    });
    const original = fixture.git.getMockImplementation()!;
    fixture.git.mockImplementation((args) => {
      if (args[0] === 'merge-base' && args[2] === sha(1) && args[3] === sha(0))
        throw new Error('not ancestor', { cause: { status: 1 } });
      if (args[0] === 'log')
        return args.at(-1) === `${sha(1)}..${sha(2)}`
          ? `${sha(2)}\0fix: later component work\n\0\n`
          : `${sha(1)}\0feat!: held contract change\n\0${sha(2)}\0fix: later component work\n\0\n`;
      return original(args);
    });
    expect((await planReleaseCuts(fixture)).components[0]).toMatchObject({
      tag: 'v5.0.0-alpha.48',
      previousTag: 'v5.0.0-alpha.46',
      breaking: true,
      authorization: 'owner-required',
      commits: [sha(1), sha(2)],
    });
  });
  it('chooses a published ancestor rather than a later published descendant, without ignoring history errors', () => {
    const fixture = planningFixture();
    const releases = [
      { tag_name: 'v5.0.0-alpha.49', draft: false },
      { tag_name: 'v5.0.0-alpha.48', draft: false },
    ];
    const git = (args: string[]) => {
      if (args[0] === 'tag') return '';
      if (args[0] === 'rev-parse') return args[2].includes('alpha.49') ? sha(3) : sha(0);
      if (args[0] === 'merge-base' && args[2] === sha(3))
        throw new Error('not ancestor', { cause: { status: 1 } });
      return '';
    };
    expect(releaseCutHistory({ releases, product: 'cli', cut: fixture.metadata.cut, git })).toEqual(
      {
        highestVersion: '5.0.0-alpha.49',
        previous: { tag: 'v5.0.0-alpha.48', sha: sha(0) },
        previousAllocated: { tag: 'v5.0.0-alpha.48', sha: sha(0) },
      }
    );
    expect(() =>
      releaseCutHistory({
        releases,
        product: 'cli',
        cut: fixture.metadata.cut,
        git: (args) => {
          if (args[0] === 'merge-base')
            throw new Error('missing history', { cause: { status: 128 } });
          return git(args);
        },
      })
    ).toThrow('missing history');
  });
  it('uses the newest allocated source ancestor without advancing the published notes boundary', () => {
    const releases = [
      { tag_name: 'v5.0.0-alpha.49', draft: false },
      { tag_name: 'v5.0.0-alpha.50', draft: true, target_commitish: sha(1) },
      { tag_name: 'v5.0.0-alpha.51', draft: true, target_commitish: sha(0) },
      { tag_name: 'v5.0.0-alpha.52', draft: true, target_commitish: sha(3) },
    ];
    const git = (args: string[]) => {
      if (args[0] === 'tag') return '';
      if (args[0] === 'rev-parse') return sha(0);
      if (args[0] === 'merge-base' && args[2] > args[3])
        throw new Error('not ancestor', { cause: { status: 1 } });
      return '';
    };
    expect(releaseCutHistory({ releases, product: 'cli', cut: sha(2), git })).toEqual({
      highestVersion: '5.0.0-alpha.52',
      previous: { tag: 'v5.0.0-alpha.49', sha: sha(0) },
      previousAllocated: { tag: 'v5.0.0-alpha.50', sha: sha(1) },
    });
  });
  it('reports missing first-release seed, stable authorization and unavailable evidence explicitly', async () => {
    const fixture = planningFixture();
    fixture.metadata.releases = [{ tag_name: 'v5.0.0', draft: false }];
    const result = await planReleaseCuts(fixture);
    expect(result.components[0].reason).toContain('owner-dispatched');
    expect(result.components[1].reason).toContain('bootstrapSha');
    fixture.metadata.evidenceError = 'Incomplete draft pagination';
    const unavailable = await planReleaseCuts(fixture);
    expect(unavailable.components).toEqual([]);
    expect(unavailable.unavailable).toContain('pagination');
  });
  it('reserves failed draft numbers without moving either source boundary, including a tagged draft', () => {
    const releases = [
      { tag_name: 'v5.0.0-alpha.49', draft: false },
      { tag_name: 'v5.0.0-alpha.50', draft: true, target_commitish: sha(1) },
      {
        tag_name: 'v5.0.0-alpha.51',
        draft: true,
        target_commitish: sha(2),
        assets: [{ name: 'verification-failed.json' }],
      },
    ];
    const git = (args: string[]) => {
      if (args[0] === 'tag') return 'v5.0.0-alpha.51';
      if (args[0] === 'rev-parse') return args[2].includes('alpha.51') ? sha(2) : sha(0);
      if (args[0] === 'merge-base' && args[2] > args[3])
        throw new Error('not ancestor', { cause: { status: 1 } });
      return '';
    };
    expect(releaseCutHistory({ releases, product: 'cli', cut: sha(2), git })).toEqual({
      highestVersion: '5.0.0-alpha.51',
      previous: { tag: 'v5.0.0-alpha.49', sha: sha(0) },
      previousAllocated: { tag: 'v5.0.0-alpha.50', sha: sha(1) },
    });
    releases[2].draft = false;
    expect(releaseCutHistory({ releases, product: 'cli', cut: sha(2), git })).toMatchObject({
      previous: { tag: 'v5.0.0-alpha.51', sha: sha(2) },
      previousAllocated: { tag: 'v5.0.0-alpha.51', sha: sha(2) },
    });
  });
  it('skips a stable component with no releasable work before requiring an explicit next version', async () => {
    const fixture = planningFixture();
    fixture.metadata.releases = [{ tag_name: 'v5.0.0', draft: false }];
    const original = fixture.git.getMockImplementation()!;
    fixture.git.mockImplementation((args) => (args[0] === 'log' ? '' : original(args)));
    expect((await planReleaseCuts(fixture)).components[0]).toMatchObject({
      status: 'no-releasable-commits',
      previousTag: 'v5.0.0',
    });
  });
  it.each(['0.1.0', '0.1.0-dev', '0.1.0-alpha.01', '0.1.0-alpha.9007199254740991'])(
    'refuses invalid first-cut seed %s',
    (initialVersion) => {
      expect(() =>
        parseComponentMap(
          JSON.stringify({
            components: { cli: { ...definitions.cli, bootstrapSha: sha(0), initialVersion } },
          })
        )
      ).toThrow('initialVersion');
    }
  );
  it('requires the package and bootstrap boundary for a first-cut seed', () => {
    expect(() =>
      parseComponentMap(
        JSON.stringify({
          components: { cli: { ...definitions.cli, initialVersion: '5.0.0-alpha.1' } },
        })
      )
    ).toThrow('initialVersion');
  });
  it('uses the component map bootstrap and approved first alpha seed, never Cargo versions', async () => {
    const fixture = planningFixture();
    fixture.metadata.releases = [];
    fixture.map = parseComponentMap(
      JSON.stringify({
        components: {
          cli: { ...definitions.cli, bootstrapSha: sha(0), initialVersion: '5.0.0-alpha.1' },
        },
      })
    );
    const result = await planReleaseCuts(fixture);
    expect(result.components[0]).toMatchObject({
      previous: sha(0),
      version: '5.0.0-alpha.1',
      status: 'proposed',
    });
    expect(result.components[0].previousTag).toBeUndefined();
    expect(() =>
      parseComponentMap(
        JSON.stringify({ components: { cli: { ...definitions.cli, bootstrapSha: 'main' } } })
      )
    ).toThrow('commit SHA');
  });
  it('plans Herdr from its registered bootstrap and first alpha, retaining missing-registration refusals', async () => {
    const source = JSON.parse(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    const driver = source.components['driver-herdr'];
    expect(driver).toMatchObject({
      owns: ['drivers/herdr'],
      package: 'tmt-driver-herdr',
      release: true,
      bootstrapSha: '6ed7f3f9f0070076d4ca78e9fcc2f4de4457fbbd',
      initialVersion: '0.1.0-alpha.1',
    });
    expect(source.components['browser-addon']).toMatchObject({
      release: false,
      releaseStatus: 'parked',
      owns: ['extensions/tmt-remote/typescript/browser-addon'],
    });
    expect(source.components['browser-addon'].releaseConsumers).toBeUndefined();
    const fixture = planningFixture();
    fixture.metadata.releases = [];
    const original = fixture.git.getMockImplementation()!;
    fixture.git.mockImplementation((args) =>
      args[0] === 'diff' ? 'drivers/herdr/rust/tmt-driver-herdr/src/main.rs\0' : original(args)
    );
    const plan = (definition: typeof driver) =>
      planReleaseCuts({
        ...fixture,
        map: parseComponentMap(JSON.stringify({ components: { 'driver-herdr': definition } })),
      });
    expect((await plan(driver)).components).toMatchObject([
      {
        product: 'driver-herdr',
        previous: driver.bootstrapSha,
        version: '0.1.0-alpha.1',
        tag: 'tmt-driver-herdr-v0.1.0-alpha.1',
        status: 'proposed',
      },
    ]);
    const missingSeed = { ...driver };
    delete missingSeed.initialVersion;
    const blocked = (await plan(missingSeed)).components[0];
    expect(blocked).toMatchObject({ status: 'blocked' });
    expect(blocked.reason).toContain('First cut needs');
    expect(() => plan({ ...driver, bootstrapSha: undefined })).toThrow('initialVersion');
    expect((await plan({ ...driver, release: false })).components).toEqual([]);
  });
  it('does not fall back when a tag is absent or a range is not on main', async () => {
    const fixture = planningFixture();
    fixture.git.mockImplementation((args) => {
      if (args[0] === 'merge-base') return '';
      throw new Error('Missing product tag');
    });
    expect(
      (await planReleaseCuts(fixture)).components.every((c) => c.reason === 'Missing product tag')
    ).toBe(true);
    fixture.git.mockImplementation(() => {
      throw new Error('Not on main');
    });
    await expect(planReleaseCuts(fixture)).rejects.toThrow('Not on main');
  });
  it('rejects incomplete/oversized local history rather than dropping commits', () => {
    expect(() => readCutRange(() => 'broken', sha(0), sha(2))).toThrow('Incomplete');
    expect(() =>
      readCutRange(
        (args) => (args[0] === 'log' ? `${sha(1)}\0fix: x\0`.repeat(501) : ''),
        sha(0),
        sha(2)
      )
    ).toThrow('oversized');
    expect(() => readCutRange(() => '', 'not-sha', sha(2))).toThrow('exact SHAs');
  });
});

describe('bounded REST-only state acquisition', () => {
  const input = {
    repository: 'pj-tmt/tmt',
    cut: sha(2),
    token: 'private-fixture-token',
    draftVisibility: 'trusted',
  };
  const release = (id: number) => ({
    id,
    tag_name: `v5.0.0-alpha.${id}`,
    draft: false,
    assets: [] as { name: string }[],
  });
  function api(
    count: number,
    mutate?: (rows: ReturnType<typeof release>[], page: number, call: number) => void
  ) {
    const rows = Array.from({ length: count }, (_, i) => release(count - i));
    const execute = vi.fn((executable: string, args: string[]) => {
      expect(executable).toBe('gh');
      expect(args).toContain('--include');
      expect(args.at(-1)).toBe('GET');
      const url = new URL(`https://api.github.com/${args[1]}`);
      const page = Number(url.searchParams.get('page'));
      mutate?.(rows, page, execute.mock.calls.length);
      const pages = Math.max(1, Math.ceil(rows.length / 100));
      const links =
        pages > 1
          ? `Link: <https://api.github.com/repos/pj-tmt/tmt/releases?per_page=100&page=${pages}>; rel="last"\r\n`
          : '';
      return `HTTP/2.0 200 OK\r\n${links}\r\n${JSON.stringify(rows.slice((page - 1) * 100, page * 100))}`;
    });
    return { rows, execute };
  }
  const response = (rows: unknown) => `HTTP/2.0 200 OK\n\n${JSON.stringify(rows)}`;

  it('fully paginates allocated releases and never exports credentials', () => {
    const f = api(101);
    f.rows[100].draft = true;
    f.rows[100].assets = [{ name: 'verification-failed.json' }];
    const result = readCutMetadata(input, f.execute);
    expect(result.releases).toEqual(f.rows);
    expect(f.execute).toHaveBeenCalledTimes(4);
    expect(JSON.stringify(result)).not.toContain(input.token);
  });
  it.each([0, 100, 1201, 5800])('reads %i releases under the P+2 request rule', (count) => {
    const f = api(count);
    expect(readCutMetadata(input, f.execute).releases).toEqual(f.rows);
    expect(f.execute).toHaveBeenCalledTimes(Math.max(1, Math.ceil(count / 100)) + 2);
  });
  it('fails closed one page beyond the maximum without increasing the request cap', () => {
    const f = api(5801);
    expect(() => readCutMetadata(input, f.execute)).toThrow('budget');
    expect(f.execute).toHaveBeenCalledTimes(1);
  });
  it('restarts the entire sweep after an insertion between pages', () => {
    const f = api(250, (rows, page, call) => {
      if (call === 3 && page === 2) rows.unshift(release(251));
    });
    expect(readCutMetadata(input, f.execute).releases).toEqual(f.rows);
    expect(f.execute).toHaveBeenCalledTimes(8); // Duplicate detection at 3, then a quiet five-request sweep.
    expect(
      f.execute.mock.calls.map(([, args]) =>
        Number(new URL(`https://fixture/${args[1]}`).searchParams.get('page'))
      )
    ).toEqual([1, 3, 2, 1, 3, 2, 3, 1]);
  });
  it('detects a mid-list removal even when the first page remains unchanged', () => {
    const f = api(350, (rows, page, call) => {
      if (call === 4 && page === 3) rows.splice(120, 1);
    });
    expect(readCutMetadata(input, f.execute).releases).toEqual(f.rows);
    expect(f.execute).toHaveBeenCalledTimes(10); // The anchored terminal converts the shift into a duplicate.
  });
  it.each([301, 350])('retries a terminal boundary changed by removal from %i records', (count) => {
    const f = api(count, (rows, _page, call) => {
      if (call === 5) rows.splice(120, 1);
    });
    expect(readCutMetadata(input, f.execute).releases).toEqual(f.rows);
    expect(f.execute).toHaveBeenCalledTimes(count === 301 ? 11 : 12);
  });
  it('detects a first-page removal between discovery and the terminal anchor', () => {
    const f = api(250, (rows, _page, call) => {
      if (call === 2) rows.splice(10, 1);
    });
    expect(readCutMetadata(input, f.execute).releases).toEqual(f.rows);
    expect(f.execute).toHaveBeenCalledTimes(10);
  });
  it('fails closed after three attempts when each attempt inserts another record', () => {
    const f = api(250, (rows, page) => {
      if (page === 2) rows.unshift(release(rows.length + 1));
    });
    expect(() => readCutMetadata(input, f.execute)).toThrow('three attempts');
    expect(f.execute).toHaveBeenCalledTimes(9);
  });
  it('shares the request budget across retries instead of resetting it', () => {
    const f = api(3001, (rows, page) => {
      if (page === 2) rows.unshift(release(rows.length + 1));
    });
    expect(() => readCutMetadata(input, f.execute)).toThrow('budget');
    expect(f.execute.mock.calls.length).toBeLessThanOrEqual(60);
  });
  it('shares the time budget across attempts and calls', () => {
    const now = vi.spyOn(Date, 'now');
    now.mockReturnValue(0);
    const f = api(250, (_rows, _page, call) => {
      if (call === 1) now.mockReturnValue(90_000);
    });
    try {
      expect(() => readCutMetadata(input, f.execute)).toThrow('budget');
      expect(f.execute).toHaveBeenCalledTimes(1);
    } finally {
      now.mockRestore();
    }
  });
  it.each([undefined, null, {}, [null], [{}], [{ name: 42 }]])(
    'refuses malformed assets without retry (%j)',
    (assets) => {
      const execute = vi.fn(() => response([{ ...release(1), assets }]));
      expect(() => readCutMetadata(input, execute)).toThrow('asset metadata');
      expect(execute).toHaveBeenCalledTimes(1);
    }
  );
  it.each([undefined, null, 1.5, '1'])('refuses invalid identities without retry (%j)', (id) => {
    const execute = vi.fn(() => response([{ ...release(1), id }]));
    expect(() => readCutMetadata(input, execute)).toThrow('identity');
    expect(execute).toHaveBeenCalledTimes(1);
  });
  it('refuses invisible drafts, malformed records and malformed pagination', () => {
    expect(() => readCutMetadata({ ...input, draftVisibility: '' })).toThrow('visibility');
    expect(() =>
      readCutMetadata(input, () => response([{ ...release(1), draft: undefined }]))
    ).toThrow('metadata');
    expect(() => readCutMetadata(input, () => JSON.stringify([]))).toThrow('headers');
    expect(() =>
      readCutMetadata(
        input,
        () => 'HTTP/2.0 200 OK\nLink: <https://fixture?page=2>; rel="next"\n\n[]'
      )
    ).toThrow('pagination');
  });
  it('never accepts duplicate records as a deduplicated catalogue', () => {
    const execute = vi.fn(() => response([release(1), release(1)]));
    expect(() => readCutMetadata(input, execute)).toThrow('duplicate');
    expect(execute).toHaveBeenCalledTimes(3);
  });
});
