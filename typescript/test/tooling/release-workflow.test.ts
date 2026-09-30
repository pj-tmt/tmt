import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (relative: string) => readFileSync(path.join(repository, relative), 'utf8');

const run = read('.github/workflows/native-release.yml');
const bundle = read('.github/workflows/native-release-bundle.yml');

/** Jobs as raw text, keyed by name; a workflow lists them at two spaces under `jobs:`. */
function jobs(workflow: string): Map<string, string> {
  const body = workflow.slice(workflow.indexOf('\njobs:\n') + 1);
  const found = new Map<string, string>();
  for (const block of body.split(/\n(?=  [a-z][a-z0-9-]*:\n)/).slice(1)) {
    found.set(block.slice(2, block.indexOf(':')), block);
  }
  return found;
}
const job = (workflow: string, name: string) => {
  const found = jobs(workflow).get(name);
  expect(found, `job ${name}`).toBeDefined();
  return found as string;
};

describe('per-product release run (native-release.yml)', () => {
  it('holds the whole run in one queued group per product, and never a group on a job', () => {
    expect(run).toMatch(
      /^concurrency:\n {2}group: release-\$\{\{ inputs\.product \}\}\n {2}cancel-in-progress: false$/m
    );
    // A group on the jobs of a matrix replaces each pending leg with the next one, so the
    // oldest draft would be dropped (measured on a throwaway branch).
    for (const [name, text] of [...jobs(run), ...jobs(bundle)]) {
      expect(text, name).not.toMatch(/^ {4}concurrency:/m);
    }
    expect(bundle).not.toMatch(/^concurrency:/m);
  });

  it('plans from the drafts after it holds the group, then builds them oldest first, one at a time', () => {
    const plan = job(run, 'plan');
    expect(plan).toContain("if: github.ref == 'refs/heads/main'");
    expect(plan).toContain('typescript/scripts/plan-release-builds.mjs --product "$PRODUCT"');
    expect(plan).toContain('--retry "$RETRY"');
    const bundleJob = job(run, 'bundle');
    expect(bundleJob).toContain('needs: plan');
    expect(bundleJob).toMatch(/max-parallel: 1/);
    expect(bundleJob).toMatch(/fail-fast: false/);
    expect(bundleJob).toContain('matrix: ${{ fromJSON(needs.plan.outputs.matrix) }}');
    expect(bundleJob).toContain('uses: ./.github/workflows/native-release-bundle.yml');
  });

  it('keeps the manual preparation: one bundle for the main commit, no draft, nothing attached', () => {
    expect(run).toMatch(
      /prepare:\n {8}description:[^\n]*\n {8}required: true\n {8}default: true\n {8}type: boolean/
    );
    expect(job(run, 'plan')).toContain(`'matrix={"include":[{"tag":"","sha":""}]}'`);
    expect(job(run, 'plan')).toContain('A retry needs prepare turned off.');
  });

  it('grants write access only to the jobs that list or upload to draft releases', () => {
    const writers = [...jobs(run), ...jobs(bundle)]
      .filter(([, text]) => /^ {6}contents: write$/m.test(text))
      .map(([name]) => name);
    expect(writers.sort()).toEqual(['attach', 'bundle', 'check', 'plan', 'record-failure']);
    expect(run).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(bundle).toMatch(/^permissions:\n {2}contents: read$/m);
  });
});

describe('release bundle pipeline (native-release-bundle.yml)', () => {
  it('is only callable, and runs the pipeline of the draft it is given', () => {
    expect(bundle).toMatch(/^on:\n {2}workflow_call:/m);
    expect(bundle).not.toContain('workflow_dispatch');
    for (const name of ['build', 'assemble', 'verify']) {
      expect(job(bundle, name), name).toContain('ref: ${{ inputs.sha || github.sha }}');
    }
    // Tooling that talks to the release API comes from the workflow's own commit.
    for (const name of ['check', 'attach', 'record-failure']) {
      expect(job(bundle, name), name).not.toContain('ref: ${{ inputs.sha');
    }
  });

  it('builds a draft only while it has no bundle and no recorded failure, on main', () => {
    expect(job(bundle, 'build')).toContain(
      "if: github.ref == 'refs/heads/main' && needs.check.outputs.todo == 'true'"
    );
    expect(job(bundle, 'build')).toContain('needs: check');
    expect(job(bundle, 'assemble')).toContain('needs: build');
    expect(job(bundle, 'verify')).toContain('needs: assemble');
  });

  it('qualifies every artifact by the draft tag, so drafts of one run do not collide', () => {
    const names = [...bundle.matchAll(/^ {10}(?:name|pattern): (native-[^\n]+)$/gm)].map(
      ([, name]) => name
    );
    expect(names.length).toBeGreaterThanOrEqual(5);
    for (const name of names) expect(name, name).toMatch(/\$\{\{ inputs\.tag( \|\| 'main')? \}\}/);
  });

  it('asserts that the draft tag is the version its commit declares', () => {
    expect(job(bundle, 'assemble')).toContain('RELEASE_TAG: ${{ inputs.tag }}');
    expect(job(bundle, 'assemble')).toContain('is not the $tag that this commit declares');
  });

  it('attaches only after every verify job passed, and records a failure but not a cancellation', () => {
    const attach = job(bundle, 'attach');
    expect(attach).toContain('needs: [check, verify]');
    expect(attach).toContain(
      "if: inputs.tag != '' && needs.check.outputs.todo == 'true' && needs.verify.result == 'success'"
    );
    expect(attach).toContain('release-draft-assets.mjs attach');
    const failure = job(bundle, 'record-failure');
    expect(failure).toContain('needs: [check, build, assemble, verify, attach]');
    expect(failure).toContain("contains(needs.*.result, 'failure')");
    expect(/^ {4}if: (.*)$/m.exec(failure)?.[1]).not.toContain('cancelled');
    expect(failure).toContain('release-draft-assets.mjs record-failure');
  });

  it('never publishes: no job creates, edits or publishes a release', () => {
    for (const text of [run, bundle]) {
      expect(text).not.toMatch(/gh release (create|edit)/);
      expect(text).not.toMatch(/draft=false|--draft=false/);
    }
  });

  it('caches Rust dependencies per product and target, written by main only', () => {
    const build = job(bundle, 'build');
    expect(build).toContain('Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6');
    expect(build).toContain('shared-key: release-${{ inputs.product }}-${{ matrix.target }}');
    expect(build).toContain("save-if: ${{ github.ref == 'refs/heads/main' }}");
  });
});
