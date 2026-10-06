import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it, vi } from 'vite-plus/test';
import { fileURLToPath } from 'node:url';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import {
  affectedProducts,
  applyUpdates,
  githubApi,
  gitEvidence,
  LIMITS,
  planUpdates,
  readClosingPrs,
  readProject,
  readReleases,
  reconcile,
  releaseIdentity,
  renderSummary,
  renderFailure,
  type Api,
  type ClosingPr,
  type Project,
  type ProjectItem,
  type Release,
} from '../../scripts/project-release.mjs';

const repository = 'pj-tmt/tmt';
const map = parseComponentMap(
  readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
);
const workspace = readCargoWorkspace(fileURLToPath(new URL('../../../', import.meta.url)));
// Synthetic files live only in temporary repositories; derive the product root from the map.
const squadRoot = map.components.find((component) => component.name === 'squad')!.owns[0];
const connection = (nodes: unknown[], cursor: string | null = null) => ({
  nodes,
  pageInfo: { hasNextPage: !!cursor, endCursor: cursor },
});
const item = (number: number, status = 'Merged', text = ''): ProjectItem => ({
  id: `item-${number}`,
  content: {
    __typename: 'Issue',
    id: `issue-${number}`,
    number,
    url: `https://github.com/${repository}/issues/${number}`,
    state: 'CLOSED',
    repository: { nameWithOwner: repository },
    labels: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } },
  },
  status: { name: status },
  released: { text },
});
const project = (items = [item(1)]): Project => ({
  projectId: 'project',
  statusId: 'status',
  releasedId: 'released',
  options: Object.fromEntries(
    ['Todo', 'In Progress', 'In Review', 'Merged', 'Released', 'Done'].map((name) => [name, name])
  ),
  pages: 1,
  items: new Map(items.map((row) => [row.content.id, row])),
});
const release = (tag_name: string, minute = 0): Release => ({
  tag_name,
  draft: false,
  published_at: `2026-10-01T00:${String(minute).padStart(2, '0')}:00Z`,
  body: 'No PR links in these notes.',
});
function fakeApi(rest: Api['rest'] = () => [], graphql: Api['graphql'] = () => ({})): Api {
  const counts = { graphql: 0, rest: 0 };
  const points = { cost: 0, remaining: null as number | null };
  return {
    counts,
    points,
    rest: vi.fn((url) => {
      counts.rest++;
      return rest(url);
    }),
    graphql: vi.fn((query) => {
      counts.graphql++;
      if (query.startsWith('query')) {
        points.cost++;
        points.remaining = 5000 - points.cost;
      }
      return graphql(query);
    }),
    reserve: vi.fn(),
  };
}
function projectResponse(p: Project, cursor: string | null = null) {
  return {
    node: {
      fields: connection([
        {
          id: p.statusId,
          name: 'Status',
          options: Object.entries(p.options).map(([name, id]) => ({ id, name })),
        },
        { id: p.releasedId, name: 'Released in', dataType: 'TEXT' },
      ]),
      items: connection([...p.items.values()], cursor),
    },
  };
}
function stateApi(
  p: Project,
  releases: Release[],
  prs: Map<string, ClosingPr[]>,
  onProjectRead?: (project: Project, read: number) => void
): Api {
  let reads = 0;
  return fakeApi(
    () => releases,
    (query) => {
      if (query.startsWith('mutation')) {
        const result: Record<string, unknown> = {};
        for (const match of query.matchAll(
          /(u\d+):(update|clear)ProjectV2ItemFieldValue\(input:\{projectId:"[^"]+",itemId:"([^"]+)",fieldId:"([^"]+)"(?:,value:\{(?:text|singleSelectOptionId):("(?:[^"\\]|\\.)*")\})?\}\)/g
        )) {
          const [, alias, method, id, field, value] = match;
          const row = [...p.items.values()].find((entry) => entry.id === id)!;
          if (field === 'status') row.status = { name: JSON.parse(value) };
          else row.released = { text: method === 'clear' ? '' : JSON.parse(value) };
          result[alias] = { projectV2Item: { id } };
        }
        return result;
      }
      if (query.includes('closedByPullRequestsReferences')) {
        return Object.fromEntries(
          [...query.matchAll(/(i\d+):node\(id:"([^"]+)"\)/g)].map(([, alias, id]) => [
            alias,
            { id, state: 'CLOSED', closedByPullRequestsReferences: connection(prs.get(id) || []) },
          ])
        );
      }
      onProjectRead?.(p, ++reads);
      return structuredClone(projectResponse(p));
    }
  );
}
function history(
  run: (input: {
    directory: string;
    git: (args: string[]) => string;
    commit: (files: string[]) => string;
  }) => void
) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-project-release-'));
  const env = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' };
  const git = (args: string[]) => {
    const result = spawnSync('git', args, {
      cwd: directory,
      env,
      encoding: 'utf8',
      timeout: 10_000,
    });
    if (result.status !== 0) throw new Error(result.stderr);
    return result.stdout.trim();
  };
  let sequence = 0;
  const commit = (files: string[]) => {
    for (const file of files) {
      mkdirSync(path.dirname(path.join(directory, file)), { recursive: true });
      writeFileSync(path.join(directory, file), `revision ${++sequence}`);
    }
    git(['add', '.']);
    git([
      '-c',
      'user.name=TMT Test',
      '-c',
      'user.email=test@example.invalid',
      '-c',
      'commit.gpgsign=false',
      'commit',
      '--quiet',
      '-m',
      'fixture\n\nCo-authored-by: Codex <codex@openai.com>',
    ]);
    return git(['rev-parse', 'HEAD']);
  };
  try {
    git(['init', '--quiet']);
    commit(['initial']);
    run({ directory, git, commit });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
const closingPr = (number: number, oid: string): ClosingPr => ({
  id: `pr-${number}`,
  number,
  merged: true,
  mergeCommit: { oid },
  repository: { nameWithOwner: repository },
});
const writes = (api: Api) =>
  vi.mocked(api.graphql).mock.calls.filter(([query]) => query.startsWith('mutation'));

describe('full repository-state release sweep', () => {
  it('reconciles the exact #1813 paths without publication and retains mixed containing-tag waits', () =>
    history(({ directory, git, commit }) => {
      const paths = [
        `${map.components.find((c) => c.name === 'cli')!.neverShippedPaths[0].root}/development.md`,
        `${map.components.find((c) => c.name === 'tmt-colab')!.neverShippedPaths[0].root}/cli.rs`,
      ];
      const never = commit(paths);
      const mixed = commit([
        ...paths,
        'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
        'rust/crates/tmt-cli/src/main.rs',
      ]);
      const p = project([item(1812, 'Merged', 'stale release'), item(2)]);
      const prs = new Map([
        ['issue-1812', [closingPr(1813, never)]],
        ['issue-2', [closingPr(2, mixed)]],
      ]);
      const releases: Release[] = [];
      const api = stateApi(p, releases, prs);
      const input = {
        api,
        repository,
        git: gitEvidence({ cwd: directory }),
        map,
        workspace,
        dryRun: true,
      };
      expect(affectedProducts(paths, map, workspace)).toEqual({ products: [], unpublished: [] });
      expect(
        affectedProducts(['.agents/skills/tmt-colab/references-other/guide.md'], map, workspace)
          .products
      ).toEqual(['cli']);
      expect(() =>
        affectedProducts(
          ['unknown'],
          parseComponentMap(JSON.stringify({ components: { cli: { owns: ['rust'] } } }))
        )
      ).toThrow('No component owns');
      const preview = reconcile(input);
      expect(preview.changed).toMatchObject([
        { issue: `https://github.com/${repository}/issues/1812`, status: 'Done', text: '' },
      ]);
      expect(writes(api)).toHaveLength(0);
      reconcile({ ...input, dryRun: false });
      expect(p.items.get('issue-1812')!.status!.name).toBe('Done');
      expect(p.items.get('issue-1812')!.released!.text).toBe('');
      expect(reconcile(input).changed).toEqual([]);
      git(['tag', 'tmt-colab-v0.1.0-alpha.1']);
      releases.push(release('tmt-colab-v0.1.0-alpha.1'));
      reconcile({ ...input, dryRun: false });
      expect(p.items.get('issue-2')!.status!.name).toBe('Merged');
      git(['tag', 'v5.0.0-alpha.1']);
      releases.push(release('v5.0.0-alpha.1', 1));
      reconcile({ ...input, dryRun: false });
      expect(p.items.get('issue-2')!.status!.name).toBe('Released');
      expect(reconcile(input).changed).toEqual([]);
    }));

  it('repairs old/not-in-notes issues, waits for every product, repairs built-in drift, and reruns without writes', () =>
    history(({ directory, git, commit }) => {
      const docs = commit(['docs/fixture.md']);
      git(['tag', 'v5.0.0-alpha.1']);
      const squad = commit([`${squadRoot}/input`]);
      git(['tag', 'tmt-squad-v0.1.0-alpha.1']);
      const mixed = commit(['rust/input', `${squadRoot}/input`]);
      git(['tag', 'tmt-squad-v0.1.0-alpha.2']);
      const releases = [release('v5.0.0-alpha.1'), release('tmt-squad-v0.1.0-alpha.1', 1)];
      for (let i = 2; i <= 15; i++) {
        git(['tag', `v5.0.0-alpha.${i}`]);
        releases.push(release(`v5.0.0-alpha.${i}`, i));
      }
      const pending = commit(['rust/input']);
      git(['tag', 'v5.0.0-alpha.99']);
      releases.push({ ...release('v5.0.0-alpha.99', 30), draft: true });
      const p = project([
        item(1),
        item(2),
        item(3, 'Released', 'incorrect release'),
        item(4),
        item(5),
        item(6, 'In Progress'),
      ]);
      p.items.get('issue-6')!.content.state = 'OPEN';
      const prs = new Map([
        ['issue-1', [closingPr(101, docs)]],
        ['issue-2', [closingPr(102, squad)]],
        ['issue-3', [closingPr(103, mixed)]],
        ['issue-5', [closingPr(105, pending)]],
      ]);
      const api = stateApi(p, releases, prs);
      const input = { api, repository, git: gitEvidence({ cwd: directory }), map, dryRun: true };
      const preview = reconcile(input);
      expect(writes(api)).toHaveLength(0);
      expect(
        preview.changed.map(({ issue, status, text }) => [issue.split('/').pop(), status, text])
      ).toEqual([
        ['1', 'Released', 'tmt-cli 5.0.0-alpha.1'],
        ['2', 'Released', 'tmt-squad 0.1.0-alpha.1'],
        ['3', 'Merged', 'tmt-cli 5.0.0-alpha.2'],
        ['4', 'Done', ''],
      ]);
      expect(preview.requests).toEqual({ rest: 1, graphql: 2 });
      expect(renderSummary(preview)).toContain('Released → Merged');
      expect(renderSummary(preview)).toContain('incorrect release → tmt-cli 5.0.0-alpha.2');
      expect(renderSummary(preview)).toContain('REST 1/20; GraphQL 2/200');
      const first = reconcile({ ...input, dryRun: false });
      expect(first.changed).toEqual(preview.changed);
      expect(p.items.get('issue-6')!.status!.name).toBe('In Progress');
      const writeCount = writes(api).length;
      expect(reconcile({ ...input, dryRun: false }).changed).toEqual([]);
      expect(writes(api)).toHaveLength(writeCount);
      // Model a late built-in close/merge event. Full sweeps do not trust existing status.
      p.items.get('issue-1')!.status = { name: 'Merged' };
      p.items.get('issue-4')!.status = { name: 'Merged' };
      expect(reconcile({ ...input, dryRun: false }).changed.map((row) => row.status)).toEqual([
        'Released',
        'Done',
      ]);
      releases.push(release('tmt-squad-v0.1.0-alpha.2', 31));
      expect(reconcile({ ...input, dryRun: false }).changed).toMatchObject([
        { status: 'Released', text: 'tmt-cli 5.0.0-alpha.2\ntmt-squad 0.1.0-alpha.2' },
      ]);
      expect(p.items.get('issue-5')!.status!.name).toBe('Merged');
    }));

  it('skips a closed epic without a PR and an epic with merged delivery, while reconciling its child', () =>
    history(({ directory, git, commit }) => {
      const sha = commit(['rust/input']);
      git(['tag', 'v5.0.0-alpha.1']);
      const noPr = item(7, 'Released', 'owner acceptance evidence');
      const withDelivery = item(8, 'In Progress', 'dogfood pending');
      for (const epic of [noPr, withDelivery]) epic.content.labels.nodes.push({ name: 'epic' });
      const child = item(9);
      Object.assign(child.content, { parent: { id: withDelivery.content.id } });
      const p = project([noPr, withDelivery, child]);
      const api = stateApi(
        p,
        [release('v5.0.0-alpha.1')],
        new Map([
          ['issue-8', [closingPr(80, sha)]],
          ['issue-9', [closingPr(90, sha)]],
        ])
      );
      const before = structuredClone([noPr, withDelivery]);
      const result = reconcile({
        api,
        repository,
        dryRun: false,
        git: gitEvidence({ cwd: directory }),
        map,
      });
      expect([noPr, withDelivery]).toEqual(before);
      expect(result.changed.map((row) => row.itemId)).toEqual(['item-9']);
      expect(
        result.rows
          .filter((row) => row.waiting.includes('skipped: epic tracker'))
          .map((row) => row.itemId)
      ).toEqual(['item-7', 'item-8']);
      expect(renderSummary(result)).toContain('skipped: epic tracker');
      for (const [query] of vi.mocked(api.graphql).mock.calls) {
        if (query.includes('closedByPullRequestsReferences'))
          expect(query).not.toMatch(/issue-[78]/);
        if (query.startsWith('mutation')) expect(query).not.toMatch(/item-[78]/);
      }
    }));

  it('does not count another product or an unpublished tag; multiple closing PRs must all be contained', () =>
    history(({ directory, git, commit }) => {
      const a = commit([`${squadRoot}/input`]);
      git(['tag', 'v5.0.0-alpha.1']);
      git(['tag', 'tmt-squad-v0.1.0-alpha.1']);
      const b = commit([`${squadRoot}/input`]);
      git(['tag', 'tmt-squad-v0.1.0-alpha.2']);
      const p = project();
      const releases = [release('v5.0.0-alpha.1')];
      const api = stateApi(p, releases, new Map([['issue-1', [closingPr(1, a), closingPr(2, b)]]]));
      const input = { api, repository, dryRun: true, git: gitEvidence({ cwd: directory }), map };
      expect(reconcile(input).rows[0]).toMatchObject({
        status: 'Merged',
        text: '',
        waiting: ['Awaiting squad'],
      });
      releases.push(release('tmt-squad-v0.1.0-alpha.1'));
      expect(reconcile(input).rows[0].status).toBe('Merged');
      releases.push(release('tmt-squad-v0.1.0-alpha.2', 1));
      expect(reconcile(input).rows[0]).toMatchObject({
        status: 'Released',
        text: 'tmt-squad 0.1.0-alpha.2',
      });
    }));

  it('waits for the driver publication and attributes a driver-only commit without a CLI release', () =>
    history(({ directory, git, commit }) => {
      const driverRoot = map.components.find((component) => component.name === 'driver-herdr')!
        .owns[0];
      const sha = commit([`${driverRoot}/src/main.rs`]);
      git(['tag', 'v5.0.0-alpha.1']);
      git(['tag', 'tmt-driver-herdr-v0.1.0-alpha.1']);
      const releases = [release('v5.0.0-alpha.1')];
      const api = stateApi(project(), releases, new Map([['issue-1', [closingPr(1, sha)]]]));
      const input = { api, repository, dryRun: true, git: gitEvidence({ cwd: directory }), map };
      expect(reconcile(input).rows[0]).toMatchObject({
        status: 'Merged',
        text: '',
        waiting: ['Awaiting driver-herdr'],
      });
      releases.push(release('tmt-driver-herdr-v0.1.0-alpha.1', 1));
      expect(reconcile(input).rows[0]).toMatchObject({
        status: 'Released',
        text: 'tmt-driver-herdr 0.1.0-alpha.1',
        waiting: [],
      });
      expect(writes(api)).toHaveLength(0);
    }));

  it('uses publication time rather than version precedence, and rejects missing git evidence', () =>
    history(({ directory, git, commit }) => {
      const sha = commit(['rust/input']);
      git(['tag', 'v5.0.0-alpha.2']);
      git(['tag', 'v5.0.0-alpha.1']);
      const p = project();
      const releases = [release('v5.0.0-alpha.1', 2), release('v5.0.0-alpha.2', 1)];
      const api = stateApi(p, releases, new Map([['issue-1', [closingPr(1, sha)]]]));
      const input = { api, repository, dryRun: true, git: gitEvidence({ cwd: directory }), map };
      expect(reconcile(input).rows[0].text).toBe('tmt-cli 5.0.0-alpha.2');
      releases.push(release('v5.0.0-alpha.3', 3));
      expect(() => reconcile(input)).toThrow('Git evidence failed');
      expect(writes(api)).toHaveLength(0);
    }));

  it('fetches a tag published after checkout from origin, and names a tag that never resolves', () =>
    history(({ directory, git }) => {
      const origin = mkdtempSync(path.join(tmpdir(), 'tmt-project-release-origin-'));
      try {
        spawnSync('git', ['init', '--bare', '-q', origin], { timeout: 10_000 });
        git(['remote', 'add', 'origin', origin]);
        git(['tag', 'v5.0.0-alpha.1']);
        // The sibling published alpha.2 after this checkout's tag fetch.
        git(['tag', 'v5.0.0-alpha.2']);
        git(['push', '-q', 'origin', 'HEAD:refs/heads/main', 'v5.0.0-alpha.1', 'v5.0.0-alpha.2']);
        git(['tag', '-d', 'v5.0.0-alpha.2']);
        const evidence = gitEvidence({ cwd: directory });
        expect(() => evidence.validateTags(['v5.0.0-alpha.1', 'v5.0.0-alpha.2'])).not.toThrow();
        expect(git(['tag', '--list', 'v5.0.0-alpha.2'])).toBe('v5.0.0-alpha.2');
        expect(() => evidence.validateTags(['v5.0.0-alpha.1', 'v5.0.0-alpha.7'])).toThrow(
          /do not resolve to a commit: v5\.0\.0-alpha\.7/
        );
        expect(git(['tag', '--list', 'v5.0.0-alpha.7'])).toBe('');
      } finally {
        rmSync(origin, { recursive: true, force: true });
      }
    }));

  it('uses ancestry across branches and includes deletion and both rename owners', () =>
    history(({ directory, git, commit }) => {
      const ancestor = git(['rev-parse', 'HEAD']);
      const unrelated = commit(['unrelated']);
      git(['tag', 'v5.0.0-alpha.1']);
      git(['checkout', '--detach', ancestor]);
      const merged = commit(['docs/fixture.md']);
      const evidence = gitEvidence({ cwd: directory });
      expect(evidence.containingTags(merged).has('v5.0.0-alpha.1')).toBe(false);
      expect(evidence.containingTags(unrelated).has('v5.0.0-alpha.1')).toBe(true);
      mkdirSync(path.join(directory, squadRoot), { recursive: true });
      git(['mv', 'docs/fixture.md', `${squadRoot}/renamed.md`]);
      git(['rm', 'initial']);
      const rename = commit(['additional']);
      expect(evidence.paths(rename)).toEqual(
        expect.arrayContaining(['docs/fixture.md', `${squadRoot}/renamed.md`, 'initial'])
      );
      expect(affectedProducts(evidence.paths(rename), map).products).toEqual(['cli', 'squad']);
    }));

  it.each(['tmt-cli-style', 'tmt-invoke'])(
    'preserves CLI membership while adding its released extension consumers for %s',
    (leaf) => {
      const file = `rust/crates/${leaf}/src/lib.rs`;
      const before = {
        ...map,
        components: map.components.filter((component) => component.name !== leaf),
      };
      expect(affectedProducts([file], before)).toEqual({ products: ['cli'], unpublished: [] });
      expect(affectedProducts([file], map, workspace)).toEqual({
        products: ['cli', 'colab', 'remote', 'squad'],
        unpublished: [],
      });
      expect(affectedProducts([`rust/crates/${leaf}-other/src/lib.rs`], map)).toEqual({
        products: ['cli'],
        unpublished: [],
      });
    }
  );

  it('keeps the explicitly excluded TUI leaf attributed only to Squad', () => {
    expect(affectedProducts(['rust/crates/tmt-tui/src/lib.rs'], map, workspace)).toEqual({
      products: ['squad'],
      unpublished: [],
    });
  });

  it.each(['tmt-cli-style', 'tmt-invoke'])(
    'requires every published consuming product for a %s change',
    (leaf) =>
      history(({ directory, git, commit }) => {
        const sha = commit([`rust/crates/${leaf}/src/lib.rs`]);
        git(['tag', 'v5.0.0-alpha.2']);
        git(['tag', 'tmt-squad-v0.1.0-alpha.1']);
        const releases = [release('tmt-squad-v0.1.0-alpha.1')];
        const api = stateApi(project(), releases, new Map([['issue-1', [closingPr(1, sha)]]]));
        const input = {
          api,
          repository,
          dryRun: true,
          git: gitEvidence({ cwd: directory }),
          map,
          workspace,
        };
        expect(reconcile(input).rows[0]).toMatchObject({
          status: 'Merged',
          text: 'tmt-squad 0.1.0-alpha.1',
          waiting: ['Awaiting cli', 'Awaiting colab', 'Awaiting remote'],
        });
        releases.push(release('v5.0.0-alpha.2', 1));
        expect(reconcile(input).rows[0]).toMatchObject({
          status: 'Merged',
          waiting: ['Awaiting colab', 'Awaiting remote'],
        });
        for (const product of ['remote', 'colab']) {
          const tag = `tmt-${product}-v0.1.0-alpha.1`;
          git(['tag', tag]);
          releases.push(release(tag, 2));
        }
        const row = reconcile(input).rows[0];
        expect(row.status).toBe('Released');
        expect(row.text).toContain('tmt-cli 5.0.0-alpha.2');
        expect(row.text).toContain('tmt-squad 0.1.0-alpha.1');
        expect(row.waiting).toEqual([]);
        expect(writes(api)).toHaveLength(0);
      })
  );

  it('waits for both native products and maps embedded app/client merges to containing product tags', () =>
    history(({ directory, git, commit }) => {
      const sha = commit([
        'extensions/tmt-remote/typescript/remote-client/src/fixture.ts',
        'extensions/tmt-colab/typescript/app/src/fixture.ts',
        'extensions/tmt-colab/typescript/colab-client/src/fixture.ts',
      ]);
      expect(
        affectedProducts(
          ['extensions/tmt-remote/typescript/remote-client/src/fixture.ts'],
          map,
          workspace
        )
      ).toEqual({ products: ['remote'], unpublished: [] });
      const releases: Release[] = [];
      const api = stateApi(project(), releases, new Map([['issue-1', [closingPr(1, sha)]]]));
      const input = {
        api,
        repository,
        dryRun: true,
        git: gitEvidence({ cwd: directory }),
        map,
        workspace,
      };
      expect(reconcile(input).rows[0]).toMatchObject({
        status: 'Merged',
        waiting: ['Awaiting colab', 'Awaiting remote'],
      });
      git(['tag', 'tmt-remote-v0.1.0-alpha.1']);
      releases.push(release('tmt-remote-v0.1.0-alpha.1'));
      expect(reconcile(input).rows[0]).toMatchObject({
        status: 'Merged',
        waiting: ['Awaiting colab'],
      });
      git(['tag', 'tmt-colab-v0.1.0-alpha.1']);
      releases.push(release('tmt-colab-v0.1.0-alpha.1', 1));
      expect(reconcile(input).rows[0]).toMatchObject({
        status: 'Released',
        waiting: [],
        text: 'tmt-colab 0.1.0-alpha.1\ntmt-remote 0.1.0-alpha.1',
      });
      expect(writes(api)).toHaveLength(0);
    }));
  it('uses the owner map, selected paths and private consumers without silently releasing private components', () => {
    expect(
      affectedProducts(
        ['docs/fixture.md', 'typescript/test/native/squad.test.ts', 'rust/crates/tmt-tui/a.rs'],
        map,
        workspace
      )
    ).toEqual({ products: ['cli', 'squad'], unpublished: [] });
    expect(affectedProducts(['extensions/tmt-office/a.rs'], map).products).toEqual(['office']);
    expect(affectedProducts(['extensions/tmt-colab/rust/a.rs'], map)).toEqual({
      products: ['colab'],
      unpublished: [],
    });
    expect(releaseIdentity('tmt-squad-v0.1.0-alpha.9')?.product).toBe('squad');
    expect(affectedProducts(['rust/crates/tmt-driver-herdr/src/main.rs'], map)).toEqual({
      products: ['driver-herdr'],
      unpublished: [],
    });
    expect(releaseIdentity('tmt-driver-herdr-v0.1.0-alpha.1')?.label).toBe(
      'tmt-driver-herdr 0.1.0-alpha.1'
    );
    for (const tag of ['v4.2.1', 'v5.bad', 'unknown-v1.0.0'])
      expect(releaseIdentity(tag)).toBeUndefined();
  });
});

describe('bounded discovery and mutation safety', () => {
  it('paginates complete releases and closing relationships, including closed PRs', () => {
    const api = fakeApi((url) =>
      url.endsWith('page=1')
        ? Array.from({ length: 100 }, () => release('v5.0.0-alpha.1'))
        : [release('v5.0.0-alpha.2')]
    );
    expect(readReleases(api)).toHaveLength(101);
    const pr = closingPr(1, 'a'.repeat(40));
    const paged = fakeApi(undefined, (query) => ({
      i0: {
        id: 'issue-1',
        state: 'CLOSED',
        closedByPullRequestsReferences: connection(
          [pr],
          query.includes('after:"next"') ? null : 'next'
        ),
      },
    }));
    expect(readClosingPrs(paged, [item(1)], repository).prs.size).toBe(1);
    expect(paged.graphql).toHaveBeenCalledTimes(2);
    for (const [query] of vi.mocked(paged.graphql).mock.calls) {
      expect(query).toContain('includeClosedPrs:true');
      expect(query).toContain('closedByPullRequestsReferences(first:10,');
      expect(query).not.toContain('closedByPullRequestsReferences(first:100,');
      expect(query).toContain('rateLimit{cost remaining}');
    }
  });
  it('batches issue reads and follows Project pages, excluding foreign/open items from reconciliation', () => {
    const p = project([item(1)]);
    const api = fakeApi(undefined, (query) =>
      projectResponse(p, query.includes('after:"next"') ? null : 'next')
    );
    expect(readProject(api).pages).toBe(2);
    for (const [query] of vi.mocked(api.graphql).mock.calls) {
      expect(query).toContain('items(first:100,');
      expect(query).toContain('labels(first:20)');
      expect(query).not.toContain('labels(first:100)');
      expect(query).toContain('rateLimit{cost remaining}');
    }
    const rows = Array.from({ length: 26 }, (_, i) => item(i));
    const reads = stateApi(project(rows), [], new Map());
    readClosingPrs(reads, rows, repository);
    expect(reads.graphql).toHaveBeenCalledTimes(2);
    p.items.get('issue-1')!.content.repository.nameWithOwner = 'other/repository';
    const skipped = reconcile({
      api: stateApi(p, [], new Map()),
      repository,
      dryRun: false,
      git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
      map,
    });
    expect(skipped.issues).toBe(0);
  });
  it('fails incomplete pagination, schema, merge and changed issue evidence before writes', () => {
    expect(() =>
      readReleases(fakeApi(() => Array.from({ length: 100 }, () => release('v5.0.0-alpha.1'))))
    ).toThrow('pagination cap');
    const p = project();
    delete p.options.Done;
    expect(() => readProject(stateApi(p, [], new Map()))).toThrow('Project requires');
    const uncertainLabels = project();
    uncertainLabels.items.get('issue-1')!.content.labels.pageInfo = {
      hasNextPage: true,
      endCursor: 'more-labels',
    };
    expect(() => readProject(stateApi(uncertainLabels, [], new Map()))).toThrow(
      'Incomplete issue labels'
    );
    const cases = [
      { id: 'issue-1', state: 'OPEN' },
      {
        id: 'issue-1',
        state: 'CLOSED',
        closedByPullRequestsReferences: connection([closingPr(1, 'bad')]),
      },
      {
        id: 'issue-1',
        state: 'CLOSED',
        closedByPullRequestsReferences: connection([
          { ...closingPr(1, 'a'.repeat(40)), repository: { nameWithOwner: 'other/repo' } },
        ]),
      },
      {
        id: 'issue-1',
        state: 'CLOSED',
        closedByPullRequestsReferences: {
          nodes: [],
          pageInfo: { hasNextPage: true, endCursor: null },
        },
      },
    ];
    for (const value of cases) {
      const api = fakeApi(undefined, () => ({ i0: value }));
      expect(() => readClosingPrs(api, [item(1)], repository)).toThrow();
      expect(writes(api)).toHaveLength(0);
    }
  });
  it('clears false text, corrects false terminal states first, and retries partial evidence writes', () => {
    const p = project([item(1, 'Released', 'wrong'), item(2)]);
    const evidence = new Map([
      ['issue-1', { status: 'Done', text: '', prs: [], waiting: [] }],
      ['issue-2', { status: 'Released', text: 'tmt-cli 5.0.0-alpha.1', prs: [2], waiting: [] }],
    ]);
    const api = stateApi(p, [], new Map());
    applyUpdates(api, p, planUpdates(evidence, p), false);
    expect(writes(api)[0][0]).toContain('singleSelectOptionId:"Done"');
    expect(writes(api)[1][0]).toContain('clearProjectV2ItemFieldValue');
    expect(writes(api)[2][0]).toContain('singleSelectOptionId:"Released"');
    expect(planUpdates(evidence, p).changes).toEqual([]);
    p.items.get('issue-2')!.status = { name: 'Merged' };
    expect(planUpdates(evidence, p).changes).toMatchObject([
      { writeText: false, writeStatus: true },
    ]);
    const failure = fakeApi(undefined, () => {
      throw new Error('write failure');
    });
    expect(() => applyUpdates(failure, p, planUpdates(evidence, p), false)).toThrow(
      'write failure'
    );
    expect(failure.graphql).toHaveBeenCalledTimes(1);
  });
  it('reserves all mutation and readback requests before writes and rejects incomplete mutation responses', () => {
    const p = project(Array.from({ length: 26 }, (_, i) => item(i)));
    const evidence = new Map(
      [...p.items.keys()].map((id) => [id, { status: 'Done', text: '', prs: [], waiting: [] }])
    );
    const plan = planUpdates(evidence, p);
    const api = stateApi(p, [], new Map());
    applyUpdates(api, p, plan, true);
    expect(api.reserve).toHaveBeenCalledWith(2 + LIMITS.pages);
    expect(writes(api)).toHaveLength(0);
    api.reserve = () => {
      throw new Error('budget');
    };
    expect(() => applyUpdates(api, p, plan, false)).toThrow('budget');
    expect(writes(api)).toHaveLength(0);
    expect(() => applyUpdates(fakeApi(), p, plan, false)).toThrow('Incomplete Project mutation');
  });
  it('detects a readback mismatch instead of reporting an ignored write as success', () => {
    const p = project();
    const api = fakeApi(
      () => [],
      (query) => {
        if (query.startsWith('mutation')) return { u0: { projectV2Item: { id: 'item-1' } } };
        if (query.includes('closedByPullRequestsReferences'))
          return {
            i0: { id: 'issue-1', state: 'CLOSED', closedByPullRequestsReferences: connection([]) },
          };
        return projectResponse(p);
      }
    );
    expect(() =>
      reconcile({
        api,
        repository,
        dryRun: false,
        map,
        git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
      })
    ).toThrow('readback did not match');
    expect(writes(api)).toHaveLength(2);
  });

  it('reports every persistent mismatching issue and both fields, then stops after one correction', () => {
    const p = project([item(1), item(2, 'Released', 'stale\nevidence')]);
    const reportReadbackMismatch = vi.fn();
    let reads = 0;
    const api = fakeApi(
      () => [],
      (query) => {
        if (query.startsWith('mutation'))
          return Object.fromEntries(
            [...query.matchAll(/(u\d+):[^\n]+?itemId:"([^"]+)"/g)].map(([, alias, id]) => [
              alias,
              { projectV2Item: { id } },
            ])
          );
        if (query.includes('closedByPullRequestsReferences'))
          return Object.fromEntries(
            [...p.items.keys()].map((id, i) => [
              `i${i}`,
              { id, state: 'CLOSED', closedByPullRequestsReferences: connection([]) },
            ])
          );
        reads++;
        return projectResponse(p);
      }
    );
    let failure: Error | undefined;
    try {
      reconcile({
        api,
        repository,
        dryRun: false,
        map,
        reportReadbackMismatch,
        git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
      });
    } catch (error) {
      failure = error as Error;
    }
    expect(failure?.message).toContain('after one correction');
    const rendered = renderFailure(failure!, api);
    expect(rendered).toContain('attempt 2/2');
    expect(rendered).toContain('https://github.com/pj-tmt/tmt/issues/1 (item-1)');
    expect(rendered).toContain('https://github.com/pj-tmt/tmt/issues/2 (item-2)');
    expect(rendered).toContain('expected {"Status":"Done","Released in":""}');
    expect(rendered).toContain('observed {"Status":"Merged","Released in":""}');
    expect(rendered).toContain('observed {"Status":"Released","Released in":"stale\\nevidence"}');
    expect(reportReadbackMismatch).toHaveBeenCalledTimes(2);
    expect(reportReadbackMismatch.mock.calls[0][0]).toContain('attempt 1/2');
    expect(reads).toBe(3);
    expect(writes(api)).toHaveLength(4);
    expect(p.items.get('issue-2')!.released!.text).toBe('stale\nevidence');
  });

  it('repairs close-to-Done drift using fresh item IDs and only fields still wrong', () => {
    const sha = 'a'.repeat(40);
    const p = project([item(1, 'Released', 'stale'), item(2), item(3, 'Released', 'owner gate')]);
    p.items.get('issue-3')!.content.labels.nodes.push({ name: 'epic' });
    const reportReadbackMismatch = vi.fn();
    const api = stateApi(
      p,
      [],
      new Map([
        ['issue-1', [closingPr(1, sha)]],
        ['issue-2', [closingPr(2, sha)]],
      ]),
      (fresh, read) => {
        if (read !== 2) return;
        fresh.items.get('issue-1')!.id = 'replacement-item';
        fresh.items.get('issue-1')!.status = { name: 'Done' };
        fresh.items.get('issue-2')!.status = { name: 'Done' };
      }
    );
    const result = reconcile({
      api,
      repository,
      dryRun: false,
      map,
      workspace,
      reportReadbackMismatch,
      git: { validateTags() {}, paths: () => ['rust/input'], containingTags: () => new Set() },
    });
    expect(reportReadbackMismatch).toHaveBeenCalledTimes(1);
    expect(reportReadbackMismatch.mock.calls[0][0]).toContain('replacement-item');
    expect(reportReadbackMismatch.mock.calls[0][0]).toContain('/issues/2');
    expect(writes(api)).toHaveLength(3);
    const correction = writes(api)[2][0];
    expect(correction).toContain('itemId:"replacement-item"');
    expect(correction).toContain('itemId:"item-2"');
    expect(correction).not.toContain('fieldId:"released"');
    expect(correction).not.toContain('item-3');
    expect([...p.items.values()].map((row) => [row.status!.name, row.released!.text])).toEqual([
      ['Merged', ''],
      ['Merged', ''],
      ['Released', 'owner gate'],
    ]);
    expect(result.requests).toEqual({ graphql: 7, rest: 1 });
    expect(renderSummary(result)).toContain('REST 1/20; GraphQL 7/200');
  });

  it('corrects text-only drift without rewriting a settled status', () => {
    const p = project();
    const api = stateApi(p, [], new Map(), (fresh, read) => {
      if (read === 2) fresh.items.get('issue-1')!.released = { text: 'late evidence' };
    });
    reconcile({
      api,
      repository,
      dryRun: false,
      map,
      git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
    });
    expect(writes(api)).toHaveLength(2);
    expect(writes(api)[1][0]).toContain('clearProjectV2ItemFieldValue');
    expect(writes(api)[1][0]).not.toContain('fieldId:"status"');
    expect(p.items.get('issue-1')!.released!.text).toBe('');
  });

  it.each(['reopened', 'missing', 'epic'])(
    'refuses a %s issue before correction writes',
    (change) => {
      const p = project();
      const api = stateApi(p, [], new Map(), (fresh, read) => {
        if (read !== 2) return;
        const row = fresh.items.get('issue-1')!;
        if (change === 'reopened') row.content.state = 'OPEN';
        else if (change === 'missing') fresh.items.delete('issue-1');
        else row.content.labels.nodes.push({ name: 'EPIC' });
      });
      expect(() =>
        reconcile({
          api,
          repository,
          dryRun: false,
          map,
          git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
        })
      ).toThrow(/during reconciliation: https:\/\/github.com\/pj-tmt\/tmt\/issues\/1/);
      expect(writes(api)).toHaveLength(1);
    }
  );

  it('refuses an issue reopened on the second readback without a third write attempt', () => {
    const p = project();
    const api = stateApi(p, [], new Map(), (fresh, read) => {
      if (read === 2) fresh.items.get('issue-1')!.status = { name: 'Merged' };
      if (read === 3) fresh.items.get('issue-1')!.content.state = 'OPEN';
    });
    expect(() =>
      reconcile({
        api,
        repository,
        dryRun: false,
        map,
        git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
      })
    ).toThrow('Issue reopened');
    expect(writes(api)).toHaveLength(2);
  });

  it('reserves the correction and second readback before retry writes', () => {
    const p = project();
    const budget = githubApi({ repository, appToken: 'test-token' });
    const reportReadbackMismatch = vi.fn();
    const api = stateApi(p, [], new Map(), (fresh, read) => {
      if (read !== 2) return;
      fresh.items.get('issue-1')!.status = { name: 'Merged' };
      budget.counts.graphql = LIMITS.graphql - LIMITS.pages;
    });
    api.reserve = vi.fn(budget.reserve);
    expect(() =>
      reconcile({
        api,
        repository,
        dryRun: false,
        map,
        reportReadbackMismatch,
        git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
      })
    ).toThrow('Insufficient GraphQL budget before writes');
    expect(api.reserve).toHaveBeenCalledTimes(2);
    expect(reportReadbackMismatch).toHaveBeenCalledTimes(1);
    expect(writes(api)).toHaveLength(1);
    expect(p.items.get('issue-1')!.status!.name).toBe('Merged');
  });

  it('does not read back or correct drift during a dry run', () => {
    const p = project();
    const onRead = vi.fn();
    const reportReadbackMismatch = vi.fn();
    const api = stateApi(p, [], new Map(), onRead);
    reconcile({
      api,
      repository,
      dryRun: true,
      map,
      reportReadbackMismatch,
      git: { validateTags() {}, paths: () => [], containingTags: () => new Set() },
    });
    expect(onRead).toHaveBeenCalledTimes(1);
    expect(reportReadbackMismatch).not.toHaveBeenCalled();
    expect(writes(api)).toHaveLength(0);
    expect(p.items.get('issue-1')!.status!.name).toBe('Merged');
  });

  it('counts real transport attempts, scopes credentials and stops without retries', () => {
    const spawn = vi.fn(() => ({
      status: 0,
      stdout: '{"data":{"rateLimit":{"cost":1,"remaining":4999}}}',
    })) as unknown as typeof spawnSync;
    const api = githubApi({
      repository,
      appToken: 'project-token',
      readToken: 'read-token',
      spawn,
    });
    for (let i = 0; i < LIMITS.graphql; i++) api.graphql('query{}');
    expect(() => api.reserve(1)).toThrow('before writes');
    expect(() => api.graphql('query{}')).toThrow('budget exceeded');
    expect(spawn).toHaveBeenCalledTimes(LIMITS.graphql);
    expect(api.counts.graphql).toBe(LIMITS.graphql);
    expect(api.points).toEqual({ cost: LIMITS.graphql, remaining: 4999 });
    expect(() => githubApi({ repository })).toThrow('RELEASE_APP_TOKEN');
    const denied = githubApi({
      repository,
      appToken: 'token',
      spawn: vi.fn(() => ({
        status: 1,
        stdout: '{"errors":[{"message":"denied"}]}',
      })) as unknown as typeof spawnSync,
    });
    expect(() => denied.graphql('query{}')).toThrow('denied');
    expect(denied.counts.graphql).toBe(1);
  });
  it('sums reported read costs, retains partial-error telemetry and reports it on failure', () => {
    const spawn = vi
      .fn()
      .mockReturnValueOnce({
        status: 0,
        stdout: JSON.stringify({ data: { rateLimit: { cost: 21, remaining: 4979 } } }),
      })
      .mockReturnValueOnce({
        status: 1,
        stdout: JSON.stringify({
          data: { rateLimit: { cost: 3, remaining: 4976 } },
          errors: [{ message: 'partial read failed' }],
        }),
      });
    const api = githubApi({
      repository,
      appToken: 'token',
      spawn: spawn as unknown as typeof spawnSync,
    });
    api.graphql('query{rateLimit{cost remaining}}');
    expect(() => api.graphql('query{rateLimit{cost remaining}}')).toThrow('partial read failed');
    expect(api.points).toEqual({ cost: 24, remaining: 4976 });
    expect(renderFailure(new Error('partial read failed'), api)).toContain(
      'GraphQL points: 24 (reported reads); last remaining: 4976.'
    );
    expect(renderFailure(new Error('before reads'), fakeApi())).toContain(
      'last remaining: unavailable'
    );
    const missing = githubApi({
      repository,
      appToken: 'token',
      spawn: vi.fn(() => ({ status: 0, stdout: '{"data":{}}' })) as unknown as typeof spawnSync,
    });
    expect(() => missing.graphql('query{}')).toThrow('omitted valid rate-limit');
  });

  it('escapes table cells and keeps full daily/main-only dispatch wiring', () => {
    const p = project([item(1, 'Merged', '<script>|x\ny')]);
    const row = planUpdates(
      new Map([['issue-1', { status: 'Done', text: '', prs: [], waiting: [] }]]),
      p
    );
    const summary = renderSummary({
      dryRun: true,
      releases: [],
      issues: 1,
      rows: row.rows,
      changed: row.changes,
      requests: { graphql: 2, rest: 1 },
      points: { cost: 7, remaining: 4993 },
    });
    expect(summary).toContain('&lt;script&gt;&#124;x<br>y');
    expect(summary).toContain('GraphQL points: 7 (reported reads); last remaining: 4993.');
    expect(summary).not.toContain('<script>');
    const workflow = readFileSync(
      new URL('../../../.github/workflows/project-release.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain("- cron: '23 4 * * *'");
    expect(workflow).not.toContain('workflow_run:');
    expect(workflow).toContain('default: true');
    expect(workflow).toContain('fetch-depth: 0');
    expect(workflow).toContain("if: github.ref == 'refs/heads/main'");
    expect(workflow).toContain('ref: main');
    expect(workflow).toContain('group: project-release-tracking');
    expect(workflow).not.toContain('contents: write');
    expect(workflow).toContain('permission-organization-projects: write');
  });
});

describe('published tag validation with an injected git', () => {
  type Call = { args: string[]; input?: string };
  /** Answers each cat-file batch from `batches`, so a later check can see a fetched tag. */
  function fakeGit(batches: string[][], fetchStatus = 0) {
    const calls: Call[] = [];
    const spawn = ((_: string, args: string[], options: { input?: string }) => {
      calls.push({ args, input: options.input });
      if (args[0] === 'rev-parse') return { status: 0, stdout: 'false\n' };
      if (args[0] === 'fetch') return { status: fetchStatus, stdout: '' };
      return { status: 0, stdout: `${batches.shift()!.join('\n')}\n` };
    }) as never;
    return { calls, evidence: gitEvidence({ cwd: '/unused', spawn }) };
  }
  const commit = `${'a'.repeat(40)} commit`;
  const fetches = (calls: Call[]) => calls.filter(({ args }) => args[0] === 'fetch');

  it('does not fetch when every tag resolves', () => {
    const { calls, evidence } = fakeGit([[commit, commit]]);
    evidence.validateTags(['v1', 'v2']);
    expect(fetches(calls)).toHaveLength(0);
  });
  it('fetches exactly the missing tags once, then revalidates every tag', () => {
    const { calls, evidence } = fakeGit([
      [commit, 'refs/tags/v2^{commit} missing', commit],
      [commit, commit, commit],
    ]);
    evidence.validateTags(['v1', 'v2', 'v3']);
    expect(fetches(calls).map(({ args }) => args)).toEqual([
      ['fetch', '--no-tags', '--quiet', 'origin', '+refs/tags/v2:refs/tags/v2'],
    ]);
    expect(calls.filter(({ args }) => args[0] === 'cat-file')).toHaveLength(2);
  });
  it('fails closed naming the tag that still does not resolve, without a second fetch', () => {
    const { calls, evidence } = fakeGit([
      [commit, 'refs/tags/v2^{commit} missing'],
      [commit, 'refs/tags/v2^{commit} missing'],
    ]);
    expect(() => evidence.validateTags(['v1', 'v2'])).toThrow(
      'Git evidence failed: published tag(s) do not resolve to a commit: v2'
    );
    expect(fetches(calls)).toHaveLength(1);
  });
  it('fails closed naming the tag when the fetch itself fails', () => {
    const { calls, evidence } = fakeGit([[commit, 'refs/tags/v2^{commit} missing']], 128);
    expect(() => evidence.validateTags(['v1', 'v2'])).toThrow(
      /commit: v2 \(Git evidence failed: git fetch/
    );
    expect(fetches(calls)).toHaveLength(1);
  });
  it('never passes an option-like or malformed tag name to git fetch', () => {
    for (const name of ['-force', 'v1:refs/heads/main', 'refs/../x', '']) {
      const { calls, evidence } = fakeGit([[`refs/tags/${name}^{commit} missing`]]);
      expect(() => evidence.validateTags([name])).toThrow('not fetched');
      expect(fetches(calls)).toHaveLength(0);
    }
  });
});
