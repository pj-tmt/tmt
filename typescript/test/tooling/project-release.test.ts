import { type spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import {
  applyUpdates,
  githubApi,
  LIMITS,
  noteReferences,
  planUpdates,
  publishedWindow,
  readProject,
  readReleases,
  reconcile,
  releaseIdentity,
  releaseIssues,
  resolveClosingIssues,
  type Api,
  type IssueEvidence,
  type Project,
  type Release,
} from '../../scripts/project-release.mjs';

const repository = 'pj-tmt/tmt';
const connection = (nodes: unknown[], cursor: string | null = null) => ({
  nodes,
  pageInfo: { hasNextPage: !!cursor, endCursor: cursor },
});
const issue = { id: 'issue-1', number: 851, url: 'https://github.com/pj-tmt/tmt/issues/851' };
const pr = (nodes = [issue], cursor: string | null = null) => ({
  __typename: 'PullRequest',
  number: 904,
  merged: true,
  closingIssuesReferences: connection(nodes, cursor),
});
const release = (
  tag = 'v5.0.0-alpha.1',
  body = '[#904](https://github.com/pj-tmt/tmt/issues/904)'
): Release => ({ tag_name: tag, body, draft: false, published_at: '2026-10-02T00:00:00Z' });
const evidence = (labels = ['tmt-cli 5.0.0-alpha.1']) =>
  new Map<string, IssueEvidence>([[issue.id, { ...issue, labels: new Set(labels) }]]);
const project = (text = '', status = 'Merged'): Project => ({
  projectId: 'project',
  statusId: 'status',
  releasedId: 'released',
  optionId: 'terminal',
  pages: 1,
  items: new Map([
    [
      issue.id,
      { id: 'item', content: { id: issue.id }, status: { name: status }, released: { text } },
    ],
  ]),
});
function fakeApi(
  rest: (path: string) => unknown = () => [],
  graphql: (query: string) => unknown = () => ({})
): Api {
  const counts = { rest: 0, graphql: 0 };
  return {
    counts,
    rest: vi.fn((path) => {
      counts.rest++;
      return rest(path);
    }),
    graphql: vi.fn((query) => {
      counts.graphql++;
      return graphql(query);
    }),
    reserve: vi.fn(),
  };
}
function projectResponse(text = '', status = 'Merged') {
  return {
    node: {
      fields: connection([
        { id: 'status', name: 'Status', options: [{ id: 'terminal', name: 'Released' }] },
        { id: 'released', name: 'Released in', dataType: 'TEXT' },
      ]),
      items: connection([
        { id: 'item', content: issue, status: { name: status }, released: { text } },
      ]),
    },
  };
}

describe('release PR and closing-issue resolution', () => {
  it('uses product-owned version parsing and ignores unrelated release lines', () => {
    expect(releaseIdentity('v5.0.0-alpha.3')?.label).toBe('tmt-cli 5.0.0-alpha.3');
    expect(releaseIdentity('tmt-squad-v0.1.0-alpha.7')?.label).toBe('tmt-squad 0.1.0-alpha.7');
    for (const tag of ['v4.2.1', 'v5.bad', 'other-v1.0.0'])
      expect(releaseIdentity(tag)).toBeUndefined();
  });
  it('deduplicates PR-style issue links, accepts pre-transfer notes, and excludes foreign/bare references', () => {
    expect(
      noteReferences(
        '[#4](https://github.com/pj-tmt/tmt/issues/4) https://github.com/wkh237/tmt/pull/4 https://github.com/pj-tmt/tmt/pull/9 #55 https://github.com/elsewhere/tmt/pull/8',
        repository
      )
    ).toEqual([4, 9]);
  });
  it('batches PRs and follows closing issue pages without interpreting ordinary issues as PRs', () => {
    const api = fakeApi(undefined, (query) =>
      query.includes('after:"next"')
        ? { repository: { p0: pr([{ ...issue, id: 'issue-2' }]) } }
        : {
            repository: {
              p0: pr([issue], 'next'),
              p1: { __typename: 'Issue' },
              p2: { ...pr(), merged: false },
            },
          }
    );
    const result = resolveClosingIssues(api, repository, [904, 904, 851, 77]);
    expect([...result.get(904)!.keys()]).toEqual(['issue-1', 'issue-2']);
    expect(result.get(851)).toBeNull();
    expect(result.get(77)).toBeNull();
    expect(api.graphql).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.graphql).mock.calls[0][0]).toContain('p2:issueOrPullRequest');
  });
  it('fails on unresolved references, missing cursors and exhausted pagination before any mutation', () => {
    expect(() =>
      resolveClosingIssues(
        fakeApi(undefined, () => ({ repository: { p0: null } })),
        repository,
        [1]
      )
    ).toThrow('could not be resolved');
    const api = fakeApi(undefined, () => ({ repository: { p0: pr([issue], 'again') } }));
    expect(() => resolveClosingIssues(api, repository, [1])).toThrow('pagination cap');
    expect(api.graphql).toHaveBeenCalledTimes(LIMITS.pages);
    expect(() =>
      resolveClosingIssues(
        api,
        repository,
        Array.from({ length: LIMITS.prs + 1 }, (_, i) => i)
      )
    ).toThrow('reference cap');
  });
  it('maps one issue closed by multiple PRs and released components to distinct append entries', () => {
    const api = fakeApi(undefined, () => ({ repository: { p0: pr() } }));
    const releases = [release(), release('tmt-squad-v0.1.0-alpha.1')];
    const result = releaseIssues(api, repository, releases, releases);
    expect(result.issues.size).toBe(1);
    expect([...result.issues.get(issue.id)!.labels]).toEqual([
      'tmt-cli 5.0.0-alpha.1',
      'tmt-squad 0.1.0-alpha.1',
    ]);
    expect(api.graphql).toHaveBeenCalledTimes(1);
  });
  it('falls back to the same-component compare range and commit-associated merged PRs', () => {
    const api = fakeApi(
      (path) =>
        path.startsWith('compare/')
          ? { status: 'ahead', total_commits: 1, commits: [{ sha: 'abc' }] }
          : [
              { number: 904, merged_at: 'now', base: { repo: { full_name: repository } } },
              { number: 9, merged_at: null },
            ],
      () => ({ repository: { p0: pr() } })
    );
    const target = release('v5.0.0-alpha.2', 'No PR links');
    const result = releaseIssues(
      api,
      repository,
      [target],
      [release(), target, release('tmt-squad-v0.1.0-alpha.9')]
    );
    expect(result.sources).toEqual([{ tag: target.tag_name, method: 'compare', prs: [904] }]);
    expect(api.rest).toHaveBeenCalledWith(
      'compare/v5.0.0-alpha.1...v5.0.0-alpha.2?per_page=100&page=1'
    );
    expect(result.issues.has(issue.id)).toBe(true);
  });
  it('fails visibly instead of silently inventing an initial-release PR range', () => {
    const r = release('v5.0.0-alpha.1', 'No references');
    expect(() => releaseIssues(fakeApi(), repository, [r], [r])).toThrow(
      'No PR notes or previous release'
    );
  });
});

describe('bounded release reconciliation', () => {
  it('selects only published supported releases and rejects an event outside the window', () => {
    const rows = Array.from({ length: 11 }, (_, i) => ({
      ...release(`v5.0.0-alpha.${i + 1}`),
      published_at: `2026-10-02T00:${String(i).padStart(2, '0')}:00Z`,
    }));
    expect(publishedWindow(rows, 'release', { release: rows[10] })).toHaveLength(10);
    expect(() => publishedWindow(rows, 'release', { release: rows[0] })).toThrow('outside');
    expect(publishedWindow(rows, 'workflow_dispatch', {}, rows[0].tag_name)).toEqual([rows[0]]);
    expect(() =>
      publishedWindow([{ ...rows[0], draft: true }], 'workflow_dispatch', {}, rows[0].tag_name)
    ).toThrow('published tag');
    expect(() =>
      publishedWindow(rows, 'workflow_run', {
        workflow_run: {
          name: 'Native release artifacts',
          head_branch: 'main',
          event: 'workflow_dispatch',
          created_at: '2026-10-01T00:00:00Z',
        },
      })
    ).toThrow('window');
    expect(() =>
      publishedWindow(rows, 'workflow_run', { workflow_run: { name: 'Other' } })
    ).toThrow('provenance');
  });
  it('bounds release listing without silently truncating history', () => {
    const api = fakeApi(() => Array.from({ length: 100 }, () => release()));
    expect(() => readReleases(api)).toThrow('pagination cap');
    expect(api.rest).toHaveBeenCalledTimes(LIMITS.pages);
  });
  it('reads Project items across pages and validates the terminal fields', () => {
    const api = fakeApi(undefined, (query) => {
      const response = projectResponse();
      if (!query.includes('after:"second"'))
        response.node.items = connection([], 'second') as typeof response.node.items;
      return response;
    });
    expect(readProject(api).items.get(issue.id)?.id).toBe('item');
    expect(api.graphql).toHaveBeenCalledTimes(2);
    expect(() => readProject(fakeApi(undefined, () => ({ node: null })))).toThrow(
      'Missing project'
    );
  });
  it('preserves existing text, deduplicates exact labels, and never moves Released backwards', () => {
    const old = 'Hand-entered history\r\ntmt-cli 5.0.0-alpha.1';
    const p = project(old, 'Released');
    const plan = planUpdates(evidence(['tmt-cli 5.0.0-alpha.1', 'tmt-squad 0.1.0-alpha.1']), p);
    expect(plan.changes).toEqual([
      {
        itemId: 'item',
        issue: issue.url,
        text: `${old}\ntmt-squad 0.1.0-alpha.1`,
        writeText: true,
        writeStatus: false,
      },
    ]);
    p.items.get(issue.id)!.released!.text = plan.changes[0].text;
    expect(
      planUpdates(evidence(['tmt-cli 5.0.0-alpha.1', 'tmt-squad 0.1.0-alpha.1']), p).changes
    ).toEqual([]);
    expect(planUpdates(evidence(), { ...p, items: new Map() }).skipped).toEqual([issue.url]);
  });
  it('writes text before terminal status, and retries a partial update without duplicate text', () => {
    const p = project();
    const api = fakeApi();
    applyUpdates(api, p, planUpdates(evidence(), p), false);
    expect(vi.mocked(api.graphql).mock.calls[0][0]).toContain('value:{text:');
    expect(vi.mocked(api.graphql).mock.calls[1][0]).toContain(
      'value:{singleSelectOptionId:"terminal"}'
    );
    const retry = planUpdates(evidence(), project('tmt-cli 5.0.0-alpha.1'));
    expect(retry.changes[0]).toMatchObject({ writeText: false, writeStatus: true });
    const failed = fakeApi(undefined, () => {
      throw new Error('partial failure');
    });
    expect(() => applyUpdates(failed, p, planUpdates(evidence(), p), false)).toThrow(
      'partial failure'
    );
    expect(failed.graphql).toHaveBeenCalledTimes(1);
  });
  it('performs a complete dry run with no mutations and a steady-state run with no writes', () => {
    const run = (existing: boolean, dryRun: boolean) => {
      const api = fakeApi(
        () => release(),
        (query) =>
          query.includes('issueOrPullRequest')
            ? { repository: { p0: pr() } }
            : projectResponse(
                existing ? 'tmt-cli 5.0.0-alpha.1' : '',
                existing ? 'Released' : 'Merged'
              )
      );
      const result = reconcile({
        api,
        repository,
        eventName: 'workflow_dispatch',
        event: {},
        tag: release().tag_name,
        dryRun,
      });
      expect(
        vi.mocked(api.graphql).mock.calls.every(([query]) => !query.startsWith('mutation'))
      ).toBe(true);
      return result;
    };
    expect(run(false, true).changed).toHaveLength(1);
    expect(run(true, false).changed).toEqual([]);
    expect(run(true, false).requests).toEqual({ rest: 1, graphql: 2 });
  });
  it('verifies a completed catch-up and makes a repeated run a mutation-free no-op', () => {
    let text = '',
      status = 'Merged';
    const api = fakeApi(
      () => release(),
      (query) => {
        if (query.startsWith('mutation')) {
          if (query.includes('value:{text:')) text = 'tmt-cli 5.0.0-alpha.1';
          if (query.includes('value:{singleSelectOptionId:')) status = 'Released';
          return {};
        }
        return query.includes('issueOrPullRequest')
          ? { repository: { p0: pr() } }
          : projectResponse(text, status);
      }
    );
    const options = {
      api,
      repository,
      eventName: 'workflow_dispatch',
      event: {},
      tag: release().tag_name,
      dryRun: false,
    };
    expect(reconcile(options).changed).toHaveLength(1);
    expect(api.counts).toEqual({ rest: 1, graphql: 5 });
    const writes = vi
      .mocked(api.graphql)
      .mock.calls.filter(([query]) => query.startsWith('mutation')).length;
    expect(reconcile(options).changed).toEqual([]);
    expect(
      vi.mocked(api.graphql).mock.calls.filter(([query]) => query.startsWith('mutation'))
    ).toHaveLength(writes);
    expect({ text, status }).toEqual({ text: 'tmt-cli 5.0.0-alpha.1', status: 'Released' });
  });

  it('rejects an insufficient write budget before mutating', () => {
    const p = project();
    const api = fakeApi();
    api.reserve = () => {
      throw new Error('budget');
    };
    expect(() => applyUpdates(api, p, planUpdates(evidence(), p), false)).toThrow('budget');
    expect(api.graphql).not.toHaveBeenCalled();
  });
  it('bounds real transport calls and reserves mutation capacity without retries', () => {
    const spawn = vi.fn(() => ({
      status: 0,
      stdout: '{"data":{}}',
    })) as unknown as typeof spawnSync;
    const api = githubApi({
      repository,
      projectToken: 'test-project',
      readToken: 'test-read',
      spawn,
    });
    for (let i = 0; i < LIMITS.graphql; i++) api.graphql('query{viewer{login}}');
    expect(() => api.reserve(1)).toThrow('before writes');
    expect(() => api.graphql('query{viewer{login}}')).toThrow('budget exceeded');
    expect(spawn).toHaveBeenCalledTimes(LIMITS.graphql);
    const denied = githubApi({
      repository,
      projectToken: 'test-project',
      spawn: vi.fn(() => ({
        status: 1,
        stdout: '{"errors":[{"message":"missing project access"}]}',
      })) as unknown as typeof spawnSync,
    });
    expect(() => denied.graphql('query{viewer{login}}')).toThrow('missing project access');
  });

  it('batches catch-up field mutations and refuses unknown future statuses', () => {
    const p = project();
    const rows = Array.from({ length: 26 }, (_, i) => ({
      itemId: `item-${i}`,
      issue: `issue-${i}`,
      text: 'tmt-cli 5.0.0-alpha.1',
      writeText: true,
      writeStatus: true,
    }));
    const api = fakeApi();
    applyUpdates(api, p, { changes: rows, skipped: [] }, false);
    expect(api.graphql).toHaveBeenCalledTimes(4);
    expect(api.reserve).toHaveBeenCalledWith(5);
    expect(
      vi.mocked(api.graphql).mock.calls[0][0].match(/updateProjectV2ItemFieldValue/g)
    ).toHaveLength(25);
    expect(() => planUpdates(evidence(), project('', 'Archived'))).toThrow('Unknown status');
  });

  it('fails before reads when PROJECT_TOKEN is missing', () => {
    expect(() => githubApi({ repository })).toThrow('PROJECT_TOKEN is missing');
  });
  it('freezes trusted event wiring, dry-run default and project-wide serialization', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/project-release.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain('types: [published]');
    expect(workflow).toContain('workflows: [Native release artifacts]');
    expect(workflow).toContain('default: true');
    expect(workflow).toContain('ref: main');
    expect(workflow).toContain('group: project-4-release-tracking');
    expect(workflow).toContain('PROJECT_TOKEN: ${{ secrets.PROJECT_TOKEN }}');
    expect(workflow).not.toContain('contents: write');
  });
});
