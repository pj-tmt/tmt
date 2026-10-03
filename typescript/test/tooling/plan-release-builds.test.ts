import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import {
  BUNDLE_ASSET,
  FAILURE_ASSET,
  HOLD_ASSET,
  planReleaseBuilds,
  releasesFrom,
  renderPlanSummary,
  type ReleaseObject,
} from '../../scripts/plan-release-builds.mjs';

const script = fileURLToPath(new URL('../../scripts/plan-release-builds.mjs', import.meta.url));
const componentMap = fileURLToPath(new URL('../../../.github/components.json', import.meta.url));

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

  it('selects standalone driver drafts without planning CLI builds or publication', () => {
    const releases = [draft('tmt-driver-herdr-v0.1.0-alpha.1', '1'), draft('v5.0.0-alpha.9', '2')];
    expect(tags(planReleaseBuilds({ product: 'driver-herdr', releases }))).toEqual([
      'tmt-driver-herdr-v0.1.0-alpha.1',
    ]);
    expect(tags(planReleaseBuilds({ product: 'cli', releases }))).toEqual(['v5.0.0-alpha.9']);
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

  it('ignores other products, published releases and tags the policy does not publish, and builds nothing for a bundled draft', () => {
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
    expect(plan.awaiting.map(({ tag }) => tag)).toEqual(['tmt-office-v0.1.0-alpha.5']);
    expect(plan.blocked).toEqual([]);
  });

  it('lists the complete drafts without a hold as awaiting their publication, oldest first', () => {
    const plan = planReleaseBuilds({
      product: 'cli',
      releases: [
        draft('v5.0.0-alpha.11', '3', [BUNDLE_ASSET]),
        draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET, 'dist-manifest.json']),
        draft('v5.0.0-alpha.10', '2', [BUNDLE_ASSET, HOLD_ASSET]),
        draft('v5.0.0-alpha.12', '4'),
      ],
    });
    expect(plan.awaiting.map(({ tag }) => tag)).toEqual(['v5.0.0-alpha.9', 'v5.0.0-alpha.11']);
    expect(plan.held).toEqual([{ tag: 'v5.0.0-alpha.10' }]);
    expect(tags(plan)).toEqual(['v5.0.0-alpha.12']);
  });

  it('parks a complete draft whose target is not a commit, since the commit to publish is unknown', () => {
    const plan = planReleaseBuilds({
      product: 'cli',
      releases: [draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET], { target_commitish: 'main' })],
    });
    expect(plan.awaiting).toEqual([]);
    expect(plan.blocked[0]?.reason).toContain('commit to publish is unknown');
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
    expect(plan).toEqual({
      builds: [],
      awaiting: [{ tag: 'v5.0.0-alpha.9', sha: sha('1'), createdAt: '2026-09-30T01:00:00Z' }],
      blocked: [],
      held: [],
    });
  });

  it('plans only the retried draft and ignores its failure marker', () => {
    const releases = [draft('v5.0.0-alpha.9', '1', [FAILURE_ASSET]), draft('v5.0.0-alpha.10', '2')];
    const plan = planReleaseBuilds({ product: 'cli', releases, retry: 'v5.0.0-alpha.9' });
    expect(tags(plan)).toEqual(['v5.0.0-alpha.9']);
    expect(plan.blocked).toEqual([]);
    expect(plan.awaiting).toEqual([]);
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

  it('lists a complete draft that carries a hold marker as held, and plans no build for it', () => {
    const releases = [
      draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET, HOLD_ASSET]),
      draft('v5.0.0-alpha.10', '2', [BUNDLE_ASSET]),
      draft('v5.0.0-alpha.11', '3'),
    ];
    const plan = planReleaseBuilds({ product: 'cli', releases });
    expect(tags(plan)).toEqual(['v5.0.0-alpha.11']);
    expect(plan.held).toEqual([{ tag: 'v5.0.0-alpha.9' }]);
    expect(plan.blocked).toEqual([]);
  });

  it('plans only the draft whose hold is released, which must be bundled and held', () => {
    const releases = [
      draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET, HOLD_ASSET]),
      draft('v5.0.0-alpha.10', '2', [BUNDLE_ASSET, HOLD_ASSET]),
      draft('v5.0.0-alpha.11', '3', [BUNDLE_ASSET]),
      draft('v5.0.0-alpha.12', '4', [HOLD_ASSET]),
      draft('v5.0.0-alpha.13', '5', [BUNDLE_ASSET, HOLD_ASSET], { target_commitish: 'main' }),
    ];
    const plan = planReleaseBuilds({ product: 'cli', releases, hold: 'v5.0.0-alpha.10' });
    expect(tags(plan)).toEqual(['v5.0.0-alpha.10']);
    expect(plan.held).toEqual([]);
    for (const tag of ['v5.0.0-alpha.11', 'v5.0.0-alpha.12']) {
      expect(() => planReleaseBuilds({ product: 'cli', releases, hold: tag }), tag).toThrow(
        'has no bundle held by publication-held.json'
      );
    }
    expect(() => planReleaseBuilds({ product: 'cli', releases, hold: 'v5.0.0-alpha.13' })).toThrow(
      'is not a commit'
    );
    expect(() => planReleaseBuilds({ product: 'cli', releases, hold: 'v9.9.9' })).toThrow(
      'not a draft release of the cli product'
    );
    expect(() =>
      planReleaseBuilds({ product: 'office', releases, hold: 'v5.0.0-alpha.10' })
    ).toThrow('not a draft release of the office product');
  });

  it('refuses to retry and to release a hold in one run', () => {
    expect(() =>
      planReleaseBuilds({
        product: 'cli',
        releases: [draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET, HOLD_ASSET])],
        retry: 'v5.0.0-alpha.9',
        hold: 'v5.0.0-alpha.9',
      })
    ).toThrow('separate runs');
  });

  it('plans only the bundled held draft for a rerun and refuses missing hold evidence', () => {
    const releases = [
      draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET, HOLD_ASSET]),
      draft('v5.0.0-alpha.10', '2', [BUNDLE_ASSET]),
      draft('v5.0.0-alpha.11', '3', [HOLD_ASSET]),
      draft('v5.0.0-alpha.12', '4', [BUNDLE_ASSET, HOLD_ASSET], { target_commitish: 'main' }),
    ];
    expect(tags(planReleaseBuilds({ product: 'cli', releases, rerun: 'v5.0.0-alpha.9' }))).toEqual([
      'v5.0.0-alpha.9',
    ]);
    for (const tag of ['v5.0.0-alpha.10', 'v5.0.0-alpha.11']) {
      expect(() => planReleaseBuilds({ product: 'cli', releases, rerun: tag })).toThrow(
        'has no bundle held'
      );
    }
    expect(() => planReleaseBuilds({ product: 'cli', releases, rerun: 'v5.0.0-alpha.12' })).toThrow(
      'not a commit'
    );
    expect(() =>
      planReleaseBuilds({ product: 'squad', releases, rerun: 'v5.0.0-alpha.9' })
    ).toThrow('not a draft');
  });

  it.each([{ retry: 'v5.0.0-alpha.9' }, { hold: 'v5.0.0-alpha.9' }])(
    'refuses rerun combined with %o',
    (mode) => {
      expect(() =>
        planReleaseBuilds({ product: 'cli', releases: [], rerun: 'v5.0.0-alpha.9', ...mode })
      ).toThrow('separate runs');
    }
  );

  it('renders the held drafts and a released hold in the run summary', () => {
    const held = renderPlanSummary({
      product: 'cli',
      builds: [],
      blocked: [],
      held: [{ tag: 'v5.0.0-alpha.9' }],
    });
    expect(held).toContain('**Held drafts**');
    expect(held).toContain('- `v5.0.0-alpha.9`');
    expect(held).toContain('publication-held.json');
    const released = renderPlanSummary({
      product: 'cli',
      builds: [{ tag: 'v5.0.0-alpha.9', sha: sha('1'), createdAt: '2026-09-30T01:00:00Z' }],
      blocked: [],
      hold: 'v5.0.0-alpha.9',
    });
    expect(released).toContain('Releasing the hold of v5.0.0-alpha.9 by dispatch.');
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

  it('plans nothing for a component that is not released, and lists its drafts as left alone', () => {
    const plan = planReleaseBuilds({
      product: 'office',
      released: false,
      releases: [
        draft('tmt-office-v0.1.0-alpha.4', '1'),
        draft('tmt-office-v0.1.0-alpha.5', '2', [BUNDLE_ASSET]),
        draft('tmt-office-v0.1.0-alpha.6', '3', [BUNDLE_ASSET, HOLD_ASSET]),
        draft('tmt-office-v0.1.0-alpha.7', '4', [FAILURE_ASSET]),
        draft('v5.0.0-alpha.9', '5'),
      ],
    });
    expect(plan.builds).toEqual([]);
    expect(plan.awaiting).toEqual([]);
    expect(plan.blocked).toEqual([]);
    expect(plan.held).toEqual([]);
    expect(plan.unreleased).toEqual([
      { tag: 'tmt-office-v0.1.0-alpha.4' },
      { tag: 'tmt-office-v0.1.0-alpha.5' },
      { tag: 'tmt-office-v0.1.0-alpha.6' },
      { tag: 'tmt-office-v0.1.0-alpha.7' },
    ]);
  });

  it('refuses to retry or release a hold for a component that is not released', () => {
    const releases = [draft('tmt-office-v0.1.0-alpha.4', '1', [BUNDLE_ASSET, HOLD_ASSET])];
    for (const extra of [
      { retry: 'tmt-office-v0.1.0-alpha.4' },
      { hold: 'tmt-office-v0.1.0-alpha.4' },
      { rerun: 'tmt-office-v0.1.0-alpha.4' },
    ]) {
      expect(() =>
        planReleaseBuilds({ product: 'office', released: false, releases, ...extra })
      ).toThrow('office is not released');
    }
  });

  it('plans a released component as before', () => {
    const plan = planReleaseBuilds({
      product: 'office',
      released: true,
      releases: [draft('tmt-office-v0.1.0-alpha.4', '1')],
    });
    expect(tags(plan)).toEqual(['tmt-office-v0.1.0-alpha.4']);
    expect(plan.unreleased ?? []).toEqual([]);
  });

  it('says in the run summary which drafts it leaves alone', () => {
    const text = renderPlanSummary({
      product: 'office',
      builds: [],
      blocked: [],
      unreleased: [{ tag: 'tmt-office-v0.1.0-alpha.4' }],
    });
    expect(text).toContain('**Left alone** (office is not released');
    expect(text).toContain('- `tmt-office-v0.1.0-alpha.4`');
  });

  it('renders the complete drafts that await their publication in the run summary', () => {
    const text = renderPlanSummary({
      product: 'cli',
      builds: [],
      awaiting: [
        { tag: 'v5.0.0-alpha.9', sha: sha('1'), createdAt: '2026-09-30T01:00:00Z' },
        { tag: 'v5.0.0-alpha.10', sha: sha('2'), createdAt: '2026-09-30T02:00:00Z' },
      ],
      blocked: [],
    });
    expect(text).toContain(
      'Complete drafts that await their gates and publication, oldest first: `v5.0.0-alpha.9`, `v5.0.0-alpha.10`.'
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

  it('puts builds and complete drafts into one matrix, oldest first, and plans a run for the complete ones alone', () => {
    const both = run(
      ['--product', 'cli'],
      [[draft('v5.0.0-alpha.11', '3'), draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET])]]
    );
    expect(both.output).toBe(
      `matrix=${JSON.stringify({
        include: [
          { tag: 'v5.0.0-alpha.9', sha: sha('1') },
          { tag: 'v5.0.0-alpha.11', sha: sha('3') },
        ],
      })}\nany=true\n`
    );
    const resume = run(['--product', 'cli'], [[draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET])]]);
    expect(resume.output).toBe(
      `matrix=${JSON.stringify({ include: [{ tag: 'v5.0.0-alpha.9', sha: sha('1') }] })}\nany=true\n`
    );
    expect(resume.summary).toContain('await their gates and publication');
  });

  it('plans no run for a product the component map does not release', () => {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'plan-components-'));
    try {
      const map = JSON.parse(readFileSync(componentMap, 'utf8')) as {
        components: Record<string, { release?: boolean }>;
      };
      const drafts = [[draft('tmt-office-v0.1.0-alpha.5', '1', [BUNDLE_ASSET])]];
      // The committed map parks Office, so its draft is left alone.
      const result = run(['--product', 'office'], drafts);
      expect(result.status).toBe(0);
      expect(result.output).toBe('matrix={"include":[]}\nany=false\n');
      expect(result.summary).toContain('**Left alone** (office is not released');
      // A map that releases it plans the run.
      delete map.components.office.release;
      const released = path.join(directory, 'components.json');
      writeFileSync(released, JSON.stringify(map));
      expect(run(['--product', 'office', '--components', released], drafts).output).toContain(
        'any=true'
      );
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it('plans no run for a held draft alone', () => {
    const held = run(
      ['--product', 'cli'],
      [[draft('v5.0.0-alpha.9', '1', [BUNDLE_ASSET, HOLD_ASSET])]]
    );
    expect(held.output).toBe('matrix={"include":[]}\nany=false\n');
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
