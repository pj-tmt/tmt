import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { PROOF_FILES } from '../../scripts/release-upgrade.mjs';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (relative: string) => readFileSync(path.join(repository, relative), 'utf8');

const run = read('.github/workflows/native-release.yml');
const bundle = read('.github/workflows/native-release-bundle.yml');
const smokeWorkflow = read('.github/workflows/native-release-smoke.yml');

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
    expect(plan).toContain('typescript/scripts/plan-release-builds.mjs');
    expect(plan).toContain('--product "$PRODUCT"');
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
    expect(job(run, 'plan')).toContain('retry, hold and rerun need prepare turned off.');
  });

  it('refuses parked Office before preparation or draft planning, retaining released products', () => {
    const plan = job(run, 'plan');
    expect(plan).toContain(
      'node typescript/scripts/native-release-policy.mjs require-released "$PRODUCT"'
    );
    expect(plan).not.toMatch(/node[^\n]*\s-e\s/);
    const shell = plan
      .slice(plan.indexOf('        run: |\n') + '        run: |\n'.length)
      .split('\n')
      .map((line) => line.replace(/^ {10}/, ''))
      .join('\n');
    expect(run).toMatch(/options:\n {10}- cli\n {10}- squad/);
    const directory = mkdtempSync(path.join(os.tmpdir(), 'release-product-'));
    try {
      const gh = path.join(directory, 'gh');
      writeExecutable(
        gh,
        '#!/bin/sh\nprintf "unexpected release API call\\n" >&2\nexit 97\n',
        0o700
      );
      const search = `${directory}${path.delimiter}${process.env.PATH ?? ''}`;
      for (const prepare of ['true', 'false']) {
        const result = spawnSync('/bin/sh', ['-eu', '-c', shell], {
          cwd: repository,
          env: { PATH: search, PRODUCT: 'office', PREPARE: prepare },
          encoding: 'utf8',
          timeout: 10_000,
        });
        expect(result.error).toBeUndefined();
        expect(result.status).toBe(1);
        expect(result.stdout).toBe('');
        expect(result.stderr.trim()).toBe(
          'office is not released (release: false in .github/components.json).'
        );
      }
      for (const product of ['cli', 'squad']) {
        const output = path.join(directory, product);
        const result = spawnSync('/bin/sh', ['-eu', '-c', shell], {
          cwd: repository,
          env: {
            PATH: search,
            PRODUCT: product,
            PREPARE: 'true',
            RETRY: '',
            HOLD: '',
            RERUN: '',
            GITHUB_OUTPUT: output,
          },
          encoding: 'utf8',
          timeout: 10_000,
        });
        expect(result.error).toBeUndefined();
        expect(result.status).toBe(0);
        expect(result.stderr).toBe('');
        expect(readFileSync(output, 'utf8')).toBe(
          'matrix={"include":[{"tag":"","sha":""}]}\nany=true\n'
        );
      }
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it('grants write access only to the jobs that list, upload to or publish draft releases', () => {
    const writers = [...jobs(run), ...jobs(bundle)]
      .filter(([, text]) => /^ {6}contents: write$/m.test(text))
      .map(([name]) => name);
    expect(writers.sort()).toEqual([
      'attach',
      'bundle',
      'check',
      'finish',
      'gates',
      'plan',
      'publish',
      'record-failure',
      'upgrade',
    ]);
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

  it('publishes only through release-publish.mjs, from one job, and checks the result from one other', () => {
    // The publishing command and its flags live in the script, which its own tests pin; no
    // workflow file spells one out.
    for (const text of [run, bundle]) {
      expect(text).not.toMatch(/gh release (create|edit)/);
      expect(text).not.toMatch(/draft=false|--draft=false/);
    }
    const callers = (command: string) =>
      [...jobs(bundle)]
        .filter(([, text]) => text.includes(`release-publish.mjs ${command}`))
        .map(([name]) => name);
    expect(callers('publish')).toEqual(['publish']);
    expect(callers('verify')).toEqual(['published']);
  });

  it('caches Rust dependencies per product and target, written by main only', () => {
    const build = job(bundle, 'build');
    expect(build).toContain('Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6');
    expect(build).toContain('shared-key: release-${{ inputs.product }}-${{ matrix.target }}');
    expect(build).toContain("save-if: ${{ github.ref == 'refs/heads/main' }}");
  });
});

describe('release workflow (release.yml)', () => {
  const release = read('.github/workflows/release.yml');

  it('starts on every push to main, prose included, and on a manual dry run by default', () => {
    // A documentation merge moves main under the open release pull requests too, so no path is
    // ignored: the run refreshes them.
    expect(release).toMatch(
      /^on:\n {2}push:\n {4}branches:\n {6}- main\n(?: {4}#[^\n]*\n)* {2}workflow_dispatch:\n/m
    );
    expect(release.split(/^jobs:/m)[0]).not.toMatch(/^\s+paths(?:-ignore)?:/m);
    expect(release).toMatch(
      /workflow_dispatch:\n {4}inputs:\n {6}dry_run:\n(?: {8}[^\n]*\n)*? {8}default: true\n {8}type: boolean/
    );
    expect(release.match(/^ {2}[a-z_]+:$/gm)?.slice(0, 2)).toEqual([
      '  push:',
      '  workflow_dispatch:',
    ]);
    expect(release).toMatch(
      /^concurrency:\n {2}group: release-please\n {2}cancel-in-progress: false$/m
    );
  });

  it('holds the release App token in one step of one job, only in a live run, pinned by commit', () => {
    const uses = [...release.matchAll(/uses: actions\/create-github-app-token@(\S+) # (v\S+)/g)];
    expect(uses).toHaveLength(1);
    expect(uses[0][1]).toMatch(/^[0-9a-f]{40}$/);
    const releasePlease = job(release, 'release-please');
    expect(releasePlease).toContain('actions/create-github-app-token@');
    expect(job(release, 'dispatch')).not.toContain('create-github-app-token');
    expect(releasePlease).toMatch(
      /- name: Create the release App token\n {8}id: app\n {8}if: steps\.mode\.outputs\.live == 'true'/
    );
    expect(releasePlease).toContain('permission-contents: write');
    expect(releasePlease).toContain('permission-pull-requests: write');
    // The token is scoped to this repository by default: no owner or repositories input.
    expect(releasePlease).not.toMatch(/^ {10}(owner|repositories):/m);
    // The secrets are read in two places: whether they exist, where the mode is decided, and
    // their values, where the token is created.
    expect(
      [...release.matchAll(/secrets\.(\w+)( != '')?/g)].map(
        ([, name, presence]) => `${name}${presence ? ' (presence)' : ''}`
      )
    ).toEqual([
      'RELEASE_APP_ID (presence)',
      'RELEASE_APP_PRIVATE_KEY (presence)',
      'RELEASE_APP_ID',
      'RELEASE_APP_PRIVATE_KEY',
    ]);
    const token = releasePleasePart(releasePlease, 'Create the release App token');
    expect(token).toContain('app-id: ${{ secrets.RELEASE_APP_ID }}');
    expect(token).toContain('private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}');
  });

  it('decides the mode from the event, the ref and whether the secrets exist, never their values', () => {
    const mode = releasePleasePart(
      job(release, 'release-please'),
      'Decide whether this run changes anything'
    );
    expect([...mode.matchAll(/^ {10}([A-Z_]+):/gm)].map(([, name]) => name)).toEqual([
      'EVENT',
      'REF',
      'DRY_RUN',
      'HAS_APP_SECRETS',
    ]);
    expect(mode).toContain('REF: ${{ github.ref }}');
    expect(mode).toContain(
      "HAS_APP_SECRETS: ${{ secrets.RELEASE_APP_ID != '' && secrets.RELEASE_APP_PRIVATE_KEY != '' }}"
    );
    expect(mode).toContain('run: node typescript/scripts/release-mode.mjs');
  });

  it('reads the App credentials from the release Environment, on the release-please job only', () => {
    expect(job(release, 'release-please')).toMatch(/^ {4}environment: release$/m);
    expect(job(release, 'dispatch')).not.toContain('environment:');
    expect(release.match(/^ {4}environment:/gm)).toHaveLength(1);
  });

  it('runs the pinned release-please API wrapper with the mode gate result', () => {
    const releasePlease = job(release, 'release-please');
    expect(releasePlease).toContain('pnpm install --frozen-lockfile --ignore-scripts');
    expect(releasePlease).toContain('working-directory: .github/release-please');
    expect(releasePlease).toContain('for command in release-pr github-release; do');
    expect(releasePlease).toContain('LIVE: ${{ steps.mode.outputs.live }}');
    expect(releasePlease).toContain('set -o pipefail');
    expect(releasePlease).toContain(
      'node ../../typescript/scripts/release-please-run.mjs "$command"'
    );
    // Candidate construction and dry/live mutation controls are exercised by release-config tests.
    expect(releasePlease).not.toMatch(/googleapis\/release-please-action/);
  });

  it('installs the isolated release pin only in CI jobs that run release-config tests', () => {
    const ci = read('.github/workflows/ci.yml');
    const consumers = [...jobs(ci)]
      .filter(([, text]) => text.includes('working-directory: .github/release-please'))
      .map(([name, text]) => {
        expect(text).toContain('run: pnpm install --frozen-lockfile --ignore-scripts');
        return name;
      });
    expect(consumers).toEqual(['code-quality', 'unit-tests']);
  });

  it('merges release pull requests through the required checks and never around them', () => {
    const step = releasePleasePart(job(release, 'release-please'), 'Enable auto-merge');
    expect(step).toContain(
      "if: steps.mode.outputs.live == 'true' && steps.release.outputs.queue_blocked != 'true'"
    );
    expect(step).toContain('RELEASE_TOKEN: ${{ steps.app.outputs.token }}');
    expect(step).toContain('LIVE: ${{ steps.mode.outputs.live }}');
    expect(step).toContain('node typescript/scripts/release-please-queue.mjs enable');
    expect(step).not.toContain('gh pr update-branch');
    expect(step).not.toContain('gh pr merge');
    for (const bypass of ['--admin', '--force', 'bypass']) expect(release).not.toContain(bypass);
    // Only the release-please job carries write access through the App token; the workflow token
    // stays read-only; a separate advisory job owns issue-write permission.
    const permissions = /permissions:\n((?: {6}[^\n]+\n)+)/.exec(
      job(release, 'release-please')
    )?.[1];
    expect(permissions).toBe('      contents: read\n      pull-requests: read\n');
  });

  it('starts a release run per product, only in a live run, and never publishes', () => {
    const dispatch = job(release, 'dispatch');
    expect(dispatch).toContain('needs: release-please');
    expect(dispatch).toMatch(/product:\n {10}- cli\n {10}- squad/);
    expect(dispatch).toContain('typescript/scripts/plan-release-builds.mjs --product "$PRODUCT"');
    // `prepare` defaults to true in native-release.yml, so a run that attaches to drafts must
    // turn it off explicitly, and this is the only place that starts one.
    expect(dispatch).toContain(
      'gh workflow run native-release.yml --ref main -f "product=$PRODUCT" -f prepare=false'
    );
    expect(dispatch.match(/gh workflow run/g)).toHaveLength(1);
    expect(dispatch).not.toMatch(/prepare=true|-f "?retry=/);
    expect(dispatch).toContain('if [ "$LIVE" = true ]; then');
    expect(dispatch).toContain('contents: write');
    expect(dispatch).toContain('actions: write');
    expect(release).not.toMatch(/gh release (create|edit)|draft=false/);
  });

  it('starts a run for exactly the products of the component map and the release configuration', () => {
    const { components } = JSON.parse(read('.github/components.json')) as {
      components: Record<string, { release?: boolean; owns: string[] }>;
    };
    const products = Object.entries(components)
      .filter(([, component]) => component.release !== false)
      .map(([name]) => name)
      .sort();
    // Parked components own files and CI scope but start no release run.
    for (const parked of ['browser-addon', 'office']) {
      expect(components[parked].release).toBe(false);
      expect(products).not.toContain(parked);
    }
    const matrix = /product:\n((?: {10}- [a-z]+\n)+)/.exec(job(release, 'dispatch'))?.[1] ?? '';
    expect(matrix.match(/[a-z]+(?=\n)/g)?.sort()).toEqual(products);
    const config = JSON.parse(read('release-please-config.json')) as {
      packages: Record<string, unknown>;
    };
    expect(Object.keys(config.packages)).toHaveLength(products.length);
    for (const parked of ['browser-addon', 'office'])
      expect(config.packages[components[parked].owns[0]]).toBeUndefined();
  });
});

/** The text of the step whose name starts with `name`, from a job's raw text. */
function releasePleasePart(text: string, name: string): string {
  const start = text.indexOf(`- name: ${name}`);
  expect(start, name).toBeGreaterThanOrEqual(0);
  const next = text.indexOf('\n      - ', start + 1);
  return text.slice(start, next < 0 ? undefined : next);
}

describe('release upgrade proof (native-release-upgrade.yml)', () => {
  const upgrade = read('.github/workflows/native-release-upgrade.yml');
  const targets = (workflow: string) =>
    [...workflow.matchAll(/- target: (\S+)\n\s+runner: (\S+)/g)].map(([, target, runner]) => [
      target,
      runner,
    ]);

  it('is callable by the publication run and by hand, for any product and release', () => {
    expect(upgrade).toMatch(
      /^on:\n {2}workflow_call:\n {4}inputs:\n(?: {4,}[^\n]*\n)+ {2}workflow_dispatch:\n {4}inputs:\n/m
    );
    for (const input of ['product', 'tag', 'sha']) {
      expect(upgrade.match(new RegExp(`^ {6}${input}:$`, 'gm')), input).toHaveLength(2);
    }
    expect(upgrade).toMatch(/type: choice\n {8}options:\n {10}- cli\n {10}- office\n {10}- squad/);
    // The publication run reads the outcome and the reason, whatever the run's own result is.
    expect(upgrade).toMatch(
      /^ {4}outputs:\n {6}outcome:\n(?: {8}[^\n]*\n)* {8}value: \$\{\{ jobs\.fetch\.outputs\.outcome \}\}\n {6}reason:\n(?: {8}[^\n]*\n)* {8}value: \$\{\{ jobs\.conclude\.outputs\.reason \}\}\n/m
    );
  });

  it('proves on the same four hosts as the bundle verification', () => {
    expect(targets(upgrade)).toHaveLength(4);
    expect(targets(upgrade)).toEqual(targets(bundle));
  });

  it('only reads releases: write access is for seeing draft assets, and nothing is written', () => {
    expect(upgrade).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(upgrade.match(/^ {6}contents: write$/gm)).toHaveLength(1);
    expect(job(upgrade, 'fetch')).toMatch(/^ {6}contents: write$/m);
    expect(job(upgrade, 'prove')).toMatch(/^ {4}permissions:\n {6}contents: read\n/m);
    expect(upgrade).not.toMatch(/actions: write|pull-requests:|id-token:|packages:/);
    expect(upgrade).not.toMatch(
      /gh release (create|edit|upload|delete)|draft=false|--method|gh workflow run/
    );
  });

  it("fetches with the write token on main's code and proves the release's code read-only", () => {
    const fetch = job(upgrade, 'fetch');
    const prove = job(upgrade, 'prove');
    // The job that holds the write token runs this repository's main, never the release commit.
    expect(fetch).toContain("if: github.ref == 'refs/heads/main'");
    expect(fetch).not.toMatch(/^ {10}ref:/m);
    expect(fetch).toContain('release-upgrade.mjs fetch --product "$PRODUCT" --tag "$RELEASE_TAG"');
    expect(fetch).not.toContain('pnpm');
    // The job that runs the release commit's scripts has no token at all.
    expect(prove).toContain('needs: fetch');
    expect(prove).toContain('ref: ${{ needs.fetch.outputs.sha }}');
    expect(prove).not.toMatch(/GH_TOKEN|github\.token|secrets\./);
    expect(prove).not.toMatch(/gh api|gh release/);
    // They meet in one run artifact.
    const name = /name: (upgrade-assets-\$\{\{ inputs\.tag \}\})/.exec(fetch)?.[1];
    expect(name).toBeDefined();
    expect(fetch).toContain('actions/upload-artifact@');
    expect(prove).toContain('actions/download-artifact@');
    expect(prove).toContain(`name: ${name}`);
  });

  it('names why a host failed from its log, as data, without a flag the release commit may lack', () => {
    const prove = job(upgrade, 'prove');
    const conclude = job(upgrade, 'conclude');
    // The release commit's own script runs the proof: only the flags every commit has reach it, and
    // the log is kept by the step around it.
    const call =
      /node typescript\/scripts\/release-upgrade\.mjs prove[^\n]*\n[^\n]*\n/.exec(prove)?.[0] ?? '';
    expect(call).not.toMatch(/--failure|--reason|--log/);
    expect(prove).toContain('set -o pipefail');
    expect(prove).toContain('2>&1 | tee "$RUNNER_TEMP/upgrade-proof.log"');
    expect(prove).toMatch(
      /if: failure\(\)\n {8}uses: actions\/upload-artifact@v4\n {8}with:\n {10}name: upgrade-proof-log-\$\{\{ matrix\.target \}\}/
    );
    // The conclusion runs this repository's code on the default ref and only reads the logs.
    expect(conclude).toMatch(/^ {4}permissions:\n {6}contents: read\n/m);
    expect(conclude).not.toMatch(/ref:|GH_TOKEN|contents: write/);
    expect(conclude).toContain('pattern: upgrade-proof-log-*');
    expect(conclude).toContain(
      'release-upgrade.mjs reason --directory "$RUNNER_TEMP/upgrade-logs"'
    );
    expect(conclude).toContain(
      'reason: ${{ steps.failure.outputs.reason || needs.fetch.outputs.reason }}'
    );
  });

  it('runs the proof from the commit of the release, and says when that commit predates it', () => {
    const fetch = job(upgrade, 'fetch');
    const prove = job(upgrade, 'prove');
    for (const script of PROOF_FILES) {
      expect(read(`typescript/scripts/${script}`), script).not.toBe('');
    }
    expect([...PROOF_FILES].sort()).toEqual(
      [
        'release-upgrade.mjs',
        'release-versions.mjs',
        'verify-native-installation.mjs',
        'verify-native-extension-upgrade.mjs',
      ].sort()
    );
    // Whether the release's commit has the scripts is decided in fetch, without a checkout, and
    // the proof only runs when it does; a release that cannot be proved fails `conclude`.
    expect(fetch).toContain('release-upgrade.mjs assess --directory "$RUNNER_TEMP/upgrade-assets"');
    expect(prove).toContain("if: needs.fetch.outputs.outcome == 'proved'");
    expect(prove).not.toContain('it predates');
    const conclude = job(upgrade, 'conclude');
    expect(conclude).toContain("if: needs.fetch.outputs.outcome == 'predates'");
    expect(conclude).toContain('exit 1');
    expect(conclude).toContain("if: ${{ !cancelled() && needs.fetch.result == 'success' }}");
    expect(prove).toContain('release-upgrade.mjs prove --product "$PRODUCT" --tag "$RELEASE_TAG"');
    expect(prove).toContain('skill=skills/tmux-team/SKILL.md');
    expect(prove).toContain('--skill "$skill"');
    // The macOS toolchain lookup is warmed before the archives run.
    expect(prove.indexOf('warm-xcrun')).toBeGreaterThan(0);
    expect(prove.indexOf('warm-xcrun')).toBeLessThan(prove.indexOf('release-upgrade.mjs prove'));
  });

  it('requires CLI adapter acceptance after the existing proof on every host, with bounded compilation and read-only caching', () => {
    const prove = job(upgrade, 'prove');
    expect(prove).toContain('timeout-minutes: 10');
    expect(prove).toMatch(
      /name: Install Rust for CLI adapter acceptance\n {8}if: inputs.product == 'cli'/
    );
    expect(prove).toMatch(
      /name: Restore Rust dependencies for CLI adapter acceptance\n {8}if: inputs.product == 'cli'/
    );
    expect(prove).toContain('shared-key: native-rust');
    expect(prove).toContain('save-if: false');
    expect(prove).toMatch(
      /name: Prove the real-archive CLI upgrade adapter\n {8}if: inputs.product == 'cli'/
    );
    expect(prove.indexOf('release-upgrade.mjs acceptance')).toBeGreaterThan(
      prove.indexOf('release-upgrade.mjs prove')
    );
    expect(prove).toContain('2>&1 | tee -a "$RUNNER_TEMP/upgrade-proof.log"');
    expect(prove).not.toContain('continue-on-error:');
    expect(prove).toContain('CARGO_TARGET_DIR: ${{ github.workspace }}/rust/target');
    const acceptance = prove.slice(
      prove.indexOf('- name: Prove the real-archive CLI upgrade adapter'),
      prove.indexOf('- name: Keep the log')
    );
    expect(acceptance).toContain('if [ "$CURRENT_TOOLING" = true ]; then');
    expect(acceptance).toContain('source_args=(--source-root "$GITHUB_WORKSPACE/release-source")');
    expect(acceptance).toContain('--directory "$RUNNER_TEMP/upgrade" "${source_args[@]}"');
  });
});

describe('publication gates (native-release-bundle.yml)', () => {
  const gates = job(bundle, 'gates');
  const upgradeJob = job(bundle, 'upgrade');
  const finish = job(bundle, 'finish');

  it('runs the gates for a draft that was just attached, a complete draft awaiting publication, or a held draft whose hold is released', () => {
    expect(gates).toContain('needs: [check, attach]');
    expect(gates).toContain(
      "if: ${{ !cancelled() && inputs.tag != '' && needs.check.result == 'success' && ((needs.check.outputs.todo == 'true' && needs.attach.result == 'success') || inputs.hold || inputs.rerun || needs.check.outputs.awaiting == 'true') }}"
    );
    // A complete draft that an earlier run did not publish is checked and published again.
    expect(job(bundle, 'check')).toContain('awaiting: ${{ steps.check.outputs.awaiting }}');
    expect(bundle).toMatch(
      /hold:\n {8}description:[^\n]*\n {8}required: false\n {8}default: false\n {8}type: boolean/
    );
    expect(gates).toContain(
      'publication-gates.mjs early --product "$PRODUCT" --tag "$RELEASE_TAG" --release-hold'
    );
    expect(gates).toContain(
      'publication-gates.mjs early --product "$PRODUCT" --tag "$RELEASE_TAG"\n'
    );
  });

  it('reads the release commit through git and the API and never checks it out', () => {
    for (const [name, text] of [
      ['gates', gates],
      ['finish', finish],
    ]) {
      expect(text, name).not.toMatch(/^ {10}ref:/m);
      expect(text, name).toMatch(/^ {6}contents: write$/m);
    }
    expect(gates).toContain('fetch-depth: 0');
    expect(gates).toMatch(/^ {6}pull-requests: read$/m);
    expect(gates).toMatch(/^ {6}checks: read$/m);
    expect(finish).not.toMatch(/pull-requests|checks:/);
    expect(gates).not.toMatch(/pnpm|cargo/);
  });

  it('proves the upgrade only after the other gates passed, and skips it only when the hold names it', () => {
    expect(upgradeJob).toContain('needs: gates');
    expect(upgradeJob).toContain(
      "if: ${{ !cancelled() && needs.gates.result == 'success' && needs.gates.outputs.held == '' && needs.gates.outputs.skip != 'upgrade' }}"
    );
    expect(upgradeJob).toContain('uses: ./.github/workflows/native-release-upgrade.yml');
    expect(upgradeJob).toMatch(/^ {6}contents: write$/m);
    expect(finish).toContain('needs: [gates, upgrade]');
    expect(finish).toContain(
      "if: ${{ !cancelled() && needs.gates.result == 'success' && needs.gates.outputs.held == '' }}"
    );
    for (const value of ['result', 'outputs.outcome', 'outputs.reason']) {
      expect(finish).toContain(`needs.upgrade.${value}`);
    }
    expect(finish).toContain('--skip "$SKIP"');
  });

  it('names what it needs in every job after attach, and starts no build after the gates', () => {
    // A job that follows a skipped job is skipped too unless its condition uses a status
    // function, and `attach` is skipped for a draft that is already complete.
    for (const [name, text] of [
      ['gates', gates],
      ['upgrade', upgradeJob],
      ['finish', finish],
      ['publish', job(bundle, 'publish')],
      ['published', job(bundle, 'published')],
      ['smoke', job(bundle, 'smoke')],
    ]) {
      expect(text, name).toContain('!cancelled()');
    }
    for (const name of ['build', 'assemble', 'verify', 'attach']) {
      expect(job(bundle, name), name).not.toMatch(/needs:[^\n]*\b(finish|publish|published)\b/);
    }
  });
});

describe('publication (native-release-bundle.yml)', () => {
  const finish = job(bundle, 'finish');
  const publish = job(bundle, 'publish');
  const published = job(bundle, 'published');

  it('publishes only after finish ran and held nothing, never for a bundle prepared without a draft', () => {
    expect(finish).toContain('held: ${{ steps.finish.outputs.held }}');
    expect(finish).toMatch(
      /- name: Hold the draft, or report that every gate passed\n {8}id: finish\n/
    );
    expect(publish).toContain('needs: finish');
    expect(publish).toContain(
      "if: ${{ !cancelled() && inputs.tag != '' && needs.finish.result == 'success' && needs.finish.outputs.held == '' }}"
    );
    expect(publish).toContain(
      'node typescript/scripts/release-publish.mjs publish --product "$PRODUCT" --tag "$RELEASE_TAG"'
    );
  });

  it("publishes with the write token on main's code and nothing else", () => {
    expect(publish).toMatch(/^ {4}permissions:\n {6}contents: write\n {4}steps:/m);
    expect(publish).not.toMatch(/^ {10}ref:/m);
    expect(publish).not.toMatch(/pnpm|cargo|download-artifact/);
  });

  it('checks the published release after it was published, and reports a failure as an issue', () => {
    expect(published).toContain('needs: publish');
    expect(published).toContain("if: ${{ !cancelled() && needs.publish.result == 'success' }}");
    expect(published).toMatch(
      /^ {4}permissions:\n {6}contents: read\n {6}issues: write\n {4}steps:/m
    );
    expect(published).not.toMatch(/^ {10}ref:/m);
    expect(published).toContain(
      'release-publish.mjs verify --product "$PRODUCT" --tag "$RELEASE_TAG"'
    );
    expect(published).toContain('--directory "$RUNNER_TEMP/published"');
    expect(published).toContain(
      '--run-url "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"'
    );
  });
});

describe('public install smoke (native-release-smoke.yml)', () => {
  const smoke = job(smokeWorkflow, 'smoke');
  const report = job(smokeWorkflow, 'report');
  const upgrade = read('.github/workflows/native-release-upgrade.yml');
  const targets = (workflow: string) =>
    [...workflow.matchAll(/- target: (\S+)\n\s+runner: (\S+)/g)].map(([, target, runner]) => [
      target,
      runner,
    ]);

  it('is called after the published release was checked, and can be run by hand for a published tag', () => {
    const caller = job(bundle, 'smoke');
    expect(caller).toContain('needs: published');
    expect(caller).toContain("if: ${{ !cancelled() && needs.published.result == 'success' }}");
    expect(caller).toContain('uses: ./.github/workflows/native-release-smoke.yml');
    expect(caller).toMatch(/^ {4}permissions:\n {6}contents: read\n {6}issues: write\n/m);
    expect(smokeWorkflow).toMatch(
      /^on:\n {2}workflow_call:\n {4}inputs:\n(?: {4,}[^\n]*\n)+ {2}workflow_dispatch:\n {4}inputs:\n/m
    );
    for (const input of ['product', 'tag']) {
      expect(smokeWorkflow.match(new RegExp(`^ {6}${input}:$`, 'gm')), input).toHaveLength(2);
    }
    expect(smokeWorkflow).toMatch(
      /type: choice\n {8}options:\n {10}- cli\n {10}- office\n {10}- squad/
    );
  });

  it('installs on the same four hosts as the upgrade proof', () => {
    expect(targets(smokeWorkflow)).toHaveLength(4);
    expect(targets(smokeWorkflow)).toEqual(targets(upgrade));
  });

  it('only reads: the install legs have no write access and no token in their step environment', () => {
    expect(smokeWorkflow).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(smoke).toMatch(/^ {4}permissions:\n {6}contents: read\n/m);
    expect(smoke).not.toMatch(/issues: write|contents: write|GH_TOKEN|GITHUB_TOKEN|secrets\./);
    expect(smokeWorkflow).not.toMatch(
      /gh release (create|edit|upload|delete)|draft=false|--method|gh workflow run|actions: write/
    );
    // The tag is checked out as data beside this repository's own code, and nothing of it runs.
    expect(smoke.match(/uses: actions\/checkout@v4/g)).toHaveLength(2);
    expect(smoke).toContain('ref: ${{ inputs.tag }}\n          path: release-source');
    expect(smoke.match(/persist-credentials: false/g)).toHaveLength(2);
    expect(smoke).toContain('node typescript/scripts/verify-public-install.mjs');
    expect(smoke).toContain('--source release-source');
    expect(smoke).not.toMatch(/release-source\/(typescript|scripts)/);
  });

  it('keeps what failed as data and reports it with the only write access, in a job of its own', () => {
    expect(smoke).toMatch(
      /if: always\(\)\n {8}uses: actions\/upload-artifact@v4\n {8}with:\n {10}name: smoke-failures-\$\{\{ matrix\.target \}\}/
    );
    expect(report).toContain('needs: smoke');
    expect(report).toContain("if: ${{ !cancelled() && needs.smoke.result == 'failure' }}");
    expect(report).toMatch(/^ {4}permissions:\n {6}contents: read\n {6}issues: write\n/m);
    expect(report).toContain('pattern: smoke-failures-*');
    expect(report).toContain('--expected-results 4');
    expect(smoke).toContain('timeout-minutes: 25');
    expect(report).toContain(
      'release-publish.mjs report --product "$PRODUCT" --tag "$RELEASE_TAG"'
    );
    expect(report).not.toMatch(/^ {10}ref:/m);
    expect(smokeWorkflow.match(/^ {6}issues: write$/gm)).toHaveLength(1);
  });
});

describe('the release run releases a hold (native-release.yml)', () => {
  it('plans exactly the held draft, only with prepare off, and tells the pipeline', () => {
    const plan = job(run, 'plan');
    expect(run).toMatch(
      /hold:\n {8}description:[^\n]*\n {8}required: false\n {8}default: ''\n {8}type: string/
    );
    expect(plan).toContain('--product "$PRODUCT" --retry "$RETRY" --hold "$HOLD" --rerun "$RERUN"');
    expect(plan).toContain('if [ -n "$RETRY" ] || [ -n "$HOLD" ] || [ -n "$RERUN" ]; then');
    expect(job(run, 'bundle')).toContain(
      "hold: ${{ inputs.hold != '' && inputs.hold == matrix.tag }}"
    );
  });

  it('grants the pipeline what its gates, its publication and its failure issue need and nothing more', () => {
    const bundleJob = job(run, 'bundle');
    expect(bundleJob).toMatch(
      /permissions:\n(?: {6}#[^\n]*\n)* {6}contents: write\n {6}actions: read\n {6}pull-requests: read\n {6}checks: read\n {6}issues: write\n/
    );
  });
});

describe('write access and release code', () => {
  // A job that holds write permission checks out this repository's own ref; a job that runs a
  // release's code (a draft's commit) is read-only.
  const workflows = [
    'native-release.yml',
    'native-release-bundle.yml',
    'native-release-upgrade.yml',
    'release.yml',
  ];

  it('never gives a job write permission and the code of a release commit together', () => {
    for (const name of workflows) {
      const workflow = read(`.github/workflows/${name}`);
      for (const [jobName, text] of jobs(workflow)) {
        const writes = /^ {6}[a-z-]+: write$/m.test(text);
        const checksOutRelease = /^ {10}ref: \$\{\{/m.test(text);
        expect(writes && checksOutRelease, `${name} job ${jobName}`).toBe(false);
      }
    }
  });
});

describe('held release rerun workflow boundary', () => {
  it('passes rerun through the planner, bundle and all gates without a skip', () => {
    expect(run).toContain("rerun: ${{ inputs.rerun != '' && inputs.rerun == matrix.tag }}");
    expect(job(bundle, 'gates')).toContain('--rerun');
    expect(job(bundle, 'finish')).toContain('--rerun-gate "$RERUN_GATE"');
    expect(job(bundle, 'upgrade')).toContain('current-tooling: ${{ inputs.rerun }}');
  });
  it('runs main tooling while keeping release source expectations separate', () => {
    const upgrade = read('.github/workflows/native-release-upgrade.yml');
    expect(job(upgrade, 'fetch')).toContain(
      'inputs.current-tooling && github.sha || steps.sha.outputs.sha'
    );
    const prove = job(upgrade, 'prove');
    expect(prove).toContain(
      'ref: ${{ inputs.current-tooling && github.sha || needs.fetch.outputs.sha }}'
    );
    expect(prove).toContain('path: release-source');
    expect(prove).toContain('source_args=(--source-root "$GITHUB_WORKSPACE/release-source")');
    expect(prove).toContain('skill="$GITHUB_WORKSPACE/release-source/skills/tmux-team/SKILL.md"');
    expect(prove).not.toMatch(/GH_TOKEN|github\.token|secrets\./);
  });
});
