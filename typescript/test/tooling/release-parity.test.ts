import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import {
  checkReleaseParity,
  inventoryWorkflow,
  releaseInventory,
} from '../../scripts/release-parity.mjs';
import { GATES } from '../../scripts/publication-gates.mjs';

const read = (file: string) => readFileSync(new URL(`../../../${file}`, import.meta.url), 'utf8');
const manifest = () => JSON.parse(read('.github/release-parity.json'));
const changed = (file: string, transform: (source: string) => string) => (relative: string) =>
  relative === file ? transform(read(relative)) : read(relative);
const bundle = '.github/workflows/native-release-bundle.yml';
const prepare = '.github/workflows/native-release-prepare.yml';

describe('release gate parity inventory', () => {
  it('covers both release entry points and every local reusable workflow with current evidence', () => {
    expect(checkReleaseParity(manifest(), { read })).toEqual({
      workflows: 6,
      jobs: 22,
      publicationGates: 6,
    });
    expect(Object.keys(releaseInventory(read))).toContain('native-release-upgrade.yml');
    expect(Object.keys(releaseInventory(read))).toContain('native-release-smoke.yml');
  });

  it('rejects a release gate added as a new job', () => {
    const readChanged = changed(
      bundle,
      (source) =>
        `${source}\n  new-gate:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Verify new contract\n        run: node scripts/new-gate.mjs\n`
    );
    expect(() => checkReleaseParity(manifest(), { read: readChanged })).toThrow(
      'unmapped new-gate'
    );
  });

  it('cannot hide a gate behind a JavaScript prototype property name', () => {
    const readChanged = changed(
      bundle,
      (source) =>
        `${source}\n  __proto__:\n    steps:\n      - name: Verify new contract\n        run: node scripts/new-gate.mjs\n`
    );
    expect(() => checkReleaseParity(manifest(), { read: readChanged })).toThrow(
      'unmapped __proto__'
    );
  });

  it('rejects a new gate in an existing job without relying on its exit status', () => {
    const readChanged = changed(
      prepare,
      (source) =>
        `${source.trimEnd()}\n      - name: Verify new contract\n        run: node scripts/new-gate.mjs\n`
    );
    expect(() => checkReleaseParity(manifest(), { read: readChanged })).toThrow(
      'verify: executable step inventory changed'
    );
  });

  it('rejects a gate command inserted into an existing named step', () => {
    const readChanged = changed(prepare, (source) =>
      source.replace(
        '          node typescript/scripts/release-policy.mjs --product "$PRODUCT" | cmp',
        '          node scripts/new-gate.mjs\n          node typescript/scripts/release-policy.mjs --product "$PRODUCT" | cmp'
      )
    );
    expect(() => checkReleaseParity(manifest(), { read: readChanged })).toThrow(
      'verify: executable definition changed'
    );
  });

  it('discovers a newly called reusable workflow instead of checking only the fixed files', () => {
    const readChanged = (file: string) =>
      file === '.github/workflows/new-release-gate.yml'
        ? 'jobs:\n  prove:\n    steps:\n      - name: Verify new contract\n        run: node scripts/new-gate.mjs\n'
        : file === bundle
          ? `${read(file)}\n  delegated-gate:\n    uses: ./.github/workflows/new-release-gate.yml\n`
          : read(file);
    expect(() => checkReleaseParity(manifest(), { read: readChanged })).toThrow(
      'unmapped new-release-gate.yml'
    );
  });

  it('rejects a new policy gate hidden inside the existing publication job', () => {
    expect(() =>
      checkReleaseParity(manifest(), { read, publicationGates: [...GATES, 'new-policy'] })
    ).toThrow('unmapped new-policy');
  });

  it('rejects missing and stale workflows, jobs, steps, policies and incident references', () => {
    const cases = [
      (value: ReturnType<typeof manifest>) => {
        delete value.workflows['release.yml'];
      },
      (value: ReturnType<typeof manifest>) => {
        value.workflows['unused.yml'] = {};
      },
      (value: ReturnType<typeof manifest>) => {
        delete value.workflows['native-release-prepare.yml'].verify;
      },
      (value: ReturnType<typeof manifest>) => {
        value.workflows['release.yml'].obsolete = {};
      },
      (value: ReturnType<typeof manifest>) => {
        value.workflows['release.yml'].cut.steps.push('name:Obsolete gate');
      },
      (value: ReturnType<typeof manifest>) => {
        value.publicationGates.obsolete = { releaseOnly: 'Obsolete' };
      },
      (value: ReturnType<typeof manifest>) => {
        value.incidents['1550'].release.step = 'name:Obsolete gate';
      },
    ];
    for (const mutate of cases) {
      const value = manifest();
      mutate(value);
      expect(() => checkReleaseParity(value, { read })).toThrow();
    }
  });

  it('requires an explicit reason or real job and path selector, never both or neither', () => {
    for (const coverage of [
      {},
      { releaseOnly: '' },
      { releaseOnly: ' ' },
      { preMerge: [] },
      { releaseOnly: 'Live tag state', preMerge: [] },
    ]) {
      const value = manifest();
      delete value.workflows['release.yml'].cut.releaseOnly;
      Object.assign(value.workflows['release.yml'].cut, coverage);
      expect(() => checkReleaseParity(value, { read })).toThrow();
    }
  });

  it('does not accept a selector mentioned only in a comment or shell text', () => {
    const source = read('.github/workflows/ci.yml');
    const job = inventoryWorkflow(source, 'ci.yml', 'native-notices')['native-notices'].body;
    const changedJob = job.replace(
      "    if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_notices == 'true'",
      "    # if: needs.changes.outputs.native_notices == 'true'\n    if: false"
    );
    const readChanged = changed('.github/workflows/ci.yml', (text) =>
      text.replace(job, changedJob)
    );
    expect(() => checkReleaseParity(manifest(), { read: readChanged })).toThrow(
      'preMerge job does not use'
    );
  });

  it('rejects absent counterpart jobs, unconnected selectors and missing policy tests', () => {
    for (const mutate of [
      (value: ReturnType<typeof manifest>) => {
        value.publicationGates.commit.preMerge[0].job = 'missing';
      },
      (value: ReturnType<typeof manifest>) => {
        value.publicationGates.commit.preMerge[0].selection.output = 'missing';
      },
      (value: ReturnType<typeof manifest>) => {
        value.publicationGates.commit.preMerge[0].selection.kind = 'unknown';
      },
      (value: ReturnType<typeof manifest>) => {
        value.publicationGates.commit.preMerge[0].tests = [];
      },
      (value: ReturnType<typeof manifest>) => {
        value.publicationGates.commit.preMerge[0].tests = [
          'typescript/test/tooling/missing.test.ts',
        ];
      },
    ]) {
      const value = manifest();
      mutate(value);
      expect(() => checkReleaseParity(value, { read })).toThrow();
    }
  });

  it('maps the rehearsed prepare stages and incidents to the selected rehearsal job', () => {
    const value = manifest();
    const rehearsal = {
      workflow: 'ci.yml',
      job: 'release-rehearsal',
      selection: {
        kind: 'ci-scope',
        output: 'release_rehearsal',
        value: 'true',
        source: 'typescript/scripts/release-rehearsal.mjs',
      },
      coverage: 'runtime',
    };
    for (const [file, job] of [
      ['native-release-bundle.yml', 'prepare'],
      ['native-release-prepare.yml', 'build'],
      ['native-release-prepare.yml', 'assemble'],
      ['native-release-prepare.yml', 'verify'],
    ])
      expect(value.workflows[file][job].preMerge, `${file}:${job}`).toEqual([rehearsal]);
    for (const issue of ['1534', '1541', '1604', '1616', '1646', '1680'])
      expect(value.incidents[issue].preMerge, issue).toEqual([rehearsal]);
    expect(value.incidents['1680'].release).toEqual({
      workflow: 'native-release-prepare.yml',
      job: 'verify',
      step: 'name:Execute final archive and bootstrap with the matching target process',
    });
    expect(value.incidents['1550'].preMerge.map((c: { job: string }) => c.job)).toEqual([
      'native-notices',
      'release-rehearsal',
    ]);
    expect(value.incidents['1542'].preMerge[0].coverage).toBe('policy');
  });

  it('keeps live-state races release-only with a concrete reason and no rehearsal claim', () => {
    const value = manifest();
    for (const issue of ['1593', '1661']) {
      expect(value.incidents[issue].preMerge, issue).toBeUndefined();
      expect(value.incidents[issue].releaseOnly.length, issue).toBeGreaterThan(40);
      expect(value.incidents[issue].followUp, issue).toBeUndefined();
    }
  });

  it('rejects a rehearsal counterpart whose selector, source or job is not real', () => {
    const rehearsalOf = (value: ReturnType<typeof manifest>) => value.incidents['1604'].preMerge[0];
    for (const mutate of [
      (value: ReturnType<typeof manifest>) => {
        rehearsalOf(value).selection.output = 'missing';
      },
      (value: ReturnType<typeof manifest>) => {
        // ci-scope does not emit this output; the declared source must.
        rehearsalOf(value).selection.source = 'typescript/scripts/ci-scope.mjs';
      },
      (value: ReturnType<typeof manifest>) => {
        rehearsalOf(value).selection.source = '../outside.mjs';
      },
      (value: ReturnType<typeof manifest>) => {
        rehearsalOf(value).selection.source = 'typescript/scripts/missing-selector.mjs';
      },
      (value: ReturnType<typeof manifest>) => {
        rehearsalOf(value).job = 'missing';
      },
      (value: ReturnType<typeof manifest>) => {
        delete value.incidents['1646'];
      },
      (value: ReturnType<typeof manifest>) => {
        value.incidents['1999'] = { releaseOnly: 'Unlisted incident' };
      },
      (value: ReturnType<typeof manifest>) => {
        value.incidents['1646'].release.step = 'name:Obsolete step';
      },
    ]) {
      const value = manifest();
      mutate(value);
      expect(() => checkReleaseParity(value, { read })).toThrow();
    }
  });

  it('does not accept a rehearsal job whose condition no longer uses its selector', () => {
    const source = read('.github/workflows/ci.yml');
    const job = inventoryWorkflow(source, 'ci.yml', 'release-rehearsal')['release-rehearsal'].body;
    const changedJob = job.replace(
      "    if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.release_rehearsal == 'true'",
      "    if: needs.changes.outputs.verify == 'true'"
    );
    expect(changedJob).not.toBe(job);
    const readChanged = changed('.github/workflows/ci.yml', (text) =>
      text.replace(job, changedJob)
    );
    expect(() => checkReleaseParity(manifest(), { read: readChanged })).toThrow(
      'preMerge job does not use'
    );
  });

  it('runs the parity guard even for docs-only PRs and merge groups', () => {
    const quality = inventoryWorkflow(read('.github/workflows/ci.yml'), 'ci.yml', 'code-quality')[
      'code-quality'
    ].body;
    expect(quality).toContain('test/tooling/release-parity.test.ts');
    expect(quality).not.toContain("needs.changes.outputs.native_scope == 'full'");
  });
});

describe('fail-closed workflow admission', () => {
  it('ignores job/step-looking shell text inside a run literal and admits duplicate checkout uses', () => {
    const source =
      'jobs:\n  prove:\n    steps:\n      - uses: actions/checkout@v4\n      - uses: actions/checkout@v4\n      - name: Proof\n        run: |\n          echo "jobs:"\n          new-job:\n            steps:\n              - name: Not a workflow gate\n';
    expect(inventoryWorkflow(source, 'fixture.yml').prove.steps).toEqual([
      'uses:actions/checkout@v4',
      'uses:actions/checkout@v4#2',
      'name:Proof',
    ]);
  });

  it.each([
    'jobs: {}\n',
    'jobs:\n  prove: {steps: []}\n',
    'jobs:\n  prove:\n    steps: *shared\n',
    'jobs:\n  prove:\n    steps:\n      - <<: *shared\n',
    'jobs:\n  prove:\n    steps:\n      - name: Missing executable\n',
    'jobs:\n  prove:\n    uses: org/repository/.github/workflows/release.yml@main\n',
    'jobs:\n  prove:\n    steps:\n      - name: Both\n        uses: action@v1\n        run: echo proof\n',
    'jobs:\n  prove:\n    steps:\n      - run: echo proof\n',
  ])('rejects unsupported or ambiguous structure: %s', (source) => {
    expect(() => inventoryWorkflow(source, 'fixture.yml')).toThrow();
  });
});
