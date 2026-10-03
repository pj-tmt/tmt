import { describe, expect, it, vi } from 'vite-plus/test';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import {
  createStallClient,
  detectReleaseStalls,
  monitorReleaseStalls,
  planReleaseCommits,
  reconcileStallIssue,
  type StallClient,
} from '../../scripts/release-stall.mjs';
import type { DraftEvidence } from '../../scripts/release-pr-safety.mjs';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';

const repository = 'fixture/fixture';
const now = Date.parse('2026-10-03T12:00:00Z');
const date = (offset: number) => new Date(now + offset).toISOString();
const cli = 'release-please--branches--main--components--tmt-cli';
const squad = 'release-please--branches--main--components--tmt-squad';
const sha = (n: number) => n.toString(16).padStart(40, 'a');
const components = parseComponentMap(
  readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
).components;
const manifest = {
  '.': '5.0.0-alpha.8',
  'extensions/tmt-squad': '0.1.0-alpha.8',
  'rust/crates/tmt-driver-herdr': '0.1.0-alpha.0',
};
function fixture() {
  const rows: Record<string, ReturnType<StallClient['list']>> = {
    releases: [],
    'issues?state=open': [],
    'pulls?state=open&base=main': [],
    'issues/42/comments': [],
  };
  const records: Record<string, ReturnType<StallClient['get']>> = {};
  const write = vi.fn<StallClient['write']>().mockReturnValue({ number: 42, state: 'open' });
  const search = vi.fn<StallClient['search']>().mockReturnValue(undefined);
  const client: StallClient = {
    repository,
    write,
    search,
    get: vi.fn((path) => {
      if (!(path in records)) throw new Error(`Unconfigured REST ${path}`);
      return records[path];
    }),
    list: vi.fn((path) => {
      if (!(path in rows)) throw new Error(`Unconfigured REST list ${path}`);
      return rows[path];
    }),
    git: () => {
      throw new Error('Unexpected git acquisition');
    },
  };
  const plan = vi.fn<typeof planReleaseCommits>().mockResolvedValue([]);
  return {
    rows,
    records,
    write,
    search,
    client,
    options: {
      client,
      manifest,
      components,
      heldPaths: [] as string[],
      drafts: [] as DraftEvidence[],
      queueSkipped: false,
      live: true,
      now,
      plan,
    },
    plan,
  };
}
const draft = (tag = 'tmt-squad-v0.1.0-alpha.8', age = 31 * 60_000) => ({
  path: tag.startsWith('tmt-squad-') ? 'extensions/tmt-squad' : '.',
  id: 7,
  tag_name: tag,
  draft: true,
  created_at: date(-age),
});
const pull = (branch = cli) => ({
  number: 21,
  head: { ref: branch, sha: sha(1), repo: { full_name: repository } },
  base: { ref: 'main' },
});

describe('post-publication smoke conclusions', () => {
  it.each([
    ['public install blocked by GitHub API rate limit', 'public smoke infrastructure blocked'],
    ['failed its post-publication checks', 'published release checks failed'],
  ])('distinguishes %s without changing publication', async (suffix, message) => {
    const f = fixture();
    f.rows.releases = [{ tag_name: 'v5.0.0-alpha.8', draft: false }];
    f.rows['issues?state=open'] = [{ number: 99, title: `Release v5.0.0-alpha.8 ${suffix}` }];
    const result = await detectReleaseStalls(f.options);
    expect(result.findings).toHaveLength(1);
    expect(result.findings[0].message).toContain(message);
    expect(f.write).not.toHaveBeenCalled();
  });

  it('prioritizes a real failure when both reporter conclusions remain open', async () => {
    const f = fixture();
    f.rows.releases = [{ tag_name: 'v5.0.0-alpha.8', draft: false }];
    f.rows['issues?state=open'] = [
      {
        number: 99,
        title: 'Release v5.0.0-alpha.8 public install blocked by GitHub API rate limit',
      },
      { number: 100, title: 'Release v5.0.0-alpha.8 failed its post-publication checks' },
    ];
    const result = await detectReleaseStalls(f.options);
    expect(result.findings).toHaveLength(1);
    expect(result.findings[0].message).toContain('published release checks failed');
  });

  it('keeps a proven draft stall when post-publication issue discovery is unavailable', async () => {
    const f = fixture();
    delete f.rows['issues?state=open'];
    f.options.drafts = [draft()];
    f.options.heldPaths = ['extensions/tmt-squad'];
    const result = await detectReleaseStalls(f.options);
    expect(result.findings).toHaveLength(1);
    expect(result.warning).toContain('Post-publication smoke evidence unavailable');
    const report = await monitorReleaseStalls(f.options);
    expect(report.healthy).toBe(false);
    expect(f.write).toHaveBeenCalled();
  });

  it('ignores old releases and pull requests that happen to use the reporter title', async () => {
    const f = fixture();
    f.rows.releases = [{ tag_name: 'v5.0.0-alpha.8', draft: false }];
    f.rows['issues?state=open'] = [
      { number: 99, title: 'Release v5.0.0-alpha.7 failed its post-publication checks' },
      {
        number: 100,
        title: 'Release v5.0.0-alpha.8 failed its post-publication checks',
        pull_request: {},
      },
    ];
    expect((await detectReleaseStalls(f.options)).findings).toEqual([]);
  });
});

describe('release stall thresholds and component evidence', () => {
  it.each(['tagless', 'queue'])('warns for an old draft held by %s', async (guard) => {
    const f = fixture();
    f.options.drafts = [draft()];
    f.options.heldPaths = guard === 'tagless' ? ['extensions/tmt-squad'] : [];
    f.options.queueSkipped = guard === 'queue';
    const { findings } = await detectReleaseStalls(f.options);
    expect(findings).toHaveLength(1);
    expect(findings[0].message).toContain('extensions/tmt-squad');
    expect(findings[0].message).toContain(guard === 'queue' ? 'queue skip' : 'TAGLESS_DRAFT');
    f.options.drafts = [draft(undefined, 30 * 60_000)];
    expect((await detectReleaseStalls(f.options)).findings).toEqual([]);
  });
  it('reports a held standalone driver draft under its own component path', async () => {
    const f = fixture();
    const path = 'rust/crates/tmt-driver-herdr';
    f.options.heldPaths = [path];
    f.options.drafts = [{ ...draft(), path, tag_name: 'tmt-driver-herdr-v0.1.0-alpha.0' }];
    const { findings } = await detectReleaseStalls(f.options);
    expect(findings).toHaveLength(1);
    expect(findings[0].message).toContain(path);
  });

  it('does not warn for an unheld or published component draft and keeps occurrence stable as it ages', async () => {
    const f = fixture();
    f.options.heldPaths = ['.'];
    f.options.drafts = [draft(), { ...draft('v5.0.0-alpha.8', 29 * 60_000), id: 8 }];
    expect((await detectReleaseStalls(f.options)).findings).toEqual([]);
    f.options.heldPaths = ['extensions/tmt-squad'];
    const {
      findings: [first],
    } = await detectReleaseStalls(f.options);
    const {
      findings: [later],
    } = await detectReleaseStalls({ ...f.options, now: now + 3600_000 });
    expect(later.key).toBe(first.key);
    f.rows.releases = [{ ...draft(), draft: false }];
    expect((await detectReleaseStalls(f.options)).findings).toEqual([]);
  });
  it('checks stale heads against the same component and actual missing commit ancestry', async () => {
    const f = fixture();
    f.rows['pulls?state=open&base=main'] = [pull(), pull(squad)];
    f.plan.mockResolvedValue([{ branch: cli, sha: sha(2), time: now }]);
    f.records[`commits/${sha(1)}`] = {
      sha: sha(1),
      commit: { committer: { date: date(-3600_001) } },
    };
    f.records[`compare/${sha(2)}...${sha(1)}`] = { status: 'diverged' };
    const { findings } = await detectReleaseStalls(f.options);
    expect(findings).toHaveLength(1);
    expect(findings[0].message).toContain('PR #21');
    expect(findings[0].message).not.toContain(squad);
    f.records[`commits/${sha(1)}`].commit!.committer!.date = date(-3600_000);
    expect((await detectReleaseStalls(f.options)).findings).toEqual([]);
    f.records[`commits/${sha(1)}`].commit!.committer!.date = date(-3600_001);
    for (const status of ['ahead', 'identical']) {
      f.records[`compare/${sha(2)}...${sha(1)}`] = { status };
      expect((await detectReleaseStalls(f.options)).findings).toEqual([]);
    }
  });
  it('rejects absent timestamps and incomplete guard/ancestry evidence instead of declaring healthy', async () => {
    const f = fixture();
    f.options.drafts = [{ ...draft(), created_at: 'bad' }];
    f.options.queueSkipped = true;
    await expect(detectReleaseStalls(f.options)).rejects.toThrow('timestamp');
    await expect(detectReleaseStalls({ ...f.options, heldPaths: ['unknown'] })).rejects.toThrow(
      'guard'
    );
    f.rows.releases = [];
    f.options.drafts = [];
    f.rows['pulls?state=open&base=main'] = [pull()];
    f.plan.mockResolvedValue([{ branch: cli, sha: sha(2), time: now }]);
    f.records[`commits/${sha(1)}`] = {
      sha: sha(1),
      commit: { committer: { date: date(-7200_000) } },
    };
    f.records[`compare/${sha(2)}...${sha(1)}`] = { status: 'unknown' };
    expect((await detectReleaseStalls(f.options)).warning).toContain('ancestry');
  });
});

describe('one durable Release stalled issue', () => {
  const findings = [{ key: 'a'.repeat(64), message: 'Squad draft held' }];
  it('creates once, comments on new occurrences, and deduplicates retried findings', () => {
    const f = fixture();
    reconcileStallIssue(f.client, findings);
    expect(f.write.mock.calls.map(([path, method]) => [path, method])).toEqual([
      ['issues', 'POST'],
      ['issues/42/comments', 'POST'],
    ]);
    expect(f.write.mock.calls[0][2].title).toBe('Release stalled');
    const comment = f.write.mock.calls[1][2].body;
    f.rows['issues/42/comments'] = [{ body: comment }];
    f.search.mockReturnValue({ number: 42, state: 'open', body: f.write.mock.calls[0][2].body });
    f.write.mockClear();
    reconcileStallIssue(f.client, findings);
    expect(f.write).not.toHaveBeenCalled();
    reconcileStallIssue(f.client, [
      ...findings,
      { key: 'b'.repeat(64), message: 'CLI head stale' },
    ]);
    expect(f.write.mock.calls.at(-1)?.[2].body).toContain('CLI head stale');
    expect(f.write.mock.calls.at(-1)?.[2].body).not.toContain('Squad draft held');
  });
  it('closes healthy and reopens the same issue for a new occurrence', () => {
    const f = fixture();
    f.search.mockReturnValue({ number: 42, state: 'open' });
    reconcileStallIssue(f.client, []);
    expect(f.write.mock.calls[0][2].state).toBe('closed');
    f.search.mockReturnValue({ number: 42, state: 'closed' });
    f.write.mockClear();
    reconcileStallIssue(f.client, findings);
    expect(f.write.mock.calls[0]).toEqual([
      'issues/42',
      'PATCH',
      expect.objectContaining({ state: 'open' }),
    ]);
  });
  it('always reports warnings, never fails the caller, and never closes on uncertain discovery', async () => {
    const f = fixture();
    f.options.drafts = [draft()];
    f.options.queueSkipped = true;
    const summarize = vi.fn();
    const warn = vi.fn();
    f.write.mockImplementation(() => {
      throw new Error('issues denied');
    });
    const result = await monitorReleaseStalls(f.options, { summarize, warn });
    expect(result.healthy).toBe(false);
    expect(result.warning).toContain('issues denied');
    expect(summarize.mock.calls[0][0]).toContain('Release stalled');
    expect(summarize.mock.calls[1][0]).toContain('issues denied');
    const g = fixture();
    g.client.list = () => {
      throw new Error('read failed');
    };
    g.search.mockReturnValue({ number: 42, state: 'open' });
    expect((await monitorReleaseStalls(g.options, { summarize, warn })).healthy).toBe(false);
    expect(g.write).not.toHaveBeenCalled();
    await expect(
      monitorReleaseStalls(g.options, {
        summarize: () => {
          throw new Error('summary failed');
        },
        warn: () => {
          throw new Error('stdout failed');
        },
      })
    ).resolves.toHaveProperty('healthy', false);
  });
  it('dry runs summarize without creating, closing or editing issues', async () => {
    const f = fixture();
    f.options.live = false;
    f.options.drafts = [draft()];
    f.options.queueSkipped = true;
    const summarize = vi.fn();
    await monitorReleaseStalls(f.options, { summarize });
    expect(summarize.mock.calls[0][0]).toContain('Release stalled');
    expect(f.search).not.toHaveBeenCalled();
    expect(f.write).not.toHaveBeenCalled();
  });
});

describe('bounded REST transport', () => {
  it('uses the workflow token for reads/writes and bounds requests and pagination', () => {
    const execute = vi.fn().mockReturnValue('[]');
    const client = createStallClient({ repository, token: 'workflow' }, execute);
    client.list('releases');
    execute.mockReturnValue('{}');
    client.write('issues/42', 'PATCH', { state: 'closed' });
    expect(execute.mock.calls[0][2].env.GH_TOKEN).toBe('workflow');
    expect(execute.mock.calls[1][2].env.GH_TOKEN).toBe('workflow');
    expect(execute.mock.calls[1][1]).toContain('state=closed');
    for (let index = 0; index < 58; index++) client.get('fixture');
    expect(() => client.get('fixture')).toThrow('budget');
    execute.mockReturnValue(JSON.stringify(Array.from({ length: 100 }, () => ({}))));
    const paged = createStallClient({ repository, token: 'workflow' }, execute);
    expect(() => paged.list('releases')).toThrow('pagination');
  });
  it('refuses incomplete or ambiguous exact-title issue search', () => {
    const execute = vi
      .fn()
      .mockReturnValue(JSON.stringify({ incomplete_results: true, total_count: 0, items: [] }));
    const client = createStallClient({ repository, token: 'workflow' }, execute);
    expect(() => client.search()).toThrow('Incomplete');
    execute.mockReturnValue(
      JSON.stringify({
        incomplete_results: false,
        total_count: 2,
        items: [{ title: 'Release stalled' }, { title: 'Release stalled' }],
      })
    );
    expect(() => client.search()).toThrow('Multiple');
    expect(execute.mock.calls[0][1].join(' ')).not.toContain('graphql');
  });
});

it('uses real pinned planning, excludes unrelated paths and old history, with no SCM network requests', async () => {
  const config = readFileSync(
    new URL('../../../release-please-config.json', import.meta.url),
    'utf8'
  );
  const changes = [
    {
      sha: sha(10),
      time: now + 1000,
      message: 'fix: driver-only correction',
      files: ['rust/crates/tmt-driver-herdr/src/lib.rs'],
    },
    {
      sha: sha(9),
      time: now,
      message: 'fix: latest private TUI change',
      files: ['rust/crates/tmt-tui/src/lib.rs'],
    },
    {
      sha: sha(8),
      time: now - 1000,
      message: 'docs: only site prose',
      files: ['site/src/content/docs/index.mdx'],
    },
    {
      sha: sha(7),
      time: now - 2000,
      message: 'fix: cli behavior',
      files: ['rust/crates/tmt-core/src/lib.rs'],
    },
    {
      sha: sha(6),
      time: now - 3000,
      message: 'chore: squad release',
      files: ['extensions/tmt-squad/Cargo.toml'],
    },
    { sha: sha(5), time: now - 4000, message: 'chore: cli release', files: ['rust/Cargo.toml'] },
    {
      sha: components.find((component) => component.name === 'driver-herdr')!.bootstrapSha!,
      time: now - 4500,
      message: 'chore: bootstrap',
      files: [],
    },
    {
      sha: sha(4),
      time: now - 5000,
      message: 'fix: old history',
      files: ['rust/crates/tmt-core/src/lib.rs'],
    },
  ];
  const f = fixture();
  f.client.git = (args) => {
    if (args[0] === 'log')
      return changes.map((c) => `${c.sha}\0${c.time / 1000}\0${c.message}\0\n`).join('');
    if (args[0] === 'for-each-ref')
      return `v5.0.0-alpha.8\0${sha(5)}\0\ntmt-squad-v0.1.0-alpha.8\0${sha(6)}\0`;
    if (args[0] === 'diff-tree')
      return changes.find((c) => c.sha === args.at(-1))!.files.join('\n');
    if (args[0] === 'show')
      return args[1].endsWith('release-please-config.json') ? config : JSON.stringify(manifest);
    throw new Error(`Unexpected git ${args.join(' ')}`);
  };
  const releases = [
    { tag_name: 'v5.0.0-alpha.8', draft: false },
    { tag_name: 'tmt-squad-v0.1.0-alpha.8', draft: false },
  ];
  const planned = await planReleaseCommits({ client: f.client, components, releases });
  expect(planned).toEqual(
    expect.arrayContaining([
      { branch: cli, sha: sha(7), time: now - 2000 },
      { branch: squad, sha: sha(9), time: now },
      {
        branch: 'release-please--branches--main--components--tmt-driver-herdr',
        sha: sha(10),
        time: now + 1000,
      },
    ])
  );
  expect(planned).toHaveLength(3);
  expect(f.client.get).not.toHaveBeenCalled();
});

it('isolates monitoring timeout/errors and captures queue skips in release workflow', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/release.yml', import.meta.url),
    'utf8'
  );
  const step = workflow.slice(
    workflow.indexOf('  release-stall:\n'),
    workflow.indexOf('  dispatch:\n')
  );
  expect(step).toContain('continue-on-error: true');
  expect(step).toContain('timeout-minutes: 2');
  expect(step).toContain('needs.release-please.outputs.queue_skipped');
  expect(step).toContain('GITHUB_TOKEN: ${{ github.token }}');
  expect(step).toContain('DRAFT_EVIDENCE: ${{ needs.release-please.outputs.drafts }}');
  expect(step).toContain('needs: release-please');
  expect(step).toContain("if: always() && needs.release-please.outputs.live != ''");
  expect(step).toMatch(/permissions:\n {6}contents: read\n {6}issues: write\n/);
  expect(step).not.toMatch(
    /RELEASE_TOKEN|ISSUE_TOKEN|steps\.app|secrets\.|environment:|permission-contents: write|pull-requests:/
  );
  const releaseJob = workflow.slice(
    workflow.indexOf('  release-please:\n'),
    workflow.indexOf('  release-stall:\n')
  );
  expect(releaseJob).not.toContain('issues: write');
  expect(releaseJob).toContain('drafts: ${{ steps.draft.outputs.drafts }}');
  expect(releaseJob.split('    outputs:\n')[1].split('    steps:')[0]).not.toMatch(/token|secret/i);
  expect(workflow).toContain('fetch-depth: 0');
  expect(workflow).toContain('queue_skipped=true');
});

it('reports a known old draft even when candidate planning is unavailable, without closing on uncertainty', async () => {
  const f = fixture();
  f.options.drafts = [draft()];
  f.options.queueSkipped = true;
  f.rows['pulls?state=open&base=main'] = [pull()];
  f.plan.mockRejectedValue(new Error('bounded history unavailable'));
  const summarize = vi.fn();
  const result = await monitorReleaseStalls(f.options, { summarize });
  expect(result.findings).toHaveLength(1);
  expect(result.warning).toContain('bounded history');
  expect(f.write.mock.calls[0][0]).toBe('issues');
  expect(f.write.mock.calls[0][2].title).toBe('Release stalled');
  expect(summarize.mock.calls.flat().join(' ')).toContain('bounded history unavailable');
  const g = fixture();
  g.rows['pulls?state=open&base=main'] = [pull()];
  g.plan.mockRejectedValue(new Error('bounded history unavailable'));
  g.search.mockReturnValue({ number: 42, state: 'open' });
  expect((await monitorReleaseStalls(g.options, { summarize })).healthy).toBe(false);
  expect(g.write).not.toHaveBeenCalled();
});

it('bounds the total deadline before invoking another command', () => {
  const clock = vi.spyOn(Date, 'now').mockReturnValue(now);
  try {
    const execute = vi.fn().mockReturnValue('{}');
    const client = createStallClient({ repository, token: 'workflow' }, execute);
    clock.mockReturnValue(now + 90_001);
    expect(() => client.get('releases')).toThrow('deadline');
    expect(execute).not.toHaveBeenCalled();
  } finally {
    clock.mockRestore();
  }
});

it('the actual CLI exits zero and writes a warning on startup failure, even if summary writing fails', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-stall-cli-'));
  try {
    const summary = path.join(directory, 'summary');
    const command = new URL('../../scripts/release-stall.mjs', import.meta.url);
    for (const target of [summary, directory]) {
      const result = spawnSync(process.execPath, [command.pathname], {
        env: { ...process.env, QUEUE_SKIPPED: '', GITHUB_STEP_SUMMARY: target },
        encoding: 'utf8',
        timeout: 5000,
      });
      expect(result.status).toBe(0);
      expect(result.stderr).toContain('Missing queue guard result');
    }
    expect(readFileSync(summary, 'utf8')).toContain('Release stall monitor warning');
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

it('uses github.token for search and a recent REST page prevents duplicates during search-index lag', () => {
  const issue = { number: 42, state: 'open', title: 'Release stalled' };
  const execute = vi
    .fn()
    .mockImplementation((_command, args) =>
      JSON.stringify(
        args[1].startsWith('search/')
          ? { incomplete_results: false, total_count: 0, items: [] }
          : [issue]
      )
    );
  const client = createStallClient({ repository, token: 'workflow' }, execute);
  expect(client.search()).toEqual(issue);
  expect(execute).toHaveBeenCalledTimes(2);
  for (const call of execute.mock.calls) expect(call[2].env.GH_TOKEN).toBe('workflow');
});

it('opens the issue from an old tagless Squad alpha.12 snapshot even though github.token cannot list the draft', async () => {
  const f = fixture();
  f.options.manifest = { ...manifest, 'extensions/tmt-squad': '0.1.0-alpha.12' };
  f.options.heldPaths = ['extensions/tmt-squad'];
  f.options.drafts = [draft('tmt-squad-v0.1.0-alpha.12')];
  expect(f.rows.releases).toEqual([]);
  const summarize = vi.fn();
  const result = await monitorReleaseStalls(f.options, { summarize });
  expect(result.findings).toHaveLength(1);
  expect(summarize.mock.calls[0][0]).toContain('tmt-squad-v0.1.0-alpha.12');
  expect(f.write.mock.calls[0][2].title).toBe('Release stalled');
});

it('a published release supersedes the guard snapshot, while absent or malformed draft transport never closes', async () => {
  const f = fixture();
  f.options.heldPaths = ['extensions/tmt-squad'];
  f.options.drafts = [draft()];
  f.rows.releases = [{ tag_name: draft().tag_name, draft: false }];
  f.search.mockReturnValue({ number: 42, state: 'open' });
  expect((await monitorReleaseStalls(f.options)).healthy).toBe(true);
  expect(f.write.mock.calls[0][2].state).toBe('closed');
  for (const drafts of [
    null,
    [],
    [{ ...draft(), path: 'unknown' }],
    [{ ...draft(), tag_name: 'wrong' }],
    [{ ...draft(), id: undefined }],
  ]) {
    f.write.mockClear();
    const result = await monitorReleaseStalls(
      { ...f.options, drafts: drafts as DraftEvidence[] },
      { warn: () => {} }
    );
    expect(result.healthy).toBe(false);
    expect(f.write).not.toHaveBeenCalled();
  }
});
