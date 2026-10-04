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

  it('records final-bundle gaps honestly while retaining policy and Rust-notice evidence', () => {
    const value = manifest();
    expect(value.incidents['1534'].followUp).toBe(1581);
    expect(value.incidents['1541'].followUp).toBe(1581);
    expect(value.incidents['1542'].preMerge[0].coverage).toBe('policy');
    expect(value.incidents['1550'].preMerge[0]).toMatchObject({
      job: 'native-notices',
      coverage: 'runtime',
      selection: { output: 'native_notices' },
    });
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
