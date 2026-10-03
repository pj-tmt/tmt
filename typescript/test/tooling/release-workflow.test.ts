import { mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs';
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

describe('independent release-tag concurrency guard', () => {
  it('keys every publishing pipeline concurrency group on the allocated tag', () => {
    const directory = path.join(repository, '.github/workflows');
    for (const file of readdirSync(directory).filter((file) => /release.*\.yml$/.test(file))) {
      const workflow = read(`.github/workflows/${file}`);
      const groups = [...workflow.matchAll(/^\s*group:\s*(.+)$/gm)].map((match) => match[1]);
      if (file === 'release.yml') {
        expect(groups).toEqual(['release-cut']); // Only allocation is serialized.
      } else if (file !== 'project-release.yml' && file !== 'release-version-injection.yml') {
        for (const group of groups) expect(group, `${file}: ${group}`).toContain('inputs.tag');
      }
    }
    expect(run).not.toContain('group: release-${{ inputs.product }}');
    expect(run).toContain('--tag "$RELEASE_TAG"');
  });
  it('does not reintroduce product-wide draft or pipeline refusals in the cut owner', () => {
    const planner = read('typescript/scripts/release-cut.mjs');
    const live = read('typescript/scripts/release-cut-live.mjs');
    expect(planner + live).not.toMatch(
      /draft is in flight|Native release is queued|status: 'in-flight'|drafts\.length/
    );
  });
});

describe('release version gate workflow boundaries', () => {
  it('builds and transfers the Rust TOML helper before ordinary tooling injection fixtures', () => {
    const ci = read('.github/workflows/ci.yml');
    const build = job(ci, 'native-runtime-build');
    const tests = job(ci, 'unit-tests');
    expect(tests).toContain('needs: [changes, native-runtime-build]');
    expect(build).toContain('cargo build --locked -p tmt-release-tool --bin release-version');
    expect(build).toContain("if: matrix.target == 'x86_64-unknown-linux-musl'");
    expect(build).toContain('name: release-version-fixture');
    expect(build).toContain('path: rust/target/debug/release-version');
    expect(build).toContain('if-no-files-found: error');
    expect(tests).toContain('name: release-version-fixture');
    expect(tests).toContain('path: rust/target/debug');
    expect(tests).toContain('chmod +x rust/target/debug/release-version');
    expect(tests.indexOf('chmod +x rust/target/debug/release-version')).toBeLessThan(
      tests.indexOf('pnpm test:run')
    );
  });
  it('builds a gated synthetic debug alpha only for selected installation and upgrade tests', () => {
    const processTests = job(read('.github/workflows/ci.yml'), 'native-process-tests');
    const injection = processTests.indexOf('name: Inject the synthetic alpha installation fixture');
    const build = processTests.indexOf(
      'name: Build the synthetic alpha installation fixture in debug mode'
    );
    const verify = processTests.indexOf('name: Verify the synthetic alpha fixture source gate');
    const restore = processTests.indexOf('name: Restore the reviewed development source');
    const tests = processTests.indexOf('name: Verify native process and shared parser contracts');
    expect(injection).toBeGreaterThan(0);
    expect(build).toBeGreaterThan(injection);
    expect(verify).toBeGreaterThan(build);
    expect(restore).toBeGreaterThan(verify);
    expect(tests).toBeGreaterThan(restore);
    expect(processTests).toContain(
      "contains(needs.changes.outputs.scoped_native_tests, 'extension-install.test.ts')"
    );
    expect(processTests).toContain('release-version-state.json');
    expect(processTests).not.toContain('tag: v5.0.0-alpha.999999');
    expect(processTests).toContain('cargo build --offline --locked -p tmt-cli --bin tmt');
    expect(processTests).toContain('phase: verify');
    expect(processTests).toContain('git diff --exit-code HEAD --');
    expect(processTests).toContain('Synthetic alpha installation fixture debug build:');
  });
  it('keeps native injection on pinned PR heads and all four hosts, with no publishing privileges', () => {
    const injection = read('.github/workflows/release-version-injection.yml');
    expect(injection).toContain('github.event.pull_request.head.sha || github.sha');
    expect(injection).toContain('product: [cli, squad, remote, colab]');
    for (const host of ['macos-15', 'macos-15-intel', 'ubuntu-24.04-arm', 'ubuntu-24.04'])
      expect(injection).toContain(`runner: ${host}`);
    const action = read('.github/actions/inject-release-version/action.yml');
    expect(injection).toContain('uses: ./.github/actions/inject-release-version');
    expect(action).toContain('update --offline --workspace');
    expect(action).toContain('Injected version incorrectly accepted a stale lockfile.');
    expect(action).toContain('metadata --offline --locked');
    expect(injection).toContain(
      'cargo build --offline --locked -p tmt-release-tool --bin release-version'
    );
    expect(injection).toContain(
      'cargo test --offline --locked -p tmt-release-tool --bin release-version'
    );
    expect(injection).toContain('release-version-injection.mjs artifact');
    expect(injection).not.toMatch(/contents: write|actions: write|secrets\.|workflow_dispatch/);
  });
});

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

describe('per-tag release run (native-release.yml)', () => {
  it('holds one allocated tag in a queued group without serializing other tags', () => {
    expect(run).toMatch(
      /^concurrency:\n {2}group: release-\$\{\{ inputs\.tag \|\| inputs\.retry \|\| inputs\.hold \|\| inputs\.rerun \|\| github\.run_id \}\}\n {2}cancel-in-progress: false$/m
    );
    // A group on the jobs of a matrix replaces each pending leg with the next one, so the
    // oldest draft would be dropped (measured on a throwaway branch).
    for (const [name, text] of [...jobs(run), ...jobs(bundle)]) {
      expect(text, name).not.toMatch(/^ {4}concurrency:/m);
    }
    expect(bundle).not.toMatch(/^concurrency:/m);
  });

  it('plans only the exact tag after acquiring that tag group', () => {
    const plan = job(run, 'plan');
    expect(plan).toContain("if: github.ref == 'refs/heads/main'");
    expect(plan).toContain('typescript/scripts/plan-release-builds.mjs');
    expect(plan).toContain('--product "$PRODUCT"');
    expect(plan).toContain('--retry "$RETRY"');
    const bundleJob = job(run, 'bundle');
    expect(bundleJob).toContain('needs: plan');
    expect(bundleJob).not.toMatch(/max-parallel: 1/);
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

  it('refuses parked Office and Herdr before preparation or draft planning, retaining released products', () => {
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
      for (const [product, prepare] of ['office', 'driver-herdr'].flatMap((product) =>
        ['true', 'false'].map((prepare) => [product, prepare])
      )) {
        const result = spawnSync('/bin/sh', ['-eu', '-c', shell], {
          cwd: repository,
          env: { PATH: search, PRODUCT: product, PREPARE: prepare },
          encoding: 'utf8',
          timeout: 10_000,
        });
        expect(result.error).toBeUndefined();
        expect(result.status).toBe(1);
        expect(result.stdout).toBe('');
        expect(result.stderr.trim()).toBe(
          `${product} is not released (release: false in .github/components.json).`
        );
      }
      for (const product of ['cli', 'squad', 'remote', 'colab']) {
        const output = path.join(directory, product);
        const result = spawnSync('/bin/sh', ['-eu', '-c', shell], {
          cwd: repository,
          env: {
            PATH: search,
            PRODUCT: product,
            PREPARE: 'true',
            RELEASE_TAG: '',
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
  it('builds Colab with frozen embedded assets and verifies outside the checkout fallback', () => {
    expect(run).toMatch(
      /options:\n {10}- cli\n {10}- squad\n {10}- driver-herdr\n {10}- remote\n {10}- colab/
    );
    expect(job(bundle, 'build')).toContain(
      "if: inputs.product == 'office' || inputs.product == 'colab'"
    );
    const verify = job(bundle, 'verify');
    expect(verify).toContain(
      'corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match build'
    );
    expect(verify).toContain(
      'mv ../extensions/tmt-colab/typescript/app/dist "$RUNNER_TEMP/colab-app"'
    );
    expect(verify).toContain('elif [ "$PRODUCT" = colab ]; then');
    expect(verify).toContain(
      '--archive "target/distrib/tmt-colab-$TARGET.tar.gz" --target "$TARGET"'
    );
    expect(verify).toContain('--app-dir "$RUNNER_TEMP/colab-app"');
  });

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
    expect(job(bundle, 'assemble')).toContain(
      'is not the $tag that the injected checkout declares'
    );
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

describe('live main release cuts (release.yml)', () => {
  const release = read('.github/workflows/release.yml');
  it('cuts on main push with cadence, hourly backup and unchanged manual dry run', () => {
    expect(release).toMatch(/^  push:\n    branches: \[main\]$/m);
    expect(release).toContain('Release cut (dry dispatch)');
    expect(release).toContain('Release cut (live dispatch)');
    expect(release.split(/^jobs:/m)[0]).toContain('workflow_dispatch:');
    expect(release).toContain("- cron: '17 * * * *'");
    expect(release.split(/^jobs:/m)[0]).not.toMatch(/paths(?:-ignore)?:/);
    expect(release).toMatch(/dry_run:\n(?: {8}[^\n]*\n)*? {8}default: true/);
    expect(release).toContain('group: release-cut');
    expect(release).toContain('cancel-in-progress: false');
  });
  it('records admission after the gate and skips all cut work on cadence', () => {
    const cut = job(release, 'cut');
    expect(cut.indexOf('id: mode')).toBeLessThan(cut.indexOf('name: Admit live release cut'));
    expect(cut.indexOf('name: Admit live release cut')).toBeLessThan(
      cut.indexOf('uses: ./.github/actions/setup-tooling')
    );
    expect(cut).toContain("if: steps.mode.outputs.live == 'true'");
    expect(cut).toMatch(
      /name: Draft immutable source cuts and dispatch their native gates\n        if: steps.mode.outputs.reason != 'cadence'/
    );
    expect(cut).toContain("if: always() && steps.mode.outputs.reason != 'cadence'");
  });
  it('captures trusted main with frozen rendering dependencies and bounded REST mutation only', () => {
    const cut = job(release, 'cut');
    expect(cut).toContain("if: github.ref == 'refs/heads/main'");
    expect(cut).toContain('environment: release');
    expect(cut).toContain('persist-credentials: false');
    expect(cut).toContain('fetch-depth: 0');
    expect(cut).toContain('pnpm install --frozen-lockfile --ignore-scripts');
    expect(cut).toContain('GH_TOKEN: ${{ github.token }}');
    expect(cut).toContain('LIVE: ${{ steps.mode.outputs.live }}');
    expect(cut).toContain('node typescript/scripts/release-cut-live.mjs');
    expect(cut).toContain('contents: write');
    expect(cut).toContain('actions: write');
    expect(release).not.toMatch(
      /release-please|release-pr-safety|release-stall|create-github-app-token|pull-requests:|gh workflow|gh release|1445/
    );
  });
  it('keeps owner versions explicit and parked products out of manual selection', () => {
    expect(release).toContain('options: [all, cli, squad, remote, colab]');
    expect(release).toContain('VERSION: ${{ inputs.version }}');
    expect(release).not.toContain('driver-herdr');
    expect(release).toContain('release-cut-plan.json');
    expect(release).toContain('if: always()');
  });
});

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
        'verify-native-driver-upgrade.mjs',
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
    expect(prove).toContain('release-upgrade.mjs" prove --product "$PRODUCT" --tag "$RELEASE_TAG"');
    expect(prove).toContain('skill=skills/tmux-team/SKILL.md');
    expect(prove).toContain('--skill "$skill"');
    // The macOS toolchain lookup is warmed before the archives run.
    expect(prove.indexOf('warm-xcrun')).toBeGreaterThan(0);
    expect(prove.indexOf('warm-xcrun')).toBeLessThan(prove.indexOf('release-upgrade.mjs" prove'));
  });

  it('requires CLI adapter acceptance after the existing proof on every host, with bounded compilation and read-only caching', () => {
    const prove = job(upgrade, 'prove');
    expect(prove).toContain('timeout-minutes: 10');
    expect(prove).toMatch(/name: Install Rust for version-only resolution and adapter acceptance/);
    expect(prove).toMatch(
      /name: Restore Rust dependencies for version-only resolution and adapter acceptance/
    );
    expect(prove).toContain('shared-key: native-rust');
    expect(prove).toContain('save-if: false');
    expect(prove).toMatch(
      /name: Prove the real-archive CLI upgrade adapter\n {8}if: inputs.product == 'cli'/
    );
    expect(prove.indexOf('release-upgrade.mjs" acceptance')).toBeGreaterThan(
      prove.indexOf('release-upgrade.mjs" prove')
    );
    expect(prove).toContain('2>&1 | tee -a "$RUNNER_TEMP/upgrade-proof.log"');
    expect(prove).not.toContain('continue-on-error:');
    expect(prove).toContain('CARGO_TARGET_DIR: ${{ github.workspace }}/release-source/rust/target');
    const acceptance = prove.slice(
      prove.indexOf('- name: Prove the real-archive CLI upgrade adapter'),
      prove.indexOf('- name: Keep the log')
    );
    expect(acceptance).toContain('if [ "$CURRENT_TOOLING" = true ]; then');
    expect(acceptance).toContain('set -- --source-root "$GITHUB_WORKSPACE/release-source"');
    expect(acceptance).toContain('--directory "$RUNNER_TEMP/upgrade" "$@"');
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
    expect(gates).toContain('pnpm install --frozen-lockfile --ignore-scripts');
    expect(gates).not.toMatch(/cargo build|cargo test/);
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
  const host = read('.github/actions/public-install-smoke/action.yml');
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

  it('authenticates only the shared acquisition step while keeping install permissions read-only', () => {
    expect(smokeWorkflow).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(smoke).toMatch(/^ {4}permissions:\n {6}contents: read\n/m);
    expect(smoke).not.toMatch(/issues: write|contents: write|GH_TOKEN|GITHUB_TOKEN|secrets\./);
    expect(smokeWorkflow).not.toMatch(
      /gh release (create|edit|upload|delete)|draft=false|--method|gh workflow run/
    );
    expect(smoke).not.toContain('actions: write');
    expect(report).not.toContain('actions: write');
    expect(smokeWorkflow).not.toContain('actions: write');
    expect(smokeWorkflow).not.toContain('retry-dispatch');
    // The tag is checked out as data beside this repository's own code, and nothing of it runs.
    expect(smoke.match(/uses: actions\/checkout@v4/g)).toHaveLength(1);
    expect(smoke).toContain('uses: ./.github/actions/public-install-smoke');
    expect(smoke).toContain('tag: ${{ inputs.tag }}');
    expect(smoke).toContain('target: ${{ matrix.target }}');
    expect(smoke.match(/persist-credentials: false/g)).toHaveLength(1);
    expect(host.match(/uses: actions\/checkout@v4/g)).toHaveLength(1);
    expect(host).toContain('ref: ${{ inputs.tag }}\n        path: release-source');
    expect(host.match(/persist-credentials: false/g)).toHaveLength(1);
    expect(host).toContain('node typescript/scripts/verify-public-install.mjs');
    expect(host).toContain('--source release-source');
    expect(host).toContain('GITHUB_TOKEN: ${{ github.token }}');
    expect(host).not.toContain('--retry');
    expect(host).not.toMatch(/GH_TOKEN:|continue-on-error|release-source\/(typescript|scripts)/);
  });

  it('forwards the action credential through env without putting it in argv or output files', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'authenticated-public-host-'));
    const token = 'fixture-action-secret';
    const record = path.join(root, 'arguments');
    const summary = path.join(root, 'summary');
    try {
      writeExecutable(
        path.join(root, 'node'),
        `#!/bin/sh
[ "$GITHUB_TOKEN" = "$EXPECTED_TOKEN" ] || exit 1
printf '%s\\n' "$@" > "$RECORD_FILE"
`,
        0o755
      );
      const script = host.split('      run: |\n')[1].replace(/^        /gm, '');
      const result = spawnSync('/bin/bash', ['-e', '-o', 'pipefail', '-c', script], {
        cwd: repository,
        env: {
          PATH: `${root}:/usr/bin:/bin`,
          GITHUB_TOKEN: token,
          EXPECTED_TOKEN: token,
          RECORD_FILE: record,
          PRODUCT: 'cli',
          RELEASE_TAG: 'v5.0.0-alpha.12',
          TARGET: 'aarch64-unknown-linux-musl',
          RUNNER_TEMP: root,
          GITHUB_STEP_SUMMARY: summary,
        },
        encoding: 'utf8',
        timeout: 10_000,
      });
      expect(result.status, result.stderr).toBe(0);
      expect(readFileSync(record, 'utf8').trim().split('\n')).toEqual([
        'typescript/scripts/verify-public-install.mjs',
        '--product',
        'cli',
        '--tag',
        'v5.0.0-alpha.12',
        '--source',
        'release-source',
        '--target',
        'aarch64-unknown-linux-musl',
        '--result-file',
        path.join(root, 'smoke-result.json'),
      ]);
      for (const text of [
        result.stdout,
        result.stderr,
        readFileSync(record, 'utf8'),
        readFileSync(summary, 'utf8'),
      ])
        expect(text).not.toContain(token);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('loads Herdr archive dependencies in the shared public smoke host owner', () => {
    expect(host).toContain("if: inputs.product == 'driver-herdr'");
    expect(host).toContain('uses: ./.github/actions/setup-tooling');
    expect(host).toContain('pnpm install --frozen-lockfile --ignore-scripts');
    expect(smoke).not.toContain('pnpm install');
  });

  it('keeps what failed as data and reports it with the only issue write access, in a job of its own', () => {
    expect(smoke).toMatch(
      /if: always\(\)\n {8}uses: actions\/upload-artifact@v4\n {8}with:\n {10}name: smoke-failures-\$\{\{ inputs\.product \}\}-\$\{\{ inputs\.tag \}\}-\$\{\{ matrix\.target \}\}/
    );
    expect(report).toContain('needs: smoke');
    expect(report).toContain("if: ${{ !cancelled() && needs.smoke.result == 'failure' }}");
    expect(report).toMatch(/^ {4}permissions:\n {6}contents: read\n {6}issues: write\n/m);
    expect(report).toContain('pattern: smoke-failures-${{ inputs.product }}-${{ inputs.tag }}-*');
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
    expect(plan).toContain(
      '--product "$PRODUCT" --tag "$RELEASE_TAG" --retry "$RETRY" --hold "$HOLD" --rerun "$RERUN"'
    );
    expect(plan).toContain(
      'if [ -n "$RELEASE_TAG" ] || [ -n "$RETRY" ] || [ -n "$HOLD" ] || [ -n "$RERUN" ]; then'
    );
    expect(job(run, 'bundle')).toContain(
      "hold: ${{ inputs.hold != '' && inputs.hold == matrix.tag }}"
    );
  });

  it('grants the pipeline what its gates, its publication and its failure issue need and nothing more', () => {
    const bundleJob = job(run, 'bundle');
    expect(bundleJob).toMatch(
      /permissions:\n(?: {6}#[^\n]*\n)* {6}contents: write\n {6}actions: write\n {6}pull-requests: read\n {6}checks: read\n {6}issues: write\n/
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
        const checksOutRelease = [...text.matchAll(/^ {10}ref: (.+)$/gm)].some(
          ([, ref]) => ref !== '${{ github.sha }}'
        );
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
    expect(prove).toContain('ref: ${{ github.sha }}');
    expect(prove).toContain('path: release-source');
    expect(prove).toContain('set -- --source-root "$GITHUB_WORKSPACE/release-source"');
    expect(prove).toContain('skill="$GITHUB_WORKSPACE/release-source/skills/tmux-team/SKILL.md"');
    expect(prove).not.toMatch(/GH_TOKEN|github\.token|secrets\./);
  });
});

describe('post-publication Project reconciliation', () => {
  const dispatch = job(bundle, 'project-release');
  it('waits for read-back and all smoke jobs, requiring successful smoke after authenticated failures', () => {
    expect(dispatch).toContain('needs: [published, smoke]');
    expect(dispatch).toContain(
      "if: ${{ !cancelled() && needs.published.result == 'success' && needs.smoke.result == 'success' }}"
    );
    expect(smokeWorkflow).not.toContain('outcome:');
    expect(dispatch).not.toContain('infrastructure');
    expect(job(smokeWorkflow, 'report')).toContain('--expected-results 4');
    expect(job(smokeWorkflow, 'smoke')).not.toContain('continue-on-error: true');
  });
  it('grants only dispatch permission, uses GITHUB_TOKEN REST, and runs no release code', () => {
    expect(dispatch).toMatch(/permissions:\n {6}actions: write\n {4}steps:/);
    expect(dispatch).toContain('GH_TOKEN: ${{ github.token }}');
    expect(dispatch).toContain('actions/workflows/project-release.yml/dispatches');
    expect(dispatch).toContain('{"ref":"main","inputs":{"dry_run":"false"}}');
    expect(dispatch).not.toMatch(
      /checkout|secrets\.|contents:|pull-requests:|issues:|environment:/
    );
    expect(
      [...jobs(bundle)]
        .filter(([, text]) => /^ {6}actions: write$/m.test(text))
        .map(([name]) => name)
    ).toEqual(['project-release']);
    // Only the Project dispatch job needs actions:write after deferred smoke retry removal.
    expect(job(bundle, 'smoke')).toContain('uses: ./.github/workflows/native-release-smoke.yml');
    expect(job(bundle, 'smoke')).not.toContain('actions: write');
    expect(job(smokeWorkflow, 'smoke')).not.toContain('actions: write');
    expect(job(smokeWorkflow, 'report')).not.toContain('actions: write');
    expect(smokeWorkflow).not.toContain('retry-dispatch');
  });
});

describe('PR-only authenticated public install proof', () => {
  it('uses the production action on the same four hosts with read-only permissions', () => {
    const proof = read('.github/workflows/public-install-smoke-pr.yml');
    const smoke = read('.github/workflows/native-release-smoke.yml');
    const matrix = (text: string) =>
      [...text.matchAll(/- target: (\S+)\n\s+runner: (\S+)/g)].map(([, target, runner]) => [
        target,
        runner,
      ]);
    expect(proof).toMatch(/^on:\n {2}pull_request:/m);
    expect(proof).not.toMatch(
      /workflow_dispatch|workflow_call|pull_request_target|: write|release-publish|dispatches|continue-on-error/
    );
    expect(matrix(proof)).toEqual(matrix(smoke));
    expect(matrix(proof)).toHaveLength(4);
    expect(proof).toContain('uses: ./.github/actions/public-install-smoke');
    expect(proof).toContain('repos/$GITHUB_REPOSITORY/releases/latest');
    expect(proof).toContain('persist-credentials: false');
    expect(proof).toContain('if-no-files-found: error');
  });
});
