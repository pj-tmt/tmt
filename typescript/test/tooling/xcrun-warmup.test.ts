import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (relative: string) => readFileSync(path.join(repository, relative), 'utf8');

const scripts = 'typescript/scripts';
const warmUp = 'uses: ./.github/actions/warm-xcrun';

/** The scripts that reach the runtime proof's `xcrun` call, found by their import. */
function proofConsumers(): string[] {
  return readdirSync(path.join(repository, scripts))
    .filter((name) => name.endsWith('.mjs') && name !== 'native-runtime-proof.mjs')
    .filter((name) => read(`${scripts}/${name}`).includes('./native-runtime-proof.mjs'));
}

/** Jobs as their raw text, keyed by name; a workflow lists them at two spaces. */
function jobs(workflow: string): Map<string, string> {
  const body = workflow.slice(workflow.indexOf('\njobs:\n') + 1);
  const found = new Map<string, string>();
  for (const block of body.split(/\n(?=  [a-z][a-z0-9-]*:\n)/).slice(1)) {
    found.set(block.slice(2, block.indexOf(':')), block);
  }
  return found;
}

/**
 * Whether the job's matrix has a macOS runner, directly or through a YAML
 * alias such as `matrix: *native-targets`, whose anchor is in another job.
 */
function runsOnMacOs(workflow: string, job: string): boolean {
  const anchored = [...job.matchAll(/matrix: \*([\w-]+)\n/g)].map(([, alias]) => {
    const start = workflow.indexOf(`&${alias}\n`);
    return workflow.slice(start, workflow.indexOf('runs-on:', start));
  });
  return [job, ...anchored].some((text) => /(?:runner|runs-on): macos-/.test(text));
}

/** Shared step anchors must be inspected at their definition, not skipped. */
function steps(workflow: string, job: string): string[] {
  const alias = job.match(/^ {4}steps: \*([\w-]+)$/m)?.[1];
  const source = alias
    ? [...jobs(workflow).values()].find((block) => block.includes(`steps: &${alias}\n`))
    : job;
  if (!source) throw new Error(`Missing shared steps anchor: ${alias}`);
  return source
    .split(/\n(?=      - )/)
    .slice(1)
    .flatMap((step) =>
      step.includes('uses: ./.github/actions/public-install-smoke')
        ? read('.github/actions/public-install-smoke/action.yml')
            .split(/\n(?=    - )/)
            .slice(1)
        : [step]
    );
}

describe('macOS toolchain warm-up before the native runtime proof', () => {
  it('finds the proof consumers the workflows run', () => {
    expect(proofConsumers()).toEqual([
      'native-upgrade-proof.mjs',
      'verify-native-artifact.mjs',
      'verify-native-bootstrap.mjs',
      'verify-native-driver-upgrade.mjs',
      'verify-native-extension-upgrade.mjs',
      'verify-native-installation.mjs',
      'verify-native-runtime.mjs',
      'verify-public-install.mjs',
    ]);
  });

  it.each([
    '.github/workflows/ci.yml',
    '.github/workflows/native-release-prepare.yml',
    '.github/workflows/native-release-smoke.yml',
    '.github/workflows/public-install-smoke-pr.yml',
    '.github/workflows/native-intel.yml',
  ])('warms xcrun before every macOS verifier in %s', (workflow) => {
    const text = read(workflow);
    const consumers = [...jobs(text)].filter(
      ([, job]) =>
        runsOnMacOs(text, job) &&
        steps(text, job).some((step) =>
          proofConsumers().some((script) => step.includes(`${scripts}/${script}`))
        )
    );
    expect(consumers.length, `${workflow} has no macOS proof jobs`).toBeGreaterThan(0);
    for (const [name, job] of consumers) {
      const list = steps(text, job);
      const warm = list.findIndex((step) => step.includes(warmUp));
      const verifier = list.findIndex((step) =>
        proofConsumers().some((script) => step.includes(`${scripts}/${script}`))
      );
      expect(warm, `${name}: the warm-up step is missing`).toBeGreaterThanOrEqual(0);
      expect(verifier, `${name}: the verifier step is missing`).toBeGreaterThanOrEqual(0);
      expect(warm, name).toBeLessThan(verifier);
      expect(list[warm], name).toContain("if: runner.os == 'macOS'");
    }
  });

  it('warms the matching-host driver proof before its release-upgrade orchestrator', () => {
    const workflow = read('.github/workflows/native-release-upgrade.yml');
    const prove = jobs(workflow).get('prove')!;
    expect(runsOnMacOs(workflow, prove)).toBe(true);
    const list = steps(workflow, prove);
    const warm = list.findIndex((step) => step.includes(warmUp));
    const run = list.findIndex((step) => /release-upgrade\.mjs["']?\s+prove\b/.test(step));
    expect(warm).toBeGreaterThanOrEqual(0);
    expect(run).toBeGreaterThan(warm);
  });

  it('keeps runner selection independent from shared steps and rejects missing anchors', () => {
    const text = read('.github/workflows/ci.yml');
    const linux = jobs(text).get('packed-native-install') as string;
    const macos = jobs(text).get('packed-native-install-macos') as string;
    expect(runsOnMacOs(text, linux)).toBe(false);
    expect(runsOnMacOs(text, macos)).toBe(true);
    expect(steps(text, macos)).toEqual(steps(text, linux));
    expect(() =>
      steps(text, macos.replace('*packed-native-install-steps', '*missing-steps'))
    ).toThrow('Missing shared steps anchor');
  });

  it('warms through the bounded retry and only on macOS', () => {
    const action = read('.github/actions/warm-xcrun/action.yml');
    expect(action).toContain("if: runner.os == 'macOS'");
    expect(action).toContain('scripts/retry-command.sh" 3 5 /usr/bin/xcrun --find otool');
    expect(action).toContain('scripts/retry-command.sh" 3 5 /usr/bin/xcrun --find lipo');
  });

  it('keeps xcrun out of every other file, so a new caller has to add its own warm-up', () => {
    const allowed = new Set([
      '.github/actions/warm-xcrun/action.yml',
      `${scripts}/native-runtime-proof.mjs`,
      // Fixture-only assertions of the shared tool lookup, not a new runtime caller.
      'typescript/test/tooling/native-runtime-proof.test.ts',
      // Fixture-only Git lookup warms its cache before changing the child TMPDIR.
      'typescript/test/tooling/release-attribution.test.ts',
      'typescript/test/tooling/xcrun-warmup.test.ts',
    ]);
    // The tool itself, not the name of the warm-up action.
    const calls = /(?<![-\w])xcrun(?![-\w])/;
    const walk = (directory: string): string[] =>
      readdirSync(path.join(repository, directory), { withFileTypes: true }).flatMap((entry) => {
        const relative = `${directory}/${entry.name}`;
        if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : walk(relative);
        return [relative];
      });
    const users = [
      ...walk('.github'),
      ...walk('scripts'),
      ...walk(scripts),
      ...walk('typescript/test'),
    ].filter((file) => /\.(ya?ml|sh|mjs|ts)$/.test(file) && calls.test(read(file)));
    expect(users.filter((file) => !allowed.has(file))).toEqual([]);
  });
});
