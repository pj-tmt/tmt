import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { PROOF_FILES } from '../../scripts/release-upgrade.mjs';
import { checkReleaseParity } from '../../scripts/release-parity.mjs';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (relative: string) => readFileSync(path.join(repository, relative), 'utf8');

const run = read('.github/workflows/native-release.yml');
const bundle = read('.github/workflows/native-release-bundle.yml');
const prepare = read('.github/workflows/native-release-prepare.yml');
const smokeWorkflow = read('.github/workflows/native-release-smoke.yml');

describe('required manifest packaging cache restore (#2076)', () => {
  const assembly = prepare.split('  assemble:\n')[1].split('  verify:\n')[0];
  const steps = assembly.split('\n      - ').slice(1);
  const restores = steps.filter((step) => step.includes('uses: actions/cache/restore@v4'));
  const gate = steps.find((step) =>
    step.startsWith('name: Require an exact packaging tools cache hit\n')
  )!;
  const field = (step: string, name: string) =>
    new RegExp(`^ {8}${name}: (.+)$`, 'm').exec(step)?.[1];
  const condition = field(restores[1], 'if')!;
  const hit = /^ {10}CACHE_HIT: (.+)$/m.exec(gate)![1];
  const command = gate.split('        run: |\n')[1].replace(/^ {10}/gm, '');
  type Result = { status: 'success' | 'failure'; hit?: string };
  function evaluate(expression: string, first?: string, second?: string) {
    return new Function(
      'first',
      'second',
      `return (${expression
        .replace(/^\$\{\{\s*|\s*\}\}$/g, '')
        .replace(/steps\.tools-restore\.outputs\.cache-hit/g, 'first')
        .replace(/steps\.tools-retry\.outputs\.cache-hit/g, 'second')});`
    )(first ?? '', second ?? '');
  }
  function simulate(first: Result, second: Result) {
    const calls = [first];
    if (evaluate(condition, first.hit)) calls.push(second);
    const last = calls.at(-1)!;
    const shell = spawnSync('bash', ['-e', '-c', command], {
      encoding: 'utf8',
      env: { ...process.env, CACHE_HIT: String(evaluate(hit, first.hit, calls[1]?.hit)) },
    });
    return { calls: calls.length, accepted: last.status === 'success' && shell.status === 0 };
  }

  it('allows one identical restore retry and requires an exact hit before every verifier', () => {
    expect(restores).toHaveLength(2);
    for (const step of restores) {
      expect(field(step, 'uses')).toBe('actions/cache/restore@v4');
      expect(step.split('        with:\n')[1]).toBe(
        '          path: ~/.tmt-packaging\n' +
          '          key: native-tools-${{ runner.os }}-${{ runner.arch }}-rust1.97-dist0.32.0-about0.9.2\n' +
          '          fail-on-cache-miss: true'
      );
    }
    expect(field(restores[0], 'continue-on-error')).toBe('true');
    expect(field(restores[1], 'continue-on-error')).toBeUndefined();
    expect(condition).toBe("steps.tools-restore.outputs.cache-hit != 'true'");
    expect(hit).toBe(
      "${{ steps.tools-restore.outputs.cache-hit == 'true' || steps.tools-retry.outputs.cache-hit == 'true' }}"
    );
    expect(assembly.indexOf('name: Require an exact packaging tools cache hit')).toBeLessThan(
      assembly.indexOf('name: Install verification dependencies')
    );
    expect(assembly).toContain('timeout-minutes: 15');
    expect(gate).not.toContain('continue-on-error');
  });

  it.each([
    ['first exact hit', { status: 'success', hit: 'true' }, { status: 'failure' }, 1, true],
    ['timeout then exact hit', { status: 'failure' }, { status: 'success', hit: 'true' }, 2, true],
    ['real miss twice', { status: 'failure' }, { status: 'failure' }, 2, false],
    ['persistent service error', { status: 'failure' }, { status: 'failure' }, 2, false],
    [
      'unavailable twice',
      { status: 'success', hit: 'false' },
      { status: 'success', hit: 'false' },
      2,
      false,
    ],
    ['missing outputs', { status: 'success' }, { status: 'success' }, 2, false],
    ['non-exact second hit', { status: 'failure' }, { status: 'success', hit: 'false' }, 2, false],
  ] as const)('keeps %s bounded and fail-closed', (_, first, second, calls, accepted) => {
    expect(simulate(first, second)).toEqual({ calls, accepted });
  });

  it('pins the incident counterpart and rejects removing it from the inventory', () => {
    const manifest = JSON.parse(read('.github/release-parity.json'));
    expect(manifest.incidents['2076']).toEqual({
      release: {
        workflow: 'native-release-prepare.yml',
        job: 'assemble',
        step: 'name:Require an exact packaging tools cache hit',
      },
      preMerge: [
        {
          workflow: 'ci.yml',
          job: 'unit-tests',
          selection: { kind: 'ci-scope', output: 'native_scope', value: 'full' },
          coverage: 'policy',
          tests: ['typescript/test/tooling/release-workflow.test.ts'],
        },
      ],
    });
    expect(() => checkReleaseParity(manifest, { read })).not.toThrow();
    delete manifest.incidents['2076'];
    expect(() => checkReleaseParity(manifest, { read })).toThrow('unmapped 2076');
  });
});

describe('signature-limited native install wiring (#1806)', () => {
  const upgrade = read('.github/workflows/native-release-upgrade-prove.yml');
  const script = read('scripts/install-native-verification-dependencies.sh');
  const selected = (source: string, owner: string, next: string) =>
    source.split(`  ${owner}:\n`)[1].split(`  ${next}:\n`)[0];
  function requireWiring(source: string, labels: string[]) {
    for (const label of labels) {
      const step = source.split('\n      - ').find((part) => part.startsWith(`name: ${label}\n`))!;
      expect(step).toContain('TARGET: ${{ matrix.target }}');
      expect(step).toContain('if [ "$TARGET" = x86_64-apple-darwin ]; then');
      expect(step).toContain(
        '"$GITHUB_WORKSPACE/scripts/install-native-verification-dependencies.sh"'
      );
      expect(step).toMatch(
        /\n          else\n            pnpm (?:--filter tmt --fail-if-no-match )?install --frozen-lockfile --ignore-scripts\n          fi/
      );
      expect(step).not.toContain('continue-on-error');
    }
  }
  it('shares both reviewed owners with rehearsal and keeps the package filter tied to the manifest', () => {
    expect(/^CLI_PACKAGE='([^']+)'$/m.exec(script)?.[1]).toBe(
      JSON.parse(read('typescript/package.json')).name
    );
    expect(script.match(/--filter "\$CLI_PACKAGE"/g)).toHaveLength(1);
    requireWiring(selected(prepare, 'verify', 'upgrade-fetch'), [
      'Install verification dependencies',
      'Install trusted CLI verification dependencies',
    ]);
    requireWiring(selected(upgrade, 'prove', 'conclude'), [
      'Install verification dependencies',
      'Install release-cut proof dependencies',
    ]);
    expect(read('.github/workflows/ci.yml')).toContain(
      'uses: ./.github/workflows/native-release-prepare.yml'
    );
    expect(prepare).toContain('uses: ./.github/workflows/native-release-upgrade-prove.yml');
  });
  it('rejects removing the controller or the x64-only boundary', () => {
    const owner = selected(upgrade, 'prove', 'conclude');
    expect(() =>
      requireWiring(
        owner.replaceAll(
          'scripts/install-native-verification-dependencies.sh',
          'scripts/missing.sh'
        ),
        ['Install verification dependencies']
      )
    ).toThrow();
    expect(() =>
      requireWiring(owner.replaceAll('x86_64-apple-darwin', 'aarch64-apple-darwin'), [
        'Install verification dependencies',
      ])
    ).toThrow();
  });
  it('pins runtime and inert pre-merge counterparts, and refuses an absent incident', () => {
    const manifest = JSON.parse(read('.github/release-parity.json'));
    expect(manifest.incidents['1806']).toEqual({
      release: {
        workflow: 'native-release-prepare.yml',
        job: 'verify',
        step: 'name:Install verification dependencies',
      },
      preMerge: [
        {
          workflow: 'ci.yml',
          job: 'release-rehearsal',
          selection: {
            kind: 'ci-scope',
            output: 'release_rehearsal',
            value: 'true',
            source: 'typescript/scripts/release-rehearsal.mjs',
          },
          coverage: 'runtime',
        },
        {
          workflow: 'ci.yml',
          job: 'unit-tests',
          selection: { kind: 'ci-scope', output: 'native_scope', value: 'full' },
          coverage: 'policy',
          tests: [
            'typescript/test/tooling/install-native-verification-dependencies.test.ts',
            'typescript/test/tooling/release-workflow.test.ts',
          ],
        },
      ],
    });
    expect(() => checkReleaseParity(manifest, { read })).not.toThrow();
    delete manifest.incidents['1806'];
    expect(() => checkReleaseParity(manifest, { read })).toThrow('unmapped 1806');
  });
});

describe('compiled CLI schema preparation order', () => {
  const blocks = (source: string) => ({
    build: source.split('  build:\n')[1].split('  assemble:\n')[0],
    assemble: source.split('  assemble:\n')[1].split('  verify:\n')[0],
    verify: source.split('  verify:\n')[1].split('  upgrade-fetch:\n')[0],
  });
  const trustedInstallName = 'name: Install trusted CLI verification dependencies';
  const trustedInstallCommand =
    'pnpm --filter tmt --fail-if-no-match install --frozen-lockfile --ignore-scripts';
  function requireTrustedCliPreparation(source: string) {
    const { verify } = blocks(source);
    const steps = verify.split(/\n      - /).slice(1);
    const installs = steps.filter((step) => step.startsWith(trustedInstallName + '\n'));
    expect(installs).toHaveLength(1);
    const install = installs[0];
    expect(install.split('\n')).toContain("        if: inputs.product == 'cli'");
    expect(install.split('\n')).toContain('        working-directory: typescript');
    expect(install).toContain(`            ${trustedInstallCommand}`);
    const setup = steps.findIndex((step) => step.startsWith('name: Set up Node.js and pnpm\n'));
    const preparation = steps.indexOf(install);
    const final = steps.findIndex((step) =>
      step.startsWith(
        'name: Execute final archive and bootstrap with the matching target process\n'
      )
    );
    expect(setup).toBeGreaterThanOrEqual(0);
    expect(preparation).toBeGreaterThan(setup);
    expect(final).toBeGreaterThan(preparation);
    expect(steps[final]).toContain(
      'node "$GITHUB_WORKSPACE/typescript/scripts/verify-native-artifact.mjs"'
    );
    const candidate = steps.find((step) =>
      step.startsWith('name: Install verification dependencies\n')
    );
    expect(candidate?.split('\n')).toContain(
      '        working-directory: release-source/typescript'
    );
    expect(candidate).toContain('            pnpm install --frozen-lockfile --ignore-scripts');
    const activation = read('.github/actions/setup-tooling/action.yml');
    expect(activation).toContain('default: 22.23.2');
    expect(activation).toContain('default: 10.33.0');
  }
  function requireSchemaOrder(source: string) {
    const { build, assemble, verify } = blocks(source);
    expect(build).toContain(
      "inputs.product == 'cli' && matrix.target == 'x86_64-apple-darwin' && 'x64'"
    );
    const capture = build.indexOf('name: Capture compiled CLI schema');
    expect(capture).toBeGreaterThan(
      build.indexOf('name: Verify plan, archive and binary versions')
    );
    expect(build.indexOf('scripts/run-native-verification.sh "$TARGET"', capture)).toBeGreaterThan(
      capture
    );
    expect(build.indexOf('native-application-schema.mjs" capture', capture)).toBeGreaterThan(
      capture
    );
    expect(capture).toBeLessThan(build.indexOf('name: Recheck version-only source'));
    expect(build).toContain('release-source/target/distrib/*-application-schema.json');
    const merge = assemble.indexOf('dist build --tag');
    const attach = assemble.indexOf('native-application-schema.mjs" assemble');
    expect(attach).toBeGreaterThan(merge);
    expect(attach).toBeLessThan(assemble.indexOf('generate-native-bootstrap.mjs'));
    expect(attach).toBeLessThan(assemble.indexOf('uses: actions/upload-artifact@v4'));
    expect(assemble).toContain('release-source/target/distrib/*-application-schema.json');
    expect(verify).toContain(
      'node "$GITHUB_WORKSPACE/typescript/scripts/verify-native-artifact.mjs"'
    );
    expect(verify).toContain('--schema-snapshot "$RUNNER_TEMP/release-version-state.json"');
    expect(verify).toContain('--schema-evidence "target/distrib/$TARGET-application-schema.json"');
    expect(verify).toContain('--source-root "$PWD"');
    expect(verify.indexOf('scripts/run-native-verification.sh "$TARGET"')).toBeLessThan(
      verify.indexOf('--schema-snapshot')
    );
    expect(verify).not.toContain('native-application-schema.mjs" assemble');
  }
  it('captures on matching hosts, inserts after cargo-dist merge and verifies without rewriting', () => {
    requireSchemaOrder(prepare);
    const source = read('typescript/scripts/verify-native-artifact.mjs');
    expect(source).toContain('verifyApplicationSchema({');
    expect(source).toContain('Final schema manifest changed during verification');
    expect(source.indexOf('verifyApplicationSchema({')).toBeLessThan(
      source.indexOf('await verifyNativeRuntime({')
    );
    expect(source.indexOf('Final schema manifest changed during verification')).toBeGreaterThan(
      source.indexOf('await verifyNativeRuntime({')
    );
    expect(source).toContain("'schema-snapshot': { type: 'string' }");
  });
  it('prepares pinned trusted tooling for CLI verification while retaining candidate dependencies', () => {
    requireTrustedCliPreparation(prepare);
  });
  it.each(['missing', 'wrong-checkout', 'late', 'condition', 'filter', 'frozen', 'scripts'])(
    'detects %s trusted CLI dependency preparation',
    (mutation) => {
      const { verify } = blocks(prepare);
      const step = verify
        .split(/\n      - /)
        .find((entry) => entry.startsWith(trustedInstallName + '\n'))!;
      const declaration = '      - ' + step;
      let changed: string;
      if (mutation === 'missing') changed = verify.replace(declaration, '');
      else if (mutation === 'late') {
        changed = verify
          .replace(declaration, '')
          .replace(
            '      - name: Recheck version-only source after this stage\n',
            declaration + '      - name: Recheck version-only source after this stage\n'
          );
      } else {
        const [before, after] = {
          'wrong-checkout': [
            'working-directory: typescript',
            'working-directory: release-source/typescript',
          ],
          condition: ["inputs.product == 'cli'", "inputs.product == 'colab'"],
          filter: ['--filter tmt', '--filter @tmt/colab-app'],
          frozen: ['--frozen-lockfile', '--no-frozen-lockfile'],
          scripts: ['--ignore-scripts', '--enable-scripts'],
        }[mutation]!;
        changed = verify.replace(declaration, declaration.replace(before, after));
      }
      expect(changed).not.toBe(verify);
      expect(() => requireTrustedCliPreparation(prepare.replace(verify, changed))).toThrow();
    }
  );
  it.each(['capture', 'carrier', 'evidence', 'snapshot', 'source', 'rosetta'])(
    'detects removal of the %s obligation',
    (guard) => {
      const changed = prepare.replace(
        {
          capture: 'name: Capture compiled CLI schema',
          carrier: 'native-application-schema.mjs" assemble',
          evidence: '--schema-evidence',
          snapshot: '--schema-snapshot',
          source: '--source-root "$PWD"',
          rosetta: "inputs.product == 'cli' && matrix.target == 'x86_64-apple-darwin' && 'x64'",
        }[guard]!,
        'REMOVED'
      );
      expect(changed).not.toBe(prepare);
      expect(() => requireSchemaOrder(changed)).toThrow();
    }
  );
  it('refuses a carrier inserted after bootstrap generation', () => {
    const line =
      'node "$GITHUB_WORKSPACE/typescript/scripts/native-application-schema.mjs" assemble';
    const changed = prepare
      .replace(line, 'REMOVED')
      .replace(
        'node typescript/scripts/generate-native-bootstrap.mjs',
        `node typescript/scripts/generate-native-bootstrap.mjs\n          ${line}`
      );
    expect(changed).not.toBe(prepare);
    expect(() => requireSchemaOrder(changed)).toThrow();
  });
});

describe('independent release-tag concurrency guard', () => {
  it('keys every publishing pipeline concurrency group on the allocated tag', () => {
    const directory = path.join(repository, '.github/workflows');
    for (const file of readdirSync(directory).filter((file) => /release.*\.yml$/.test(file))) {
      const workflow = read(`.github/workflows/${file}`);
      const groups = [...workflow.matchAll(/^\s*group:\s*(.+)$/gm)].map((match) => match[1]);
      if (file === 'release.yml') {
        expect(groups).toEqual(['release-cut']); // Only allocation is serialized.
      } else if (file === 'release-index-bootstrap.yml') {
        // Historical pointer writes are serialized; this workflow publishes no release.
        expect(groups).toEqual(['release-index-bootstrap']);
      } else if (
        !['project-release.yml', 'release-version-injection.yml', 'release-rehearsal.yml'].includes(
          file
        )
      ) {
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
  it('prepares the full locked Cargo inventory before Code quality offline metadata tests', () => {
    const quality = job(read('.github/workflows/ci.yml'), 'code-quality');
    const preparation = quality
      .split(/\n      - /)
      .find((step) => step.startsWith('name: Build the Rust TOML helper'))!;
    expect(preparation).toBeDefined();
    expect(quality).toContain('name: Code quality');
    expect(quality).toContain('timeout-minutes: 10');
    expect(preparation).toContain('RUSTUP_TOOLCHAIN: 1.97.0');
    const toolchain = preparation.indexOf('rustup toolchain install 1.97.0 --profile minimal');
    const fetch = preparation.indexOf('cargo fetch --locked --manifest-path rust/Cargo.toml');
    const build = preparation.indexOf(
      'cargo build --locked -p tmt-release-tool --bin release-version --manifest-path rust/Cargo.toml'
    );
    expect(toolchain).toBeGreaterThan(0);
    expect(fetch).toBeGreaterThan(toolchain);
    expect(build).toBeGreaterThan(fetch);
    expect(
      preparation
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.startsWith('cargo fetch'))
    ).toEqual(['cargo fetch --locked --manifest-path rust/Cargo.toml']);
    expect(preparation).not.toMatch(/continue-on-error|retry-command/);
    expect(quality.indexOf('name: Build the Rust TOML helper')).toBeLessThan(
      quality.indexOf('name: Run code quality checks')
    );
    expect(quality).toContain('test/tooling/ci-scope.test.ts');
  });
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
  it('keeps caller-selected x64 Node through Intel version injection and final verification', () => {
    const verify = job(prepare, 'verify');
    const setup = verify.indexOf('uses: ./.github/actions/setup-tooling');
    const injection = verify.indexOf('uses: ./.github/actions/inject-release-version');
    const finalVerification = verify.indexOf('name: Execute final archive and bootstrap');
    expect(setup).toBeGreaterThan(0);
    expect(injection).toBeGreaterThan(setup);
    expect(finalVerification).toBeGreaterThan(injection);
    expect(verify.slice(setup, injection)).toContain(
      "architecture: ${{ matrix.target == 'x86_64-apple-darwin' && 'x64' || '' }}"
    );
    const action = read('.github/actions/inject-release-version/action.yml');
    // Node selection belongs to the caller; another setup defaults back to runner arm64.
    expect(action).not.toMatch(/uses:.*(?:setup-node|setup-tooling)|GITHUB_PATH|export PATH=/);
    expect(verify.slice(injection, finalVerification)).not.toMatch(
      /uses:.*(?:setup-node|setup-tooling)/
    );
    expect(read('.github/actions/setup-tooling/action.yml')).toContain(
      'architecture: ${{ inputs.architecture }}'
    );
  });
  it('makes every injection caller supply pinned Node before either phase', () => {
    const callers: string[] = [];
    for (const file of readdirSync(path.join(repository, '.github/workflows')).filter((file) =>
      file.endsWith('.yml')
    )) {
      for (const [name, block] of jobs(read(`.github/workflows/${file}`))) {
        const injection = block.indexOf('uses: ./.github/actions/inject-release-version');
        if (injection === -1) continue;
        callers.push(`${file}:${name}`);
        const before = block.slice(0, injection);
        const setup = before
          .split(/\n      - /)
          .find((step) => step.includes('uses: ./.github/actions/setup-tooling'));
        expect(setup, `${file}:${name} needs caller Node setup`).toBeDefined();
        expect(setup, `${file}:${name} must set up Node for every product`).not.toMatch(
          /^        if:/m
        );
      }
    }
    expect(callers.sort()).toEqual([
      'ci.yml:native-process-tests',
      'native-release-prepare.yml:assemble',
      'native-release-prepare.yml:build',
      'native-release-prepare.yml:verify',
      'native-release-upgrade-prove.yml:prove',
      'release-version-injection.yml:injection',
    ]);
    expect(read('.github/actions/setup-tooling/action.yml')).toContain('default: 22.23.2');
  });
  it('keeps native injection on pinned PR heads and all four hosts, with no publishing privileges', () => {
    const injection = read('.github/workflows/release-version-injection.yml');
    expect(injection).toContain('github.event.pull_request.head.sha || github.sha');
    expect(injection).toContain('product: [cli, ops, remote, colab]');
    const products = /product: \[([^\]]+)\]/
      .exec(injection)?.[1]
      .split(',')
      .map((product) => product.trim());
    const map = JSON.parse(read('.github/components.json'));
    expect(products?.includes('ops')).toBe(
      !!map.components.ops && map.components.ops.release !== false
    );
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
    expect(run).toMatch(/options:\n {10}- cli\n {10}- ops/);
    const directory = mkdtempSync(path.join(os.tmpdir(), 'release-product-'));
    try {
      const gh = path.join(directory, 'gh');
      writeExecutable(
        gh,
        '#!/bin/sh\nprintf "unexpected release API call\\n" >&2\nexit 97\n',
        0o700
      );
      const search = `${directory}${path.delimiter}${process.env.PATH ?? ''}`;
      for (const [product, prepare] of ['office', 'squad'].flatMap((product) =>
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
      for (const product of ['cli', 'ops', 'driver-herdr', 'remote', 'colab']) {
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
  it('installs frozen Colab dependencies before building and moving the complete expected app', () => {
    const step = job(prepare, 'verify')
      .split(/\n      - /)
      .find((block) => block.startsWith('name: Build independent expected Colab app bytes\n'));
    expect(step).toBeDefined();
    expect(step).toContain("if: inputs.product == 'colab'");
    expect(step).toContain('working-directory: release-source/typescript');
    expect(step).toContain(
      'corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match install --frozen-lockfile --ignore-scripts\n' +
        '          corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match build\n' +
        '          mv ../extensions/tmt-colab/typescript/app/dist "$RUNNER_TEMP/colab-app"'
    );
  });

  it('builds expected Colab bytes from the candidate when the tooling checkout differs', () => {
    const step = job(prepare, 'verify')
      .split(/\n      - /)
      .find((block) => block.startsWith('name: Build independent expected Colab app bytes\n'))!;
    const directory = /^ {8}working-directory: (.+)$/m.exec(step)![1];
    const script = step.split('        run: |\n')[1].replace(/^ {10}/gm, '');
    const root = mkdtempSync(path.join(os.tmpdir(), 'tmt-colab-candidate-app-'));
    try {
      const search = path.join(root, 'bin');
      const temporary = path.join(root, 'temporary');
      mkdirSync(search);
      mkdirSync(temporary);
      for (const [checkout, marker] of [
        [root, 'tooling checkout'],
        [path.join(root, 'release-source'), 'candidate checkout'],
      ]) {
        mkdirSync(path.join(checkout, 'typescript'), { recursive: true });
        writeFileSync(path.join(checkout, 'typescript', 'app-source'), marker);
      }
      // Only the package manager is injected: the workflow selects its real cwd and moves bytes.
      writeExecutable(
        path.join(search, 'corepack'),
        '#!/bin/sh\nset -eu\n' +
          'case "$*" in\n' +
          '  *install*) ;;\n' +
          '  *build*) mkdir -p ../extensions/tmt-colab/typescript/app/dist; ' +
          'cp app-source ../extensions/tmt-colab/typescript/app/dist/index.html ;;\n' +
          '  *) exit 2 ;;\nesac\n'
      );
      const result = spawnSync('bash', ['-euo', 'pipefail', '-c', script], {
        cwd: path.join(root, directory),
        env: {
          ...process.env,
          PATH: `${search}${path.delimiter}${process.env.PATH}`,
          RUNNER_TEMP: temporary,
        },
        encoding: 'utf8',
        timeout: 10_000,
      });
      expect(result.error).toBeUndefined();
      expect(result.status, result.stderr).toBe(0);
      expect(readFileSync(path.join(temporary, 'colab-app/index.html'), 'utf8')).toBe(
        'candidate checkout'
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('builds Colab with frozen embedded assets and verifies outside the checkout fallback', () => {
    expect(run).toMatch(
      /options:\n {10}- cli\n {10}- ops\n {10}- driver-herdr\n {10}- remote\n {10}- colab/
    );
    expect(job(prepare, 'build')).toMatch(
      /- name: Set up Node.js and pnpm\n {8}uses: \.\/\.github\/actions\/setup-tooling/
    );
    const verify = job(prepare, 'verify');
    expect(verify).toContain(
      'corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match build'
    );
    expect(verify).toContain(
      'mv ../extensions/tmt-colab/typescript/app/dist "$RUNNER_TEMP/colab-app"'
    );
    expect(verify).toContain(
      '--archive "target/distrib/tmt-$PRODUCT-$TARGET.tar.gz" --target "$TARGET"'
    );
    expect(verify).toMatch(
      /if \[ "\$PRODUCT" = colab \]; then\n\s+verification_args\+=\(--app-dir "\$RUNNER_TEMP\/colab-app"\)\n\s+fi/
    );
    expect(verify).toContain(
      'node typescript/scripts/verify-native-artifact.mjs "${verification_args[@]}"'
    );
  });

  it('is only callable, and runs the pipeline of the draft it is given', () => {
    expect(bundle).toMatch(/^on:\n {2}workflow_call:/m);
    expect(bundle).not.toContain('workflow_dispatch');
    expect(prepare).toMatch(/^on:\n {2}workflow_call:/m);
    expect(prepare).not.toContain('workflow_dispatch');
    for (const name of ['build', 'assemble', 'verify']) {
      expect(job(prepare, name), name).toContain('ref: ${{ inputs.sha || github.sha }}');
    }
    expect(job(bundle, 'prepare')).toContain('sha: ${{ inputs.sha }}');
    // Tooling that talks to the release API comes from the workflow's own commit.
    for (const name of ['check', 'attach', 'record-failure']) {
      expect(job(bundle, name), name).not.toContain('ref: ${{ inputs.sha');
    }
  });

  it('builds a draft only while it has no bundle and no recorded failure, on main', () => {
    // The caller owns admission; the prepare workflow itself carries no draft or branch condition.
    const call = job(bundle, 'prepare');
    expect(call).toContain(
      "if: github.ref == 'refs/heads/main' && needs.check.outputs.todo == 'true'"
    );
    expect(call).toContain('needs: check');
    expect(call).toContain('uses: ./.github/workflows/native-release-prepare.yml');
    expect(job(prepare, 'build')).not.toMatch(/^ {4}(needs|if):/m);
    expect(job(prepare, 'assemble')).toContain('needs: build');
    expect(job(prepare, 'verify')).toContain('needs: assemble');
  });

  it('qualifies every artifact by the draft tag, so drafts of one run do not collide', () => {
    const names = [
      ...(prepare + bundle).matchAll(/^ {10}(?:name|pattern): (native-[^\n]+)$/gm),
    ].map(([, name]) => name);
    expect(names.length).toBeGreaterThanOrEqual(5);
    for (const name of names) expect(name, name).toMatch(/\$\{\{ inputs\.tag( \|\| 'main')? \}\}/);
  });

  it('asserts that the draft tag is the version its commit declares', () => {
    expect(job(prepare, 'assemble')).toContain('RELEASE_TAG: ${{ inputs.tag }}');
    expect(job(prepare, 'assemble')).toContain(
      'is not the $tag that the injected checkout declares'
    );
  });

  it('attaches only after every verify job passed, and records a failure but not a cancellation', () => {
    const attach = job(bundle, 'attach');
    expect(attach).toContain('needs: [check, prepare]');
    expect(attach).toContain(
      "if: inputs.tag != '' && needs.check.outputs.todo == 'true' && needs.prepare.result == 'success'"
    );
    expect(attach).toContain('release-draft-assets.mjs attach');
    const failure = job(bundle, 'record-failure');
    expect(failure).toContain('needs: [check, prepare, attach]');
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
    const build = job(prepare, 'build');
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
    expect(release).toContain('options: [all, cli, ops, driver-herdr, remote, colab]');
    expect(release).toContain('VERSION: ${{ inputs.version }}');
    expect(release).toContain('driver-herdr');
    expect(release).toContain('release-cut-plan.json');
    expect(release).toContain('if: always()');
  });
});

describe('release upgrade proof (native-release-upgrade.yml)', () => {
  const upgrade = read('.github/workflows/native-release-upgrade.yml');
  const proveWf = read('.github/workflows/native-release-upgrade-prove.yml');
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
    expect(upgrade).toMatch(/type: choice\n {8}options:\n {10}- cli\n {10}- office\n {10}- ops/);
    // The publication run reads the outcome and the reason, whatever the run's own result is.
    expect(upgrade).toMatch(
      /^ {4}outputs:\n {6}outcome:\n(?: {8}[^\n]*\n)* {8}value: \$\{\{ jobs\.fetch\.outputs\.outcome \}\}\n {6}reason:\n(?: {8}[^\n]*\n)* {8}value: \$\{\{ jobs\.prove\.outputs\.reason \}\}\n/m
    );
  });

  it('proves on the same four hosts as the bundle verification', () => {
    expect(targets(proveWf)).toHaveLength(4);
    expect(targets(proveWf)).toEqual(targets(prepare));
  });

  it('only reads releases: write access is for seeing draft assets, and nothing is written', () => {
    expect(upgrade).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(upgrade.match(/^ {6}contents: write$/gm)).toHaveLength(1);
    expect(job(upgrade, 'fetch')).toMatch(/^ {6}contents: write$/m);
    expect(job(upgrade, 'prove')).toMatch(/^ {4}permissions:\n {6}contents: read\n/m);
    // The proof stages hold no write permission at all.
    expect(proveWf).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(proveWf).not.toMatch(/: write$/m);
    for (const text of [upgrade, proveWf]) {
      expect(text).not.toMatch(/actions: write|pull-requests:|id-token:|packages:/);
      expect(text).not.toMatch(
        /gh release (create|edit|upload|delete)|draft=false|--method|gh workflow run/
      );
    }
  });

  it("fetches with the write token on main's code and proves the release's code read-only", () => {
    const fetch = job(upgrade, 'fetch');
    const prove = job(proveWf, 'prove');
    // The job that holds the write token runs this repository's main, never the release commit.
    expect(fetch).toContain("if: github.ref == 'refs/heads/main'");
    expect(fetch).not.toMatch(/^ {10}ref:/m);
    expect(fetch).toContain('release-upgrade.mjs fetch --product "$PRODUCT" --tag "$RELEASE_TAG"');
    expect(fetch).not.toContain('pnpm');
    // The job that runs the release commit's scripts has no token at all.
    expect(prove).toContain('ref: ${{ inputs.sha }}');
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
    const prove = job(proveWf, 'prove');
    const conclude = job(proveWf, 'conclude');
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
      'reason: ${{ steps.failure.outputs.reason || inputs.fetch-reason }}'
    );
  });

  it('runs the proof from the commit of the release, and says when that commit predates it', () => {
    const fetch = job(upgrade, 'fetch');
    const prove = job(proveWf, 'prove');
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
    expect(prove).toContain("if: inputs.outcome == 'proved'");
    expect(prove).not.toContain('it predates');
    const conclude = job(proveWf, 'conclude');
    expect(conclude).toContain("if: inputs.outcome == 'predates'");
    expect(conclude).toContain('exit 1');
    expect(job(upgrade, 'prove')).toContain(
      "if: ${{ !cancelled() && needs.fetch.result == 'success' }}"
    );
    expect(prove).toContain('release-upgrade.mjs" prove --product "$PRODUCT" --tag "$RELEASE_TAG"');
    expect(prove).toContain('skill=skills/tmt/SKILL.md');
    expect(prove).toContain('--skill "$skill"');
    // The macOS toolchain lookup is warmed before the archives run.
    expect(prove.indexOf('warm-xcrun')).toBeGreaterThan(0);
    expect(prove.indexOf('warm-xcrun')).toBeLessThan(prove.indexOf('release-upgrade.mjs" prove'));
  });

  it('requires CLI adapter acceptance after the existing proof on every host, with bounded compilation and a Darwin adapter cache', () => {
    const prove = job(proveWf, 'prove');
    expect(prove).toContain('timeout-minutes: 13');
    expect(prove).toMatch(/name: Install Rust for version-only resolution and adapter acceptance/);
    expect(prove).toMatch(
      /name: Restore Rust dependencies for version-only resolution and adapter acceptance/
    );
    expect(prove).toContain(
      "shared-key: ${{ inputs.product == 'cli' && runner.os == 'macOS' && 'native-upgrade-adapter' || 'native-rust' }}"
    );
    expect(prove).toContain(
      "save-if: ${{ github.ref == 'refs/heads/main' && inputs.product == 'cli' && runner.os == 'macOS' }}"
    );
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

describe('shared prepare pipeline and release rehearsal', () => {
  const rehearsal = read('.github/workflows/release-rehearsal.yml');
  it('keeps the prepare workflow read-only: callable, no secrets, no writes, no publication', () => {
    expect(prepare).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(prepare).not.toMatch(
      /^ {2,6}(contents|actions|issues|pull-requests|checks|id-token): write$/m
    );
    expect(prepare).not.toMatch(
      /secrets\.|environment:|gh release|release-publish|release-draft-assets/
    );
    expect([...jobs(prepare).keys()]).toEqual([
      'build',
      'assemble',
      'verify',
      'upgrade-fetch',
      'upgrade',
      'gates-dry',
    ]);
  });
  it('dry-runs the publication gates only for rehearsals, read-only, with the commit gate on main only', () => {
    const dry = job(prepare, 'gates-dry');
    expect(dry).toContain('if: inputs.upgrade');
    expect(dry).toContain('contents: read');
    expect(dry).toContain('publication-gates.mjs dry');
    expect(dry).toContain("ON_MAIN: ${{ github.ref == 'refs/heads/main' }}");
    expect(dry).toContain('CANDIDATE_TAG: ${{ needs.assemble.outputs.tag }}');
    expect(dry).not.toMatch(/\b(early|finish)\b|release-hold|--rerun/);
  });
  it('lets both the draft pipeline and the rehearsal call it with an exact source commit', () => {
    expect(job(bundle, 'prepare')).toContain(
      'uses: ./.github/workflows/native-release-prepare.yml'
    );
    expect(job(bundle, 'prepare')).toContain('tag: ${{ inputs.tag }}');
    const ci = read('.github/workflows/ci.yml');
    for (const [name, call, sha] of [
      ['nightly', job(rehearsal, 'prepare'), 'sha: ${{ github.sha }}'],
      [
        'pull request',
        job(ci, 'release-rehearsal'),
        'sha: ${{ github.event.pull_request.head.sha }}',
      ],
    ]) {
      expect(call, name).toContain('uses: ./.github/workflows/native-release-prepare.yml');
      expect(call, name).toContain(sha);
      // No tag: a rehearsal never names, allocates or attaches to a draft.
      expect(call, name).not.toContain('tag:');
      expect(call, name).toContain('contents: read');
    }
  });
  it('has no privileges, secrets or publication path in the rehearsal workflow', () => {
    expect(rehearsal).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(rehearsal).not.toMatch(/: write$/m);
    expect(rehearsal).not.toMatch(
      /secrets\.|environment:|gh release|workflow_run|repository_dispatch/
    );
    expect(rehearsal).not.toMatch(/native-release\.yml|native-release-bundle\.yml#|release\.yml/);
    expect([...jobs(rehearsal).keys()]).toEqual(['plan', 'prepare']);
    expect(rehearsal).not.toContain('merge_group');
    expect(rehearsal).not.toContain('pull_request');
    expect(rehearsal).toMatch(
      /^on:\n {2}schedule:\n {4}- cron: '[^']+'\n {2}workflow_dispatch:\n/m
    );
    // Nightly drift detection covers every active product, not a selection.
    expect(job(rehearsal, 'plan')).toContain('release-rehearsal.mjs all');
    expect(job(rehearsal, 'prepare')).toContain(
      'product: ${{ fromJSON(needs.plan.outputs.products) }}'
    );
  });
  it('writes the Rust cache from main only, so a rehearsal restores without saving', () => {
    expect(job(prepare, 'build')).toContain("save-if: ${{ github.ref == 'refs/heads/main' }}");
  });
});

describe('rehearsal upgrade proof and the publishing upgrade call', () => {
  const upgradeWf = read('.github/workflows/native-release-upgrade.yml');
  const proveWf = read('.github/workflows/native-release-upgrade-prove.yml');
  const ci = read('.github/workflows/ci.yml');
  const rehearsal = read('.github/workflows/release-rehearsal.yml');

  it('lets a pull request reach only the read-only local fetch, never the write-token fetch', () => {
    // The write-token fetch lives in native-release-upgrade.yml, called only by the publication bundle.
    const callers = readdirSync(path.join(repository, '.github/workflows')).filter((file) =>
      read(`.github/workflows/${file}`).includes(
        'uses: ./.github/workflows/native-release-upgrade.yml'
      )
    );
    expect(callers).toEqual(['native-release-bundle.yml']);
    expect(job(upgradeWf, 'fetch')).toContain("if: github.ref == 'refs/heads/main'");
    expect(job(upgradeWf, 'fetch')).toMatch(/^ {6}contents: write$/m);
    // Everything a pull request can call (prepare, the proof stages) holds no write permission.
    for (const [name, text] of [
      ['prepare', prepare],
      ['proof stages', proveWf],
    ]) {
      expect(text, name).not.toMatch(/: write$/m);
      expect(text, name).not.toContain('uses: ./.github/workflows/native-release-upgrade.yml');
    }
    const local = job(prepare, 'upgrade-fetch');
    expect(local).toContain('if: inputs.upgrade');
    expect(local).toMatch(/^ {4}permissions:\n {6}contents: read\n/m);
    expect(local).toContain('--candidate-directory "$RUNNER_TEMP/candidate"');
    expect(local).toContain(
      "native-release-${{ inputs.product }}-bundle-${{ inputs.tag || 'main' }}"
    );
    expect(local).not.toMatch(/secrets\./);
    expect(job(prepare, 'upgrade')).toContain(
      'uses: ./.github/workflows/native-release-upgrade-prove.yml'
    );
  });

  it('enables the upgrade proof only for rehearsal callers, never the publication path', () => {
    expect(job(ci, 'release-rehearsal')).toContain('upgrade: true');
    expect(job(rehearsal, 'prepare')).toContain('upgrade: true');
    expect(job(bundle, 'prepare')).not.toContain('upgrade');
    expect(run).not.toContain('upgrade: true');
    expect(prepare).toMatch(
      /upgrade:\n {8}description: [^\n]+\n {8}required: false\n {8}default: false\n {8}type: boolean/
    );
    // The rehearsal's tag is the synthetic announcement tag of its own assembled manifest.
    expect(job(prepare, 'assemble')).toContain('tag: ${{ steps.version.outputs.tag }}');
    expect(job(prepare, 'upgrade')).toContain('tag: ${{ needs.assemble.outputs.tag }}');
  });

  // The publication path's upgrade call after moving prove and conclude into the shared stages:
  // evaluate the real expressions against the pre-#1666 behavior for every outcome.
  const evaluate = (expression: string, context: Record<string, unknown>) =>
    new Function(
      'ctx',
      `const cancelled = () => ctx.cancelled; return (${expression
        .replace(/^\$\{\{\s*|\s*\}\}$/g, '')
        .replace(
          /\bneeds\.([\w-]+)\.(result|outputs\.\w+)/g,
          (_, name, path) => `ctx.needs['${name}'].${path}`
        )
        .replace(/\b(inputs|github|runner)\.([\w-]+)/g, "ctx['$1']['$2']")}) `
    )({ cancelled: false, ...context });
  const ifOf = (text: string) => /^ {4}if: (.+)$/m.exec(text)?.[1] ?? '';

  it('seeds adapter dependencies only from main Darwin CLI proofs without adding compilation', () => {
    const prove = job(proveWf, 'prove');
    const caches = prove
      .split('\n      - ')
      .filter((step) =>
        step.startsWith(
          'name: Restore Rust dependencies for version-only resolution and adapter acceptance\n'
        )
      );
    expect(caches).toHaveLength(1);
    const cache = caches[0];
    expect(cache).toContain('uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6');
    expect(cache).toContain('workspaces: release-source/rust');
    expect(cache).toContain('CARGO_PROFILE_DEV_DEBUG: 0');
    expect(cache).toContain('CARGO_INCREMENTAL: 0');
    expect(prove).toContain('RUSTUP_TOOLCHAIN: 1.97.0');
    expect(prove).toContain('run: rustup toolchain install 1.97.0 --profile minimal');
    expect(prove).not.toMatch(
      /add-rust-environment-hash-key:|cache-on-failure:|cache-workspace-crates:/
    );
    const key = /^ {10}shared-key: (.+)$/m.exec(cache)?.[1];
    const save = /^ {10}save-if: (.+)$/m.exec(cache)?.[1];
    expect(key).toBeDefined();
    expect(save).toBeDefined();
    for (const ref of [
      'refs/heads/main',
      'refs/pull/42/merge',
      'refs/heads/gh-readonly-queue/main/pr-42',
      'refs/tags/v5.0.0-alpha.1',
      'refs/heads/feature',
    ]) {
      for (const product of ['cli', 'ops', 'remote', 'colab']) {
        for (const os of ['macOS', 'Linux']) {
          const context = { github: { ref }, inputs: { product }, runner: { os } };
          const adapter = product === 'cli' && os === 'macOS';
          const label = `${ref}/${product}/${os}`;
          expect(evaluate(key!, context), label).toBe(
            adapter ? 'native-upgrade-adapter' : 'native-rust'
          );
          expect(evaluate(save!, context), label).toBe(adapter && ref === 'refs/heads/main');
        }
      }
    }
    expect(prove.match(/name: Prove the real-archive CLI upgrade adapter/g)).toHaveLength(1);
    expect(prove.match(/release-upgrade\.mjs" acceptance --product cli/g)).toHaveLength(1);
    expect(prove).not.toContain('cargo test');
  });

  it('keeps the same jobs running and the same outputs consumed on the publication path', () => {
    const call = ifOf(job(upgradeWf, 'prove'));
    const proveIf = ifOf(job(proveWf, 'prove'));
    const concludeIf = ifOf(job(proveWf, 'conclude'));
    for (const fetchResult of ['success', 'failure', 'skipped', 'cancelled']) {
      for (const cancelled of [false, true]) {
        for (const outcome of ['proved', 'nothing', 'predates']) {
          const needs = { fetch: { result: fetchResult, outputs: { outcome } } };
          const callRuns = evaluate(call, { cancelled, needs });
          // Before: prove ran when fetch succeeded and proved; conclude ran unless cancelled after a
          // successful fetch. After: the call runs under the old conclude condition, and the stages
          // apply the old prove condition to the outcome handed over.
          const oldConclude = !cancelled && fetchResult === 'success';
          expect(callRuns, `${fetchResult}/${cancelled}/${outcome}`).toBe(oldConclude);
          if (!callRuns) continue;
          const inputs = { outcome };
          expect(evaluate(proveIf, { inputs })).toBe(outcome === 'proved');
          // Before and after, a prove failure or skip never stops conclude from running.
          expect(evaluate(concludeIf, { cancelled, inputs })).toBe(!cancelled);
        }
      }
    }
    // Every value the stages consume is the one fetch produced, and the old consumers are gone.
    const withBlock = job(upgradeWf, 'prove');
    for (const [key, value] of [
      ['sha', 'needs.fetch.outputs.sha'],
      ['outcome', 'needs.fetch.outputs.outcome'],
      ['fetch-reason', 'needs.fetch.outputs.reason'],
      ['current-tooling', 'inputs.current-tooling'],
      ['product', 'inputs.product'],
      ['tag', 'inputs.tag'],
    ])
      expect(withBlock).toContain(`${key}: \${{ ${value} }}`);
    expect(upgradeWf).toMatch(/value: \$\{\{ jobs\.fetch\.outputs\.outcome \}\}/);
    expect(upgradeWf).toMatch(/value: \$\{\{ jobs\.prove\.outputs\.reason \}\}/);
    expect([...jobs(upgradeWf).keys()]).toEqual(['fetch', 'prove']);
    expect([...jobs(proveWf).keys()]).toEqual(['prove', 'conclude']);
    // The stages choose their reason exactly as conclude did: a host failure first, else fetch's.
    expect(job(proveWf, 'conclude')).toContain(
      'reason: ${{ steps.failure.outputs.reason || inputs.fetch-reason }}'
    );
  });
});

describe('pull-request release rehearsal in ci.yml', () => {
  const ci = read('.github/workflows/ci.yml');
  it('rehearses selected products on pull requests only, never in the merge group', () => {
    const select = ci
      .split('      - name: Select the release rehearsal products\n')[1]
      .split('\n\n')[0];
    expect(select).toContain("if: github.event_name == 'pull_request'");
    expect(select).toContain('release-rehearsal.mjs select "$BASE_SHA" "$HEAD_SHA"');
    expect(select).not.toContain('merge_group');
    const rehearse = job(ci, 'release-rehearsal');
    expect(rehearse).toContain(
      "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.release_rehearsal == 'true'"
    );
    expect(rehearse).toContain(
      'product: ${{ fromJSON(needs.changes.outputs.release_rehearsal_products) }}'
    );
    // Outputs default to "no rehearsal" for every event that skips the selection step.
    expect(ci).toContain(
      "release_rehearsal: ${{ steps.rehearsal.outputs.release_rehearsal || 'false' }}"
    );
    expect(ci).toContain("|| '[]' }}");
  });
  it('fails the aggregate native gate for a selected rehearsal that is missing, failed or skipped', () => {
    const gate = job(ci, 'native-install-gate');
    expect(gate).toMatch(/^ {8}release-rehearsal,$/m);
    const step = gate.split('      - name: Require the selected release rehearsal\n')[1];
    expect(step).toContain('REHEARSAL_SELECTED: ${{ needs.changes.outputs.release_rehearsal }}');
    expect(step).toContain('REHEARSAL_RESULT: ${{ needs.release-rehearsal.result }}');
    expect(step).toContain('ci-scope.mjs gate "$REHEARSAL_SELECTED" "$REHEARSAL_RESULT"');
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
    for (const name of ['prepare', 'attach']) {
      expect(job(bundle, name), name).not.toMatch(/needs:[^\n]*\b(finish|publish|published)\b/);
    }
  });
});

describe('publication (native-release-bundle.yml)', () => {
  const finish = job(bundle, 'finish');
  const publish = job(bundle, 'publish');
  const published = job(bundle, 'published');
  const indexJob = job(run, 'release-index');

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
    expect(published).toContain('persist-credentials: false');
    expect(published).not.toContain('environment: release');
    expect(published).toContain(
      'release-publish.mjs verify --product "$PRODUCT" --tag "$RELEASE_TAG"'
    );
    expect(published).toContain('--directory "$RUNNER_TEMP/published"');
    expect(published).toContain(
      '--run-url "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"'
    );
  });
  it('indexes only verified publication after the bundle completes smoke and Project dispatch', () => {
    expect(bundle).toContain('value: ${{ jobs.published.outputs.published }}');
    expect(published).toContain('published: ${{ steps.verify.outputs.published }}');
    expect(published.indexOf('echo "published=true" >> "$GITHUB_OUTPUT"')).toBeGreaterThan(
      published.indexOf('release-publish.mjs verify')
    );
    expect(indexJob).toContain('needs: [plan, bundle]');
    expect(indexJob).toContain('matrix: ${{ fromJSON(needs.plan.outputs.matrix) }}');
    expect(indexJob).toContain('RELEASE_TAG: ${{ matrix.tag }}');
    expect(indexJob).toContain('timeout-minutes: 15');
    expect(indexJob).toMatch(
      /^ {4}permissions:\n {6}contents: read\n {6}issues: write\n {4}steps:/m
    );
    const condition = /^ {4}if: \$\{\{ (.+) \}\}$/m.exec(indexJob)![1];
    const selected = (plan: string, bundleResult: string, publication: string) =>
      new Function(
        'plan',
        'bundleResult',
        'publication',
        `return (${condition
          .replace(/!cancelled\(\)/g, 'true')
          .replace(/needs\.plan\.result/g, 'plan')
          .replace(/needs\.bundle\.result/g, 'bundleResult')
          .replace(/needs\.bundle\.outputs\.published/g, 'publication')});`
      )(plan, bundleResult, publication);
    expect(selected('success', 'success', 'true')).toBe(true);
    for (const [plan, result, publication] of [
      ['success', 'success', ''], // held/tagless: the Published verifier never ran
      ['success', 'failure', 'true'],
      ['failure', 'skipped', ''],
    ])
      expect(selected(plan, result, publication)).toBe(false);
    expect(job(bundle, 'smoke')).toContain('needs: published');
    expect(job(bundle, 'project-release')).toContain('needs: [published, smoke]');
    expect(bundle).not.toContain('release-publish.mjs index');
  });

  it('fails visibly on a non-main ref or missing App credentials before token creation', () => {
    const admission = indexJob.slice(
      indexJob.indexOf('name: Require release index App credentials'),
      indexJob.indexOf('name: Create the release index App token')
    );
    const command = admission
      .split('        run: |\n')[1]
      .split('\n      - ')[0]
      .replace(/^ {10}/gm, '');
    for (const [ref, available, status] of [
      ['refs/heads/main', 'true', 0],
      ['refs/heads/main', 'false', 1],
      ['refs/heads/topic', 'true', 1],
    ] as const) {
      const result = spawnSync('bash', ['-e', '-c', command], {
        encoding: 'utf8',
        env: { ...process.env, GITHUB_REF: ref, HAS_APP_CREDENTIALS: available },
      });
      expect(result.status).toBe(status);
    }
  });

  it('creates a minimum-scope App token only after full verification, and reuses its download directory', () => {
    const verify = indexJob.indexOf('release-publish.mjs verify');
    const token = indexJob.indexOf(
      'actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1'
    );
    const writer = indexJob.indexOf('release-publish.mjs index');
    const admission = indexJob.indexOf('name: Require release index App credentials');
    expect(admission).toBeGreaterThan(verify);
    expect(admission).toBeLessThan(token);
    const credentials = indexJob.slice(
      admission,
      indexJob.indexOf('name: Create the release index App token')
    );
    expect(credentials).toContain(`if [ "$GITHUB_REF" != refs/heads/main ]; then
            echo 'Release index writer requires refs/heads/main.' >&2
            exit 1
          fi`);
    expect(credentials).not.toMatch(/^\s+if:/m);
    expect(indexJob).not.toMatch(/if:.*github\.ref/);
    expect(indexJob).toContain('environment: release');
    expect(indexJob).not.toMatch(/^ {10}ref:/m);
    expect(verify).toBeGreaterThan(0);
    expect(token).toBeGreaterThan(verify);
    expect(writer).toBeGreaterThan(token);
    expect(indexJob).toContain('permission-contents: write');
    expect(indexJob).not.toMatch(/permission-(issues|pull-requests|organization-projects):/);
    expect(indexJob).toContain('GH_TOKEN: ${{ steps.index-app.outputs.token }}');
    expect([...indexJob.matchAll(/--directory "\$RUNNER_TEMP\/release-index"/g)]).toHaveLength(2);
    expect(indexJob).not.toMatch(/continue-on-error|if:.*always|git push|--force/);
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
      /type: choice\n {8}options:\n {10}- cli\n {10}- office\n {10}- ops/
    );
  });

  it('installs on the same four hosts as the upgrade proof', () => {
    expect(targets(smokeWorkflow)).toHaveLength(4);
    expect(targets(smokeWorkflow)).toEqual(
      targets(read('.github/workflows/native-release-upgrade-prove.yml'))
    );
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
    expect(job(upgrade, 'prove')).toContain('current-tooling: ${{ inputs.current-tooling }}');
    const prove = job(read('.github/workflows/native-release-upgrade-prove.yml'), 'prove');
    expect(prove).toContain('ref: ${{ github.sha }}');
    expect(prove).toContain('path: release-source');
    expect(prove).toContain('set -- --source-root "$GITHUB_WORKSPACE/release-source"');
    expect(prove).toContain('skill="$GITHUB_WORKSPACE/release-source/skills/tmt/SKILL.md"');
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

describe('release environment App secrets stay in top-level workflows', () => {
  const secretReaders = (workflows: Record<string, string>) => {
    const reusable = new Set<string>();
    for (const [file, text] of Object.entries(workflows)) {
      if (/\bworkflow_call(?:\s*:|(?=[,\]]))/.test(text)) reusable.add(file);
      for (const [, target] of text.matchAll(/uses:\s*\.\/\.github\/workflows\/([\w-]+\.yml)/g))
        reusable.add(target);
    }
    return [...reusable]
      .filter((file) => {
        const text = workflows[file];
        expect(text, `Missing reusable workflow ${file}`).toBeDefined();
        return /secrets(?:\.RELEASE_APP_(?:ID|PRIVATE_KEY)\b|\[['"]RELEASE_APP_(?:ID|PRIVATE_KEY)['"]\])/.test(
          text
            .split('\n')
            .filter((line) => !/^\s*#/.test(line))
            .join('\n')
        );
      })
      .sort();
  };

  it('refuses App-secret reads in all local reusable workflows, including nested and dormant files', () => {
    const workflows = Object.fromEntries(
      readdirSync(path.join(repository, '.github/workflows'))
        .filter((file) => file.endsWith('.yml'))
        .map((file) => [file, read(`.github/workflows/${file}`)])
    );
    expect(secretReaders(workflows)).toEqual([]);
    expect(bundle).not.toMatch(/secrets[.\[][^\n]*RELEASE_APP_/);
    expect(run).toContain('secrets.RELEASE_APP_ID');
    expect(run).toContain('secrets.RELEASE_APP_PRIVATE_KEY');
  });

  it('detects dotted and bracket secret access without rejecting a top-level environment owner', () => {
    const workflows = {
      'owner.yml':
        'on: [workflow_dispatch]\njobs:\n  child:\n    uses: ./.github/workflows/child.yml\n  writer:\n    env: ${{ secrets.RELEASE_APP_ID }}',
      'child.yml':
        'on:\n  workflow_call:\njobs:\n  nested:\n    uses: ./.github/workflows/nested.yml',
      'nested.yml': 'on:\n  workflow_call:\njobs:\n  reader:\n    run: true',
      'dormant.yml': 'on:\n  workflow_call:\njobs:\n  reader:\n    run: true',
    };
    expect(secretReaders(workflows)).toEqual([]);
    expect(
      secretReaders({
        ...workflows,
        'nested.yml': workflows['nested.yml'] + '\n    env: ${{ secrets.RELEASE_APP_ID }}',
      })
    ).toEqual(['nested.yml']);
    expect(
      secretReaders({
        ...workflows,
        'dormant.yml':
          workflows['dormant.yml'] + "\n    env: ${{ secrets['RELEASE_APP_PRIVATE_KEY'] }}",
      })
    ).toEqual(['dormant.yml']);
  });
});

describe('owner-authorized release index bootstrap', () => {
  const bootstrap = read('.github/workflows/release-index-bootstrap.yml');
  const backfill = job(bootstrap, 'backfill');
  const steps = backfill.split('\n      - ').slice(1);
  const command = (name: string) =>
    steps
      .find((step) => step.startsWith(`name: ${name}\n`))!
      .split('        run: |\n')[1]
      .replace(/^ {10}/gm, '');

  it('requires main visibly and keeps inventory admission ahead of the minimum-scope App', () => {
    expect(bootstrap).toMatch(/^on:\n {2}workflow_dispatch:/m);
    expect(bootstrap).not.toMatch(/workflow_call|pull_request|^ {2}(push|schedule):/m);
    expect(bootstrap).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(bootstrap).toContain('group: release-index-bootstrap\n  cancel-in-progress: false');
    expect(backfill).toContain('environment: release');
    expect(backfill).toContain('timeout-minutes: 15');
    expect(backfill).toContain('persist-credentials: false');
    expect(backfill).toContain('node-version: 22.23.2');
    expect(backfill).not.toMatch(/^\s+(if|ref):|continue-on-error|--force|git push/m);
    for (const [ref, status] of [
      ['refs/heads/main', 0],
      ['refs/heads/topic', 1],
    ] as const) {
      const result = spawnSync('bash', ['-e', '-c', command('Require the main ref')], {
        encoding: 'utf8',
        env: { ...process.env, GITHUB_REF: ref },
      });
      expect(result.status).toBe(status);
      if (status) expect(result.stderr).toContain('requires refs/heads/main.');
    }
    const order = [
      'Require the main ref',
      'backfill-inventory',
      'Require the reviewed inventory tuple',
      'Require release index App credentials',
      'actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1',
      'Backfill the verified historical release',
    ].map((part) => backfill.indexOf(part));
    expect(
      order.every((value, index) => value >= 0 && (index === 0 || value > order[index - 1]))
    ).toBe(true);
    expect(backfill).toContain('permission-contents: write');
    expect(backfill).not.toMatch(/permission-(issues|actions|pull-requests):/);
    expect(backfill).toContain('GH_TOKEN: ${{ github.token }}');
    expect(backfill).toContain('GH_TOKEN: ${{ steps.index-app.outputs.token }}');
    for (const field of ['product', 'channel', 'tag', 'release-id', 'source-sha']) {
      expect(backfill).toContain(`steps.reviewed.outputs.${field}`);
    }
    expect(backfill).toContain(
      '--tag "$RELEASE_TAG" --release-id "$RELEASE_ID" --source-sha "$SOURCE_SHA"'
    );
    expect(backfill).toContain('--directory "$RUNNER_TEMP/release-index-backfill"');
    for (const [available, status] of [
      ['true', 0],
      ['false', 1],
    ] as const) {
      expect(
        spawnSync('bash', ['-e', '-c', command('Require release index App credentials')], {
          env: { ...process.env, HAS_APP_CREDENTIALS: available },
        }).status
      ).toBe(status);
    }
  });

  function checkTuple(inventory: unknown, overrides: Record<string, string> = {}, missing = false) {
    const directory = mkdtempSync(path.join(os.tmpdir(), 'release-index-tuple-'));
    const output = path.join(directory, 'output');
    try {
      if (!missing)
        writeFileSync(
          path.join(directory, 'release-index-inventory.json'),
          JSON.stringify(inventory)
        );
      const result = spawnSync(
        'bash',
        ['-e', '-c', command('Require the reviewed inventory tuple')],
        {
          cwd: repository,
          encoding: 'utf8',
          env: {
            ...process.env,
            PRODUCT: 'cli',
            CHANNEL: 'alpha',
            RELEASE_TAG: 'v5.0.0-alpha.1',
            RELEASE_ID: '123',
            SOURCE_SHA: 'a'.repeat(40),
            RUNNER_TEMP: directory,
            GITHUB_OUTPUT: output,
            ...overrides,
          },
        }
      );
      return {
        status: result.status,
        output: readdirSync(directory).includes('output') ? readFileSync(output, 'utf8') : '',
      };
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }
  const inventory = {
    product: 'cli',
    channel: 'alpha',
    tag: 'v5.0.0-alpha.1',
    releaseId: 123,
    sourceSha: 'a'.repeat(40),
  };

  it('captures the exact reviewed tuple for every published product and channel', () => {
    for (const [product, prefix] of [
      ['cli', 'v'],
      ['ops', 'tmt-ops-v'],
      ['remote', 'tmt-remote-v'],
      ['colab', 'tmt-colab-v'],
      ['driver-herdr', 'tmt-driver-herdr-v'],
    ]) {
      for (const channel of ['alpha', 'stable']) {
        const tag = `${prefix}5.0.0${channel === 'alpha' ? '-alpha.1' : ''}`;
        expect(bootstrap).toContain(`          - ${product}\n`);
        expect(
          checkTuple(
            { ...inventory, product, channel, tag },
            { PRODUCT: product, CHANNEL: channel, RELEASE_TAG: tag }
          )
        ).toEqual({
          status: 0,
          output: `product=${product}\nchannel=${channel}\ntag=${tag}\nrelease-id=123\nsource-sha=${'a'.repeat(40)}\n`,
        });
      }
    }
  });

  it('refuses absent, malformed or changed inventory without exporting any writer inputs', () => {
    const invalid = [
      null,
      [],
      {},
      { ...inventory, releaseId: 124 },
      { ...inventory, sourceSha: 'b'.repeat(40) },
      { ...inventory, tag: 'v5.0.0-alpha.2' },
      { ...inventory, channel: 'stable' },
      { ...inventory, extra: true },
    ];
    for (const value of invalid) expect(checkTuple(value)).toEqual({ status: 1, output: '' });
    expect(checkTuple(inventory, {}, true)).toEqual({ status: 1, output: '' });
    const invalidInputs: Record<string, string>[] = [
      { RELEASE_ID: '01' },
      { RELEASE_ID: '9007199254740992' },
      { SOURCE_SHA: 'A'.repeat(40) },
      { PRODUCT: 'squad' },
      { CHANNEL: 'beta' },
      { RELEASE_TAG: 'v5.0.0-alpha.1\ninjected=value' },
      { RELEASE_TAG: 'v5.0.0' },
      { RELEASE_TAG: 'tmt-ops-v5.0.0-alpha.1' },
    ];
    for (const overrides of invalidInputs) {
      expect(checkTuple(inventory, overrides)).toEqual({ status: 1, output: '' });
    }
  });
});
