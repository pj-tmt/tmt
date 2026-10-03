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
  renderCutNotes,
  renderCutSummary,
  type CutCommit,
  type CutMetadata,
} from '../../scripts/release-cut.mjs';

const sha = (n: number) => n.toString(16).padStart(40, 'a');
const definitions = {
  cli: { owns: ['.'], excludes: ['extensions', 'shared'], package: 'tmt-cli' },
  squad: { owns: ['extensions/squad'], package: 'tmt-squad', selectedBy: ['tests/squad.*'] },
  private: { owns: ['shared'], release: false, releaseConsumers: ['squad'] },
  parked: { owns: ['extensions/parked'], release: false, package: 'tmt-parked' },
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
  it('uses ownership, exclusions, selectedBy and declared consumers, deduplicating multi-path changes', () => {
    const commits = [
      commit('feat: CLI', ['rust/x', 'extensions/parked/x']),
      commit('feat: private leaf', ['shared/a', 'shared/b'], 2),
      commit('fix: selected test', ['tests/squad.test'], 3),
      commit('feat: parked only', ['extensions/parked/a'], 4),
    ];
    expect(attributeCutCommits(commits, map, 'cli').map((c) => c.sha)).toEqual([sha(1)]);
    expect(attributeCutCommits(commits, map, 'squad').map((c) => c.sha)).toEqual([sha(2), sha(3)]);
  });
  it('rejects unknown, parked or non-private consumption instead of a second attribution list', () => {
    for (const consumers of [['missing'], ['parked']]) {
      expect(() =>
        parseComponentMap(
          JSON.stringify({
            components: {
              ...definitions,
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
  it('preserves CLI plus Squad for style/invoke, and Squad only for CLI-excluded TUI', async () => {
    const baseline = {
      cli: {
        owns: ['.'],
        excludes: ['extensions/squad', 'rust/crates/tmt-tui'],
        package: 'tmt-cli',
      },
      squad: { owns: ['extensions/squad'], package: 'tmt-squad' },
      tui: { owns: ['rust/crates/tmt-tui'], release: false, releaseConsumers: ['squad'] },
    };
    const extended = parseComponentMap(
      JSON.stringify({
        components: {
          ...baseline,
          style: {
            owns: ['rust/crates/tmt-cli-style'],
            release: false,
            releaseConsumers: ['squad'],
          },
          invoke: { owns: ['rust/crates/tmt-invoke'], release: false, releaseConsumers: ['squad'] },
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
    const squad = await render(attributeCutCommits(commits, extended, 'squad'));
    expect(squad.commits).toEqual([sha(1), sha(2), sha(3)]);
    expect(squad.notes).toContain('style change');
    expect(squad.notes).toContain('invocation change');
    expect(squad.notes).toContain('TUI change');
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
        'chore: aggregate\n\nfix(cli): first fix\n\nBEGIN_NESTED_COMMIT\nfeat(squad)!: nested feature\n\nBREAKING CHANGE: changed shape\nEND_NESTED_COMMIT'
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
    const componentMap = parseComponentMap(JSON.stringify(fixture.map));
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
      { tag_name: 'tmt-squad-v0.1.0-alpha.13', draft: false },
    ],
    runs: [],
  };
  const git = vi.fn((args: string[]) => {
    if (args[0] === 'merge-base') return '';
    if (args[0] === 'rev-parse') return sha(0);
    if (args[0] === 'log') return `${sha(1)}\0feat: feature\n\0\n`;
    if (args[0] === 'show') return `${sha(0)} ${sha(8)}`;
    if (args[0] === 'diff') return 'rust/file.rs\0';
    throw new Error(`Unexpected git ${args}`);
  });
  return { metadata, git, map, date: '2026-10-03' };
}
describe('immutable plans and in-flight guards', () => {
  it('computes a cut/version/notes with ordinary tag ancestry, without a mutation client', async () => {
    const fixture = planningFixture();
    const result = await planReleaseCuts(fixture);
    expect(result.mode).toBe('shadow');
    expect(result.components.map((c) => [c.product, c.status])).toEqual([
      ['cli', 'proposed'],
      ['squad', 'no-releasable-commits'],
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
  it.each(['queued', 'in_progress', 'requested', 'waiting', 'pending'])(
    'blocks only a known product for native status %s',
    async (status) => {
      const fixture = planningFixture();
      fixture.metadata.runs!.push({ id: 1, status, display_title: 'Native release: cli' });
      const result = await planReleaseCuts(fixture);
      expect(result.components[0].reason).toContain('queued/running');
      expect(result.components[1].status).toBe('no-releasable-commits');
    }
  );
  it('blocks ambiguous/unknown native runs and component drafts; completed runs do not hold a cut', async () => {
    for (const title of ['Native release artifacts', 'Native release: unknown']) {
      const fixture = planningFixture();
      fixture.metadata.runs!.push({ id: 1, status: 'waiting', display_title: title });
      expect((await planReleaseCuts(fixture)).components.every((c) => c.status === 'blocked')).toBe(
        true
      );
    }
    const fixture = planningFixture();
    fixture.metadata.runs!.push({
      id: 1,
      status: 'completed',
      display_title: 'Native release: cli',
    });
    fixture.metadata.releases!.push({ tag_name: 'tmt-squad-v0.1.0-alpha.14', draft: true });
    const result = await planReleaseCuts(fixture);
    expect(result.components[0].status).toBe('proposed');
    expect(result.components[1].reason).toContain('draft is in flight');
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
  it('uses a validated bootstrap cut and an explicitly supplied first alpha seed, never Cargo versions', async () => {
    const fixture = planningFixture();
    fixture.metadata.releases = [];
    fixture.map = parseComponentMap(
      JSON.stringify({
        components: {
          cli: { ...definitions.cli, bootstrapSha: sha(0) },
        },
      })
    );
    const result = await planReleaseCuts({ ...fixture, initialVersions: { cli: '5.0.0-alpha.1' } });
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
  it('fully paginates releases, reads every active status, and never exports credentials', () => {
    const execute = vi.fn((executable: string, args: string[]) => {
      expect(executable).toBe('gh');
      expect(args.at(-1)).toBe('GET');
      const endpoint = args[1];
      if (endpoint.includes('/releases?')) {
        const releases = endpoint.endsWith('page=1')
          ? Array.from({ length: 100 }, (_, i) => ({
              id: i + 1,
              tag_name: `v5.0.0-alpha.${i}`,
              draft: false,
            }))
          : [{ id: 101, tag_name: 'v5.0.0-alpha.101', draft: true }];
        return JSON.stringify(releases);
      }
      return JSON.stringify({ workflow_runs: [], total_count: 0 });
    });
    const result = readCutMetadata(input, execute);
    expect(result.releases).toHaveLength(101);
    expect(execute).toHaveBeenCalledTimes(7);
    expect(JSON.stringify(result)).not.toContain(input.token);
  });
  it('refuses invisible drafts, incomplete pages/counts, duplicate records and missing statuses', () => {
    expect(() => readCutMetadata({ ...input, draftVisibility: '' })).toThrow('visibility');
    expect(() =>
      readCutMetadata(input, () =>
        JSON.stringify(
          Array.from({ length: 100 }, (_, i) => ({
            id: i,
            tag_name: 'v1.0.0-alpha.1',
            draft: false,
          }))
        )
      )
    ).toThrow('pagination');
    for (const page of [
      { workflow_runs: [], total_count: 1 },
      { workflow_runs: [], total_count: undefined },
    ]) {
      expect(() =>
        readCutMetadata(input, (_exe, args) =>
          JSON.stringify(args[1].includes('/releases?') ? [] : page)
        )
      ).toThrow();
    }
    expect(() => readCutMetadata(input, () => JSON.stringify([{ id: 1 }, { id: 1 }]))).toThrow(
      'duplicate'
    );
  });
});
