import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  BUNDLE_ASSET,
  FAILURE_ASSET,
  planReleaseBuilds,
  releasesFrom,
  renderPlanSummary,
  type ReleaseObject,
} from '../../scripts/plan-release-builds.mjs';

const script = fileURLToPath(new URL('../../scripts/plan-release-builds.mjs', import.meta.url));

const sha = (digit: string) => digit.repeat(40);

function draft(
  tag: string,
  created: string,
  assets: string[] = [],
  overrides: Partial<ReleaseObject> = {}
): ReleaseObject {
  return {
    draft: true,
    tag_name: tag,
    target_commitish: sha(created.slice(-1)),
    created_at: `2026-09-30T0${created}:00:00Z`,
    assets: assets.map((name) => ({ name })),
    ...overrides,
  };
}

const tags = (plan: { builds: readonly { tag: string }[] }) => plan.builds.map(({ tag }) => tag);

describe('release build plan', () => {
  it('plans every draft of the product that has no bundle, oldest first, whatever order GitHub lists them', () => {
    const plan = planReleaseBuilds({
      product: 'cli',
      releases: [
        draft('v5.0.0-alpha.11', '3'),
        draft('v5.0.0-alpha.9', '1'),
        draft('v5.0.0-alpha.10', '2'),
      ],
    });
    expect(tags(plan)).toEqual(['v5.0.0-alpha.9', 'v5.0.0-alpha.10', 'v5.0.0-alpha.11']);
    expect(plan.builds[0]).toEqual({
      tag: 'v5.0.0-alpha.9',
      sha: sha('1'),
      createdAt: '2026-09-30T01:00:00Z',
    });
    expect(plan.blocked).toEqual([]);
  });

  it('breaks a tie in creation time by tag, so the order is deterministic', () => {
    const same = '2026-09-30T01:00:00Z';
    const plan = planReleaseBuilds({
      product: 'cli',
      releases: [
        draft('v5.0.0-alpha.10', '1', [], { created_at: same }),
        draft('v5.0.0-alpha.9', '1', [], { created_at: same }),
      ],
    });
    expect(tags(plan)).toEqual(['v5.0.0-alpha.10', 'v5.0.0-alpha.9']);
  });

  it('ignores other products, published releases, tags the policy does not publish, and bundled drafts', () => {
    const plan = planReleaseBuilds({
      product: 'office',
      releases: [
        draft('tmt-office-v0.1.0-alpha.4', '1'),
        draft('tmt-squad-v0.1.0-alpha.2', '1'),
        draft('v5.0.0-alpha.9', '1'),
        draft('some-other-tag', '1'),
        draft('tmt-office-v0.1.0-alpha.5', '2', [BUNDLE_ASSET, 'dist-manifest.json']),
        draft('tmt-office-v0.1.0-alpha.6', '3', [], { draft: false }),
      ],
    });
    expect(tags(plan)).toEqual(['tmt-office-v0.1.0-alpha.4']);
    expect(plan.blocked).toEqual([]);
  });

  it('parks a draft whose failure is recorded, and says why', () => {
    const plan = planReleaseBuilds({
      product: 'squad',
      releases: [
        draft('tmt-squad-v0.1.0-alpha.2', '1', [FAILURE_ASSET]),
        draft('tmt-squad-v0.1.0-alpha.3', '2'),
      ],
    });
    expect(tags(plan)).toEqual(['tmt-squad-v0.1.0-alpha.3']);
    expect(plan.blocked).toEqual([
      {
        tag: 'tmt-squad-v0.1.0-alpha.2',
        reason: 'its verification failed earlier; retry it by dispatch or delete the draft',
      },
    ]);
  });

  it('never builds a commit it cannot name: a branch or short target parks the draft', () => {
    for (const target of ['main', 'abc1234', '', sha('a').toUpperCase()]) {
      const plan = planReleaseBuilds({
        product: 'cli',
        releases: [draft('v5.0.0-alpha.9', '1', [], { target_commitish: target })],
      });
      expect(plan.builds, target).toEqual([]);
      expect(plan.blocked[0]?.reason, target).toContain('not a commit SHA');
    }
  });

  it('lets a bundle win over a failure marker, since the bundle is uploaded last and after the last check', () => {
    const plan = planReleaseBuilds({
      product: 'cli',
      releases: [draft('v5.0.0-alpha.9', '1', [FAILURE_ASSET, BUNDLE_ASSET])],
    });
    expect(plan).toEqual({ builds: [], blocked: [] });
  });

  it('plans only the retried draft and ignores its failure marker', () => {
    const releases = [draft('v5.0.0-alpha.9', '1', [FAILURE_ASSET]), draft('v5.0.0-alpha.10', '2')];
    const plan = planReleaseBuilds({ product: 'cli', releases, retry: 'v5.0.0-alpha.9' });
    expect(tags(plan)).toEqual(['v5.0.0-alpha.9']);
    expect(plan.blocked).toEqual([]);
  });

  it.each([
    [
      'a published release',
      draft('v5.0.0-alpha.9', '1', [], { draft: false }),
      'not a draft release',
    ],
    [
      'another product',
      draft('tmt-office-v0.1.0-alpha.4', '1'),
      'not a draft release of the cli product',
    ],
    [
      'a draft with a bundle',
      draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET]),
      'already carries a verified bundle',
    ],
    [
      'a draft with no commit',
      draft('v5.0.0-alpha.9', '1', [], { target_commitish: 'main' }),
      'is not a commit',
    ],
  ])('refuses to retry %s', (_name, release, message) => {
    expect(() =>
      planReleaseBuilds({ product: 'cli', releases: [release], retry: release.tag_name })
    ).toThrow(message);
  });

  it('refuses to retry a tag that is not listed at all', () => {
    expect(() => planReleaseBuilds({ product: 'cli', releases: [], retry: 'v9.9.9' })).toThrow(
      'Cannot retry v9.9.9'
    );
  });

  it('accepts the pages gh prints with --slurp as well as a flat list', () => {
    const [one, two] = [draft('v5.0.0-alpha.9', '1'), draft('v5.0.0-alpha.10', '2')];
    expect(releasesFrom([[one], [two]])).toEqual([one, two]);
    expect(releasesFrom([one, two])).toEqual([one, two]);
  });

  it('renders the drafts to build and the parked ones for the run summary', () => {
    const text = renderPlanSummary({
      product: 'cli',
      builds: [{ tag: 'v5.0.0-alpha.9', sha: sha('1'), createdAt: '2026-09-30T01:00:00Z' }],
      blocked: [{ tag: 'v5.0.0-alpha.8', reason: 'its verification failed earlier' }],
    });
    expect(text).toContain('Drafts to build and verify, oldest first: `v5.0.0-alpha.9`.');
    expect(text).toContain('**Parked drafts**');
    expect(text).toContain('- `v5.0.0-alpha.8`: its verification failed earlier');
    expect(renderPlanSummary({ product: 'cli', builds: [], blocked: [] })).toContain(
      'No draft release needs a build.'
    );
  });
});

describe('plan-release-builds.mjs', () => {
  function run(args: string[], input: unknown) {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'plan-release-'));
    try {
      const output = path.join(directory, 'output');
      const summary = path.join(directory, 'summary');
      writeFileSync(output, '');
      writeFileSync(summary, '');
      const result = spawnSync('node', [script, ...args], {
        input: JSON.stringify(input),
        encoding: 'utf8',
        env: { PATH: process.env.PATH, GITHUB_OUTPUT: output, GITHUB_STEP_SUMMARY: summary },
        timeout: 10_000,
      });
      return {
        status: result.status,
        stderr: result.stderr,
        output: readFileSync(output, 'utf8'),
        summary: readFileSync(summary, 'utf8'),
      };
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }

  it('writes the matrix and whether anything is planned to the step outputs and the summary', () => {
    const result = run(['--product', 'cli'], [[draft('v5.0.0-alpha.9', '1')], []]);
    expect(result.status).toBe(0);
    expect(result.output).toBe(
      `matrix=${JSON.stringify({ include: [{ tag: 'v5.0.0-alpha.9', sha: sha('1') }] })}\nany=true\n`
    );
    expect(result.summary).toContain('`v5.0.0-alpha.9`');
  });

  it('reports an empty plan as an empty matrix', () => {
    const result = run(['--product', 'squad'], [[draft('v5.0.0-alpha.9', '1')]]);
    expect(result.output).toBe('matrix={"include":[]}\nany=false\n');
  });

  it('fails with a message, and no outputs, for a refused retry or a missing product', () => {
    const refused = run(['--product', 'cli', '--retry', 'v9.9.9'], [[]]);
    expect(refused.status).toBe(1);
    expect(refused.stderr).toContain('Cannot retry v9.9.9');
    expect(refused.output).toBe('');
    const missing = run([], [[]]);
    expect(missing.status).toBe(1);
    expect(missing.stderr).toContain('Usage: plan-release-builds.mjs');
  });
});
