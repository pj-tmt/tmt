import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import ts from 'typescript';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (relative: string) => readFileSync(path.join(repository, relative), 'utf8');

const scripts = 'typescript/scripts';
const warmUp = 'uses: ./.github/actions/warm-xcrun';

const runtimeModule = './native-runtime-proof.mjs';
const scriptSources = Object.fromEntries(
  readdirSync(path.join(repository, scripts))
    .filter((name) => name.endsWith('.mjs') && name !== 'native-runtime-proof.mjs')
    .map((name) => [name, read(`${scripts}/${name}`)])
);

/** Raw discovery remains complete even for unsupported import/reference forms. */
function runtimeModuleConsumers(sources = scriptSources): string[] {
  return Object.keys(sources)
    .filter((name) => sources[name].includes(runtimeModule))
    .sort();
}

/** Only the reviewed, complete bare host-check declaration avoids proof treatment. */
function hasOnlyHostImport(text: string): boolean {
  if (text.split(runtimeModule).length !== 2) return false;
  const source = ts.createSourceFile('consumer.mjs', text, ts.ScriptTarget.Latest, true);
  const declarations = source.statements
    .filter(ts.isImportDeclaration)
    .filter(
      (node) =>
        ts.isStringLiteralLike(node.moduleSpecifier) && node.moduleSpecifier.text === runtimeModule
    );
  if (declarations.length !== 1) return false;
  return /^import\s*\{\s*(?:assertNativeTarget(?:\s*,\s*nativeHostTarget)?|nativeHostTarget(?:\s*,\s*assertNativeTarget)?)\s*,?\s*\}\s*from\s*(['"])\.\/native-runtime-proof\.mjs\1\s*;\s*$/.test(
    declarations[0].getText(source)
  );
}

function proofConsumers(sources = scriptSources): string[] {
  return runtimeModuleConsumers(sources).filter((name) => !hasOnlyHostImport(sources[name]));
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

const expectedProofConsumers = [
  'native-upgrade-proof.mjs',
  'verify-native-artifact.mjs',
  'verify-native-bootstrap.mjs',
  'verify-native-driver-upgrade.mjs',
  'verify-native-extension-upgrade.mjs',
  'verify-native-installation.mjs',
  'verify-native-runtime.mjs',
  'verify-public-install.mjs',
];
const hostConsumer = 'native-application-schema.mjs';
const expectedModuleConsumers = [...expectedProofConsumers, hostConsumer].sort();

function assertConsumerInventory(sources = scriptSources): void {
  expect(runtimeModuleConsumers(sources)).toEqual(expectedModuleConsumers);
  expect(proofConsumers(sources)).toEqual(expectedProofConsumers);
  expect(
    runtimeModuleConsumers(sources).filter((name) => hasOnlyHostImport(sources[name]))
  ).toEqual([hostConsumer]);
}

function proofJobs(text: string, consumers = proofConsumers()): [string, string][] {
  return [...jobs(text)].filter(
    ([, job]) =>
      runsOnMacOs(text, job) &&
      steps(text, job).some((step) =>
        consumers.some((script) => step.includes(`${scripts}/${script}`))
      )
  );
}

function assertWorkflowWarmup(text: string, label: string, consumers = proofConsumers()): void {
  const selected = proofJobs(text, consumers);
  expect(selected.length, `${label} has no macOS proof jobs`).toBeGreaterThan(0);
  for (const [name, job] of selected) {
    const list = steps(text, job);
    const warm = list.findIndex((step) => step.includes(warmUp));
    const verifier = list.findIndex((step) =>
      consumers.some((script) => step.includes(`${scripts}/${script}`))
    );
    expect(warm, `${name}: the warm-up step is missing`).toBeGreaterThanOrEqual(0);
    expect(verifier, `${name}: the verifier step is missing`).toBeGreaterThanOrEqual(0);
    expect(warm, name).toBeLessThan(verifier);
    expect(list[warm], name).toContain("if: runner.os == 'macOS'");
  }
}

function assertWarmAction(action: string): void {
  expect(action).toContain("if: runner.os == 'macOS'");
  expect(action).toContain('scripts/retry-command.sh" 3 5 /usr/bin/xcrun --find otool');
  expect(action).toContain('scripts/retry-command.sh" 3 5 /usr/bin/xcrun --find lipo');
}

const directToolCall = /(?<![-\w])xcrun(?![-\w])/;
const allowedToolCallers = new Set([
  '.github/actions/warm-xcrun/action.yml',
  `${scripts}/native-runtime-proof.mjs`,
  // Fixture-only assertions of the shared tool lookup, not a new runtime caller.
  'typescript/test/tooling/native-runtime-proof.test.ts',
  // Fixture-only Git lookup warms its cache before changing the child TMPDIR.
  'typescript/test/tooling/release-attribution.test.ts',
  'typescript/test/tooling/xcrun-warmup.test.ts',
]);

function assertToolCallers(sources: Record<string, string>): void {
  const users = Object.keys(sources).filter(
    (file) => /\.(ya?ml|sh|mjs|ts)$/.test(file) && directToolCall.test(sources[file])
  );
  expect(users.filter((file) => !allowedToolCallers.has(file))).toEqual([]);
}

const hostImport = `import { assertNativeTarget } from '${runtimeModule}';`;
const withHostSource = (source: string): Record<string, string> => ({
  ...scriptSources,
  [hostConsumer]: source,
});
const fixtureWorkflow = `name: Fixture
jobs:
  proof:
    runs-on: macos-15
    steps:
      - name: Warm
        if: runner.os == 'macOS'
        ${warmUp}
      - name: Verify
        run: node ${scripts}/verify-native-runtime.mjs
`;

describe('macOS toolchain warm-up before the native runtime proof', () => {
  it('finds all module consumers and exactly the real proof and host-only inventories', () => {
    assertConsumerInventory();
    const prepare = read('.github/workflows/native-release-prepare.yml');
    expect(proofJobs(prepare).map(([name]) => name)).toEqual(['verify']);
    expect(steps(prepare, jobs(prepare).get('build')!).join('\n')).toContain(
      `${scripts}/${hostConsumer}`
    );
    assertWorkflowWarmup(prepare, 'prepare');
  });

  it.each([
    hostImport,
    `import { nativeHostTarget } from "${runtimeModule}";`,
    `import { assertNativeTarget, nativeHostTarget } from '${runtimeModule}';`,
    `import { nativeHostTarget, assertNativeTarget } from '${runtimeModule}';`,
    `import {\n  nativeHostTarget,\n  assertNativeTarget,\n} from "${runtimeModule}";`,
  ])('recognizes only complete bare host bindings: %s', (source) => {
    expect(hasOnlyHostImport(source)).toBe(true);
    expect(runtimeModuleConsumers({ 'different-name.mjs': source })).toEqual([
      'different-name.mjs',
    ]);
    expect(proofConsumers({ 'different-name.mjs': source })).toEqual([]);
    assertConsumerInventory(withHostSource(source));
  });

  it.each(['assertMacOsArchitecture', 'verifyNativeRuntime'])(
    'retains %s as a proof entry, directly, aliased or mixed',
    (entry) => {
      for (const bindings of [entry, `${entry} as proof`, `assertNativeTarget, ${entry}`]) {
        const source = `import { ${bindings} } from '${runtimeModule}';`;
        const sources = withHostSource(source);
        expect(hasOnlyHostImport(source)).toBe(false);
        expect(proofConsumers(sources)).toContain(hostConsumer);
        expect(() => assertConsumerInventory(sources)).toThrow();
        expect(() =>
          assertWorkflowWarmup(
            read('.github/workflows/native-release-prepare.yml'),
            'prepare',
            proofConsumers(sources)
          )
        ).toThrow('build: the warm-up step is missing');
      }
    }
  );

  it.each([
    `import { assertNativeTarget as host } from '${runtimeModule}';`,
    `import { unknown } from '${runtimeModule}';`,
    `import host from '${runtimeModule}';`,
    `import * as host from '${runtimeModule}';`,
    `import '${runtimeModule}';`,
    `export { assertNativeTarget } from '${runtimeModule}';`,
    `await import('${runtimeModule}');`,
    `new URL('${runtimeModule}', import.meta.url);`,
    `import { assertNativeTarget, assertNativeTarget } from '${runtimeModule}';`,
    `import { assertNativeTarget, } from '${runtimeModule}'`,
    `import { assertNativeTarget from '${runtimeModule}';`,
    `import { /* reviewed? */ assertNativeTarget } from '${runtimeModule}';`,
    `${hostImport}\nawait import('${runtimeModule}');`,
    `${hostImport}\n${hostImport}`,
    `// ${hostImport}`,
    `const source = ${JSON.stringify(hostImport)};`,
    'const source = `' + hostImport + '`;',
  ])('keeps unsupported references conservative: %s', (source) => {
    const sources = withHostSource(source);
    expect(hasOnlyHostImport(source)).toBe(false);
    expect(runtimeModuleConsumers(sources)).toEqual(expectedModuleConsumers);
    expect(proofConsumers(sources)).toContain(hostConsumer);
    expect(() => assertConsumerInventory(sources)).toThrow();
  });

  it('detects removal of every proof import and discovery entry, and of the host helper', () => {
    for (const name of expectedModuleConsumers) {
      const removed = { ...scriptSources };
      delete removed[name];
      expect(() => assertConsumerInventory(removed)).toThrow();
      const withoutImport = { ...scriptSources, [name]: '// no runtime module reference' };
      expect(() => assertConsumerInventory(withoutImport)).toThrow();
    }
    for (const name of expectedProofConsumers) {
      // Keeping discovery via a pure host import cannot replace the proof obligation.
      const sources = { ...scriptSources, [name]: hostImport };
      expect(runtimeModuleConsumers(sources)).toEqual(expectedModuleConsumers);
      expect(() => assertConsumerInventory(sources)).toThrow();
    }
    // The old module-only classification is sensitive to the actual nine/eight distinction.
    expect(() => expect(runtimeModuleConsumers()).toEqual(expectedProofConsumers)).toThrow();
    expect(() =>
      assertWorkflowWarmup(
        read('.github/workflows/native-release-prepare.yml'),
        'old inventory',
        runtimeModuleConsumers()
      )
    ).toThrow('build: the warm-up step is missing');
  });

  it('detects missing, late or unconditioned warm-up and lost proof jobs in memory', () => {
    assertWorkflowWarmup(fixtureWorkflow, 'fixture');
    const warmStep =
      "      - name: Warm\n        if: runner.os == 'macOS'\n        " + warmUp + '\n';
    expect(() => assertWorkflowWarmup(fixtureWorkflow.replace(warmStep, ''), 'fixture')).toThrow(
      'the warm-up step is missing'
    );
    expect(() =>
      assertWorkflowWarmup(fixtureWorkflow.replace(warmStep, '') + warmStep, 'fixture')
    ).toThrow();
    expect(() =>
      assertWorkflowWarmup(
        fixtureWorkflow.replace("if: runner.os == 'macOS'", "if: runner.os == 'Linux'"),
        'fixture'
      )
    ).toThrow();
    expect(() =>
      assertWorkflowWarmup(
        fixtureWorkflow.replace('verify-native-runtime.mjs', hostConsumer),
        'fixture'
      )
    ).toThrow('has no macOS proof jobs');
    const prepare = read('.github/workflows/native-release-prepare.yml');
    const actualWarm =
      "      - name: Warm the macOS toolchain lookup\n        if: runner.os == 'macOS'\n        " +
      warmUp +
      '\n';
    expect(prepare.split(actualWarm)).toHaveLength(2);
    expect(() => assertWorkflowWarmup(prepare.replace(actualWarm, ''), 'prepare')).toThrow();
    expect(() =>
      assertWorkflowWarmup(prepare.replace(actualWarm, '') + actualWarm, 'prepare')
    ).toThrow();
  });

  it('detects lost retry or macOS action guards and additional direct tool callers', () => {
    const action = read('.github/actions/warm-xcrun/action.yml');
    for (const removed of [
      "if: runner.os == 'macOS'",
      '3 5 /usr/bin/xcrun --find otool',
      '3 5 /usr/bin/xcrun --find lipo',
    ]) {
      expect(() => assertWarmAction(action.replace(removed, 'removed'))).toThrow();
    }
    assertToolCallers({ [`${scripts}/native-runtime-proof.mjs`]: '/usr/bin/xcrun --find lipo' });
    expect(() =>
      assertToolCallers({ [`${scripts}/unexpected.mjs`]: '/usr/bin/xcrun --find lipo' })
    ).toThrow();
  });

  it.each([
    '.github/workflows/ci.yml',
    '.github/workflows/native-release-prepare.yml',
    '.github/workflows/native-release-smoke.yml',
    '.github/workflows/public-install-smoke-pr.yml',
    '.github/workflows/native-intel.yml',
  ])('warms xcrun before every macOS verifier in %s', (workflow) => {
    assertWorkflowWarmup(read(workflow), workflow);
  });

  it('warms the matching-host driver proof before its release-upgrade orchestrator', () => {
    const workflow = read('.github/workflows/native-release-upgrade-prove.yml');
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
    assertWarmAction(read('.github/actions/warm-xcrun/action.yml'));
  });

  it('keeps xcrun out of every other file, so a new caller has to add its own warm-up', () => {
    const walk = (directory: string): string[] =>
      readdirSync(path.join(repository, directory), { withFileTypes: true }).flatMap((entry) => {
        const relative = `${directory}/${entry.name}`;
        if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : walk(relative);
        return [relative];
      });
    const files = [
      ...walk('.github'),
      ...walk('scripts'),
      ...walk(scripts),
      ...walk('typescript/test'),
    ];
    assertToolCallers(
      Object.fromEntries(
        files.filter((file) => /\.(ya?ml|sh|mjs|ts)$/.test(file)).map((file) => [file, read(file)])
      )
    );
  });
});
