import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

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
  const anchored = [...job.matchAll(/\*([\w-]+)\n/g)].map(([, alias]) => {
    const start = workflow.indexOf(`&${alias}\n`);
    return workflow.slice(start, workflow.indexOf('runs-on:', start));
  });
  return [job, ...anchored].some((text) => /runner: macos-/.test(text));
}

const steps = (job: string) => job.split(/\n(?=      - )/).slice(1);

describe('macOS toolchain warm-up before the native runtime proof', () => {
  it('finds the proof consumers the workflows run', () => {
    expect(proofConsumers()).toEqual(['verify-native-artifact.mjs', 'verify-native-runtime.mjs']);
  });

  it.each([
    ['.github/workflows/ci.yml', 'packed-native-install'],
    ['.github/workflows/native-release-bundle.yml', 'verify'],
  ])('warms xcrun on macOS before the verifier in %s job %s', (workflow, name) => {
    const text = read(workflow);
    const job = jobs(text).get(name);
    expect(job, `${workflow} has no job ${name}`).toBeDefined();
    expect(runsOnMacOs(text, job as string), `${name} has no macOS runner`).toBe(true);
    const list = steps(job as string);
    const warm = list.findIndex((step) => step.includes(warmUp));
    const verifier = list.findIndex((step) =>
      proofConsumers().some((script) => step.includes(`${scripts}/${script}`))
    );
    expect(warm, 'the warm-up step is missing').toBeGreaterThanOrEqual(0);
    expect(verifier, 'the verifier step is missing').toBeGreaterThanOrEqual(0);
    expect(warm).toBeLessThan(verifier);
    expect(list[warm]).toContain("if: runner.os == 'macOS'");
  });

  it('runs every job that executes a proof consumer on a runner that warms first', () => {
    for (const workflow of ['ci.yml', 'native-release-bundle.yml']) {
      const text = read(`.github/workflows/${workflow}`);
      for (const [name, job] of jobs(text)) {
        const runsProof = proofConsumers().some((script) => job.includes(`${scripts}/${script}`));
        if (runsProof && runsOnMacOs(text, job)) {
          expect(job, `${workflow} job ${name}`).toContain(warmUp);
        }
      }
    }
  });

  it('warms through the bounded retry and only on macOS', () => {
    const action = read('.github/actions/warm-xcrun/action.yml');
    expect(action).toContain("if: runner.os == 'macOS'");
    expect(action).toContain('scripts/retry-command.sh" 3 5 /usr/bin/xcrun --find otool');
  });

  it('keeps xcrun out of every other file, so a new caller has to add its own warm-up', () => {
    const allowed = new Set([
      '.github/actions/warm-xcrun/action.yml',
      `${scripts}/native-runtime-proof.mjs`,
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
