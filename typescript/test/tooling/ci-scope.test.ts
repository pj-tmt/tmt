import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { listE2eFiles, RETIRED_E2E_FILES } from '../../scripts/e2e-shards.mjs';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import {
  RUST_WORKERS,
  ciGatePasses,
  e2eGatePasses,
  explainCiSelection,
  globToRegExp,
  isReleased,
  nativeGatePasses,
  ownerOf,
  parseComponentMap,
  releasedComponentsForPath,
  releasedComponentNamesOfPath,
  readChangedCiAreas,
  renderSelectionEvidence,
  runCiScope,
  rustGatePasses,
  scopedChecks,
  selectCiAreas,
  selectNativeScope,
  selectOfficeBrowser,
  selectColabApp,
  selectRemoteFirestore,
  selectColabHarness,
  selectNativeNotices,
} from '../../scripts/ci-scope.mjs';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);
const { isComponentRetired } = (await import(
  new URL('../../scripts/native-release-policy.mjs', import.meta.url).href
)) as { isComponentRetired: (name: string) => boolean };

describe('native dependency notices selection', () => {
  it.each([
    'rust/Cargo.lock',
    'rust/rust-toolchain.toml',
    'rust/about.toml',
    'rust/about.hbs',
    'rust/licenses/yrs-0.28.0/LICENSE',
    'rust/Cargo.toml',
    'extensions/tmt-colab/rust/tmt-colab/Cargo.toml',
    'dist-workspace.toml',
    '.github/components.json',
    '.github/workflows/ci.yml',
    'scripts/build-native-artifact.sh',
    'typescript/scripts/verify-native-notices.mjs',
    'typescript/scripts/native-artifact-policy.mjs',
    'typescript/scripts/packed-command.mjs',
  ])('selects notice generation for %s', (file) => {
    expect(selectNativeNotices([file])).toBe(true);
  });

  it.each([
    'rust/Cargo.lock.backup',
    'rust/MyCargo.toml',
    'rust/about.toml.backup',
    'rust/licenses-other/LICENSE',
    'extensions/tmt-colab/rust/tmt-colab/Cargo.toml.backup',
    'rust/crates/tmt-core/src/request.rs',
    'typescript/scripts/verify-native-notices.mjs.backup',
    'site/src/chapters/start.mdx',
    'DEVELOPMENT.md',
    'typescript/pnpm-lock.yaml',
  ])('does not select unrelated or prefix-similar %s', (file) => {
    expect(selectNativeNotices([file])).toBe(false);
  });

  it('retains notice verification on an empty conservative diff', () => {
    expect(selectNativeNotices([])).toBe(true);
  });

  it('routes the selected job result into the native gate without delaying Code quality', () => {
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    const job = (name: string) => {
      const block = ci.split(`\n  ${name}:\n`)[1];
      expect(block).toBeDefined();
      return block.split(/\n {2}[a-z0-9-]+:\n/)[0];
    };
    const quality = job('code-quality');
    const native = job('native-install-gate');
    const notices = job('native-notices');
    expect(quality).toContain('needs: [changes, office]');
    expect(quality).not.toContain('native-notices');
    expect(native).toMatch(/needs:[\s\S]*native-notices,/);
    expect(native).toContain('NOTICES_SELECTED: ${{ needs.changes.outputs.native_notices }}');
    expect(native).toContain('NOTICES_RESULT: ${{ needs.native-notices.result }}');
    expect(native).toContain('gate "$NOTICES_SELECTED" "$NOTICES_RESULT"');
    expect(notices).toContain(
      "needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_notices == 'true'"
    );
    expect(notices).toContain('uses: actions/cache/restore@v4');
    expect(notices).toContain('save-if: false');
    expect(notices).toContain('cargo fetch --locked');
    expect(notices).toContain('cargo-about --version 0.9.2 --features cli --locked');
    expect(notices).toContain('node typescript/scripts/verify-native-notices.mjs');
    expect(notices).not.toMatch(
      /cargo-dist|cargo build|dist build|pnpm|corepack|setup-tooling|continue-on-error|: write|environment:/
    );
    expect(ciGatePasses('true', ['success'])).toBe(true);
    for (const result of ['skipped', 'failure', 'cancelled', ''])
      expect(ciGatePasses('true', [result])).toBe(false);
    expect(ciGatePasses('false', ['skipped'])).toBe(true);
    expect(ciGatePasses('', ['skipped'])).toBe(false);
  });
});

describe('CI area selection', () => {
  it('keeps the extension state leaf privately owned by Remote with full native verification', () => {
    const paths = [
      'rust/crates/tmt-extension-state/Cargo.toml',
      'rust/crates/tmt-extension-state/src/lib.rs',
      'rust/crates/tmt-extension-state/tests/state.rs',
    ];
    for (const file of paths) expect(ownerOf(file)).toBe('tmt-remote');
    expect(
      isReleased(
        parseComponentMap(
          readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
        ),
        'tmt-remote'
      )
    ).toBe(true);
    const rows = explainCiSelection(paths);
    for (const row of rows) {
      expect(row.owner).toBe('tmt-remote');
      expect(row.rule).toBe('native-source');
      expect(selectCiAreas([row.path])).toEqual(
        selectCiAreas(['rust/crates/tmt-invoke/src/lib.rs'])
      );
    }
    expect(ownerOf('rust/crates/tmt-extension-state-other/src/lib.rs')).toBe('cli');
  });

  it('keeps the extension objects leaf privately owned by Remote with full native verification', () => {
    const paths = [
      'rust/crates/tmt-extension-objects/Cargo.toml',
      'rust/crates/tmt-extension-objects/src/lib.rs',
      'rust/crates/tmt-extension-objects/src/codec/tests.rs',
    ];
    for (const row of explainCiSelection(paths)) {
      expect(row.owner).toBe('tmt-remote');
      expect(row.rule).toBe('native-source');
      expect(selectCiAreas([row.path])).toEqual(
        selectCiAreas(['rust/crates/tmt-invoke/src/lib.rs'])
      );
    }
    expect(ownerOf('rust/crates/tmt-extension-objects-other/src/lib.rs')).toBe('cli');
  });

  it('rejects a misspelled private-release declaration', () => {
    expect(() =>
      parseComponentMap(
        JSON.stringify({ components: { addon: { owns: ['addon'], release: 'false' } } })
      )
    ).toThrow('release must be boolean');
  });

  it('owns colab-client privately while frozen Office stays unselected', () => {
    const file = 'extensions/tmt-colab/typescript/colab-client/src/object.ts';
    expect(ownerOf(file)).toBe('colab-client');
    const map = parseComponentMap(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    expect(isReleased(map, 'colab-client')).toBe(false);
    expect(selectCiAreas([file])).toEqual({ native: true, office: false, nativeOffice: false });
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain(
      'pnpm --filter @tmt/colab-client install --frozen-lockfile --ignore-scripts'
    );
    for (const command of ['check', 'test'])
      expect(workflow).toContain(`pnpm --filter @tmt/colab-client --fail-if-no-match ${command}`);
  });

  it('owns the private colab app and checks it without selecting frozen Office', () => {
    const file = 'extensions/tmt-colab/typescript/app/src/renderer.ts';
    expect(ownerOf(file)).toBe('colab-app');
    const map = parseComponentMap(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    expect(isReleased(map, 'colab-app')).toBe(false);
    expect(selectCiAreas([file])).toEqual({ native: true, office: false, nativeOffice: false });
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain(
      'pnpm --filter @tmt/colab-app install --frozen-lockfile --ignore-scripts'
    );
    for (const command of ['check', 'test', 'build'])
      expect(workflow).toContain(`pnpm --filter @tmt/colab-app --fail-if-no-match ${command}`);
  });

  it('verifies the browser presentation leaf without narrowing native or activating Office', () => {
    for (const file of [
      'design/browser-ui/package.json',
      'design/browser-ui/src/static.ts',
      'design/browser-ui/src/header.tsx',
      'design/browser-ui/scripts/generate-static-css.mjs',
      'design/browser-ui/generated/static.css',
    ]) {
      expect(ownerOf(file)).toBe('browser-ui');
      expect(selectCiAreas([file])).toEqual({ native: true, office: false, nativeOffice: false });
      expect(selectNativeScope([file])).toBe('full');
    }
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    const quality = ci.split('\n  code-quality:\n')[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
    const leaf = quality
      .split('      - name: Verify isolated browser presentation leaf\n')[1]
      .split('      - name:')[0];
    expect(leaf).toContain('working-directory: typescript');
    expect(leaf).toContain(
      'pnpm --filter @tmt/browser-ui install --frozen-lockfile --ignore-scripts'
    );
    for (const command of ['check', 'test'])
      expect(leaf).toContain(`pnpm --filter @tmt/browser-ui --fail-if-no-match ${command}`);
    expect(leaf).toContain('test/tooling/browser-ui-boundary.test.ts');
    expect(leaf).not.toMatch(/continue-on-error|\n\s+if:/);
    const unit = ci.split('\n  unit-tests:\n')[1].split('\n  docker-e2e-shard-1:\n')[0];
    const install = 'pnpm --filter @tmt/browser-ui install --frozen-lockfile --ignore-scripts';
    const admitsGuardInputs = (text: string): boolean => {
      const step = text
        .split('      - name: Install locked browser presentation guard inputs\n')[1]
        ?.split('      - name:')[0];
      const installAt = text.indexOf(install);
      const testsAt = text.indexOf('pnpm test:run');
      return (
        !!step?.includes('working-directory: typescript') &&
        step.includes(`run: ${install}`) &&
        !/continue-on-error|\n\s+if:/.test(step) &&
        installAt >= 0 &&
        testsAt > installAt
      );
    };
    expect(admitsGuardInputs(unit)).toBe(true);
    const withoutInstall = unit.replace(install, 'pnpm --filter tmt install --frozen-lockfile');
    expect(admitsGuardInputs(withoutInstall)).toBe(false);
    const installStep =
      '      - name: Install locked browser presentation guard inputs\n' +
      unit
        .split('      - name: Install locked browser presentation guard inputs\n')[1]
        .split('      - name:')[0];
    expect(admitsGuardInputs(unit.replace(installStep, '') + installStep)).toBe(false);
  });

  it('selects the add-on workflow only for shell/tool inputs without narrowing look-alikes', () => {
    expect(selectCiAreas(['extensions/tmt-remote/typescript/browser-addon/src/popup.ts'])).toEqual({
      native: false,
      office: false,
      nativeOffice: false,
    });
    for (const file of [
      'extensions/tmt-remote/typescript/browser-addon-other/src/popup.ts',
      'extensions/tmt-remote/rust/tmt-remote/src/main.rs',
      '.github/workflows/browser-addon.yml.orig',
    ]) {
      expect(selectCiAreas([file])).toMatchObject({ native: true, office: false });
    }
    expect(ownerOf('extensions/tmt-remote/typescript/browser-addon/src/popup.ts')).toBe(
      'browser-addon'
    );
    const workflow = readFileSync(
      new URL('../../../.github/workflows/browser-addon.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toMatch(/^on:\n  pull_request:\n    paths:/m);
    expect(workflow).toMatch(/^  workflow_dispatch:/m);
    expect(workflow).not.toContain('continue-on-error');
    const filters = [...workflow.matchAll(/^      - '([^']+)'$/gm)].map((m) => globToRegExp(m[1]));
    const selected = (file: string) => filters.some((filter) => filter.test(file));
    for (const file of [
      'extensions/tmt-remote/typescript/browser-addon/src/popup.ts',
      '.github/workflows/browser-addon.yml',
      '.github/actions/setup-tooling/action.yml',
      'scripts/retry-command.sh',
      'typescript/package.json',
      'typescript/pnpm-workspace.yaml',
      'typescript/pnpm-lock.yaml',
    ])
      expect(selected(file), file).toBe(true);
    for (const file of [
      'rust/crates/tmt-core/src/lib.rs',
      'extensions/tmt-remote/typescript/browser-addon-other/src/popup.ts',
      '.github/workflows/browser-addon.yml.orig',
      'ARCHITECTURE.md',
    ])
      expect(selected(file), file).toBe(false);
    expect(workflow).toContain('--fail-if-no-match');
    expect(workflow).toContain('test:browser');
  });

  it('selects native checks for shared design tokens without narrowing look-alikes', () => {
    const map = parseComponentMap(
      readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
    );
    for (const name of ['tokens.json', 'tokens-plugin.ts', 'package.json']) {
      const file = `design/tokens/${name}`;
      expect(explainCiSelection([file])[0].rule).toBe('design-tokens');
      expect([...releasedComponentNamesOfPath(file, map)].sort()).toEqual([
        'cli',
        'tmt-colab',
        'tmt-remote',
      ]);
      expect(selectCiAreas([file])).toEqual({ native: true, office: false, nativeOffice: false });
      expect(selectNativeScope([file])).toBe('full');
    }
    const lookalike = 'design/tokens-other/tokens.json';
    expect(explainCiSelection([lookalike])[0].rule).toBe('unmapped');
    expect(selectCiAreas([lookalike])).toEqual({
      native: true,
      office: false,
      nativeOffice: false,
    });
  });

  it.each(['colab-header.css', 'reader-style.css', 'notice-card.css'])(
    'retains full native verification for embedded Colab %s',
    (name) => {
      const file = `extensions/tmt-colab/typescript/app/src/${name}`;
      expect(explainCiSelection([file])[0].rule).toBe('colab-native-chrome');
      expect(selectCiAreas([file])).toEqual({ native: true, office: false, nativeOffice: false });
      expect(selectNativeScope([file])).toBe('full');
      expect(explainCiSelection([`${file}.backup`])[0].rule).toBe('unmapped');
    }
  );

  it.each([
    'rust/Cargo.toml',
    'rust/Cargo.lock',
    'rust/rust-toolchain.toml',
    'rust/crates/tmt-core/src/room.rs',
    'rust/crates/tmt-adapters/src/api.rs',
    'rust/crates/tmt-adapters/src/setup/providers.rs',
    'rust/crates/tmt-cli/src/office_facade.rs',
    'rust/crates/tmt-cli/src/api_command.rs',
    'rust/crates/tmt-cli/src/extension_install_command.rs',
    'skills/tmt/SKILL.md',
    'typescript/package.json',
    'typescript/pnpm-lock.yaml',
    'typescript/pnpm-workspace.yaml',
    '.github/workflows/ci.yml',
    '.github/components.json',
    'typescript/scripts/ci-scope.mjs',
    'typescript/test/support/cli-process.ts',
    'typescript/test/e2e/harness.ts',
    'typescript/test/e2e/Dockerfile',
    'typescript/test/native/api.test.ts',
    'typescript/test/tooling/ci-scope.test.ts',
    'typescript/test/e2e/ops.e2e.test.ts',
    'new-owner/file.ts',
    'extensions/tmt-ops-other/file.rs',
    'typescript/apps/office/src/main.tsx',
    'contracts/office/request.json',
    'docs/office.md.orig',
    'rust/archive/NATIVE-INSTALL.md',
  ])('keeps native verification without frozen Office work for %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: true, office: false, nativeOffice: false });
  });

  it.each([
    'extensions/tmt-office/rust/tmt-office/src/main.rs',
    'extensions/tmt-office/contracts/request.json',
    'extensions/tmt-office/skills/tmt-office/SKILL.md',
    'typescript/test/native/office-board.test.ts',
    'typescript/test/e2e/office-command.e2e.test.ts',
    'typescript/test/stress/office-native-installation-capacity.test.ts',
    'typescript/test/support/office-world.ts',
    'extensions/tmt-office/typescript/services/office/firestore.rules',
    'extensions/tmt-office/typescript/apps/office-other/file.ts',
    'extensions/tmt-office/rusty/file.rs',
    '.github/workflows/office-browser.yml',
    '.dockerignore',
    'typescript/scripts/verify-office-emulators.mjs',
  ])('retires Office product checks while retaining native inputs for %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: true, office: false, nativeOffice: false });
  });

  it.each([
    'extensions/tmt-office/typescript/apps/office/src/main.tsx',
    'extensions/tmt-office/docs/architecture.md',
  ])('retires Office product checks without inventing native consumers for %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: false, office: false, nativeOffice: false });
  });

  it.each([
    'ARCHITECTURE.md',
    'DEVELOPMENT.md',
    'contracts/extension-api.md',
    '.agents/skills/tmt-dev/SKILL.md',
    '.github/pull_request_template.md',
    'site/src/chapters/ops.mdx',
    'site/pnpm-lock.yaml',
    '.github/workflows/site.yml',
    'extensions/tmt-remote/typescript/browser-addon/src/popup.ts',
  ])('selects nothing beyond Code quality for %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: false, office: false, nativeOffice: false });
  });

  it('keeps empty diffs conservative for native work without inventing Office ownership', () => {
    expect(selectCiAreas([])).toEqual({ native: true, office: false, nativeOffice: false });
  });

  it('unions ownership across mixed paths and both sides of a no-renames diff', () => {
    expect(
      selectCiAreas(['extensions/tmt-office/typescript/apps/office/removed.ts', 'rust/new.rs'])
    ).toEqual({ native: true, office: false, nativeOffice: false });
    expect(selectCiAreas(['ARCHITECTURE.md', 'typescript/package.json'])).toEqual({
      native: true,
      office: false,
      nativeOffice: false,
    });
  });
});

describe('component map', () => {
  const repository = fileURLToPath(new URL('../../../', import.meta.url));
  const mapText = readFileSync(path.join(repository, '.github/components.json'), 'utf8');
  const map = parseComponentMap(mapText);
  const tracked = (): string[] =>
    runPackedCommand('git', ['ls-files', '-z'], { cwd: repository, env: process.env })
      .split('\0')
      .filter(Boolean);
  const invalid = (change: (value: Record<string, any>) => void) => {
    const value = JSON.parse(mapText);
    change(value);
    return () => parseComponentMap(JSON.stringify(value));
  };

  it('says which components are released: all but the ones that declare release: false', () => {
    expect(isReleased(map, 'cli')).toBe(true);
    expect(isReleased(map, 'ops')).toBe(true);
    expect(() => isReleased(map, 'squad')).toThrow('Unknown component squad.');
    // Office is parked, as the private browser add-on is.
    expect(isReleased(map, 'office')).toBe(false);
    expect(isReleased(map, 'browser-addon')).toBe(false);
    const parked = parseComponentMap(
      JSON.stringify({ components: { office: { owns: ['office'], release: false } } })
    );
    expect(isReleased(parked, 'office')).toBe(false);
    expect(() => isReleased(map, 'nothing')).toThrow('Unknown component nothing.');
  });

  it.each([
    ['rust/crates/tmt-cli-style', ['cli']],
    ['rust/crates/tmt-cli-style/src/lib.rs', ['cli']],
    ['rust/crates/tmt-invoke/src/lib.rs', ['cli']],
    ['rust/crates/tmt-tui/src/lib.rs', []],
    ['rust/crates/tmt-tui-other/src/lib.rs', ['cli']],
    ['extensions/tmt-ops/rust/src/lib.rs', ['ops']],
    ['extensions/tmt-ops-other/rust/src/lib.rs', ['cli']],
    ['extensions/tmt-office/src/lib.rs', []],
    ['extensions/tmt-colab/rust/src/lib.rs', ['tmt-colab']],
    ['rust/crates/tmt-test-support/src/lib.rs', []],
    ['typescript/test/native/ops.test.ts', ['cli']],
  ])('finds released root membership independently of CI ownership for %s', (file, names) => {
    expect(releasedComponentsForPath(file, map).map((component) => component.name)).toEqual(names);
    expect(releasedComponentsForPath(file).map((component) => component.name)).toEqual(names);
  });

  it('matches globs by whole path, with ** across directories and newlines', () => {
    expect(globToRegExp('rust/**').test('rust/crates/tmt-core/src/lib.rs')).toBe(true);
    expect(globToRegExp('rust/**').test('rustic/lib.rs')).toBe(false);
    expect(globToRegExp('apps/office/**').test('apps/office/name with\nnewline.ts')).toBe(true);
    expect(globToRegExp('docs/*.md').test('docs/guide.md')).toBe(true);
    expect(globToRegExp('docs/*.md').test('docs/nested/guide.md')).toBe(false);
    expect(globToRegExp('docs/*.md').test('docs/guide.mdx')).toBe(false);
    expect(globToRegExp('a?c').test('abc')).toBe(true);
    expect(globToRegExp('a?c').test('a/c')).toBe(false);
    expect(globToRegExp('a.b+c').test('aXb+c')).toBe(false);
  });

  it('rejects a malformed map instead of selecting the wrong work', () => {
    expect(invalid((value) => (value.rules[0].consumers = ['native', 'browser']))).toThrow(
      'unknown consumer'
    );
    expect(invalid((value) => (value.rules[1].id = value.rules[0].id))).toThrow('repeated');
    expect(invalid((value) => (value.rules[0].paths = []))).toThrow('non-empty list');
    expect(invalid((value) => delete value.rules[0].why)).toThrow('needs a reason');
    expect(invalid((value) => (value.components = {}))).toThrow('no components');
    expect(invalid((value) => (value.components.cli.owns = 'rust'))).toThrow('non-empty list');
  });

  it('resolves one owner per path: a selectedBy glob wins, look-alike prefixes do not match', () => {
    const owner = (file: string) => ownerOf(file, map);
    expect(owner('rust/crates/tmt-core/src/lib.rs')).toBe('cli');
    expect(owner('typescript/test/native/api.test.ts')).toBe('cli');
    expect(owner('extensions/tmt-office/rust/tmt-office/src/main.rs')).toBe('office');
    expect(owner('extensions/tmt-ops/rust/tmt-ops/src/main.rs')).toBe('ops');
    expect(owner('typescript/test/native/office-board.test.ts')).toBe('office');
    expect(owner('typescript/test/e2e/ops.e2e.test.ts')).toBe('ops');
    expect(owner('typescript/test/e2e/ops-reminder.e2e.test.ts')).toBe('ops');
    expect(owner('extensions/tmt-ops-other/file.rs')).toBe('cli');
    expect(owner('extensions/tmt-officer/file.rs')).toBe('cli');
  });

  it('explains every changed path with its owner, rule and consumers', () => {
    const rows = explainCiSelection(
      ['ARCHITECTURE.md', 'extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'new-owner/file.ts'],
      map
    );
    expect(rows.map(({ path: file, owner, rule }) => [file, owner, rule])).toEqual([
      ['ARCHITECTURE.md', 'cli', 'prose'],
      ['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'ops', 'native-source'],
      ['new-owner/file.ts', 'cli', 'unmapped'],
    ]);
    expect(rows[0]).toMatchObject({ native: false, office: false, nativeOffice: false });
    expect(rows[2]).toMatchObject({ native: true, office: false, nativeOffice: false });
  });

  it('renders the evidence table with the map digest, and bounds a large diff', () => {
    const rows = explainCiSelection(['ARCHITECTURE.md', 'rust/lib.rs'], map);
    const text = renderSelectionEvidence({
      base: 'a'.repeat(40),
      head: 'b'.repeat(40),
      rows,
      areas: selectCiAreas(['ARCHITECTURE.md', 'rust/lib.rs'], map),
      digest: map.digest,
    });
    expect(text).toContain('Diff `aaaaaaaaaaaa...bbbbbbbbbbbb`, 2 changed path(s)');
    expect(text).toContain(`sha256:${map.digest}`);
    expect(text).toContain('Selected: native=true, office=false, native_office=false.');
    expect(text).toContain('| `ARCHITECTURE.md` | cli | prose | nothing |');
    expect(text).toContain('| `rust/lib.rs` | cli | native-source | native |');
    const many = explainCiSelection(
      Array.from({ length: 130 }, (_, index) => `.agents/file-${index}.md`),
      map
    );
    const bounded = renderSelectionEvidence({
      base: 'a'.repeat(40),
      head: 'b'.repeat(40),
      rows: many,
      areas: { native: false, office: false, nativeOffice: false },
      digest: map.digest,
    });
    expect(bounded.match(/^\| `\.agents\//gm)).toHaveLength(100);
    expect(bounded).toContain('30 more path(s) not listed: prose 30.');
  });

  it.each([
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs'], 'ops'],
    [['extensions/tmt-ops/skills/tmt-ops/SKILL.md', 'ARCHITECTURE.md'], 'ops'],
    [['typescript/test/e2e/ops.e2e.test.ts', 'typescript/test/native/ops.test.ts'], 'ops'],
    [['typescript/test/e2e/ops-reminder.e2e.test.ts'], 'ops'],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'rust/Cargo.lock'], 'full'],
    [
      ['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'rust/crates/tmt-cli-style/src/lib.rs'],
      'full',
    ],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'design/cli-style.md'], 'full'],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'typescript/test/native/api.test.ts'], 'full'],
    [['extensions/tmt-ops/rust/tmt-ops/src/main.rs', 'new-owner/file.ts'], 'full'],
    [['extensions/tmt-ops-other/file.rs'], 'full'],
    [['rust/crates/tmt-core/src/lib.rs'], 'full'],
    [['rust/crates/tmt-cli-style/src/lib.rs'], 'full'],
    [['rust/crates/tmt-invoke/src/lib.rs'], 'full'],
    [['extensions/tmt-office/rust/tmt-office/src/main.rs'], 'full'],
    [['ARCHITECTURE.md', 'DEVELOPMENT.md'], 'none'],
    [['extensions/tmt-office/typescript/apps/office/src/main.tsx'], 'none'],
    [[], 'full'],
  ] as const)('selects the native scope for %j: %s', (paths, scope) => {
    expect(selectNativeScope([...paths], map)).toBe(scope);
  });

  it('names the checks a scoped component runs and none for the other scopes', () => {
    expect(scopedChecks('ops', map)).toEqual({
      nativeTests: [
        'ops.test.ts',
        'extension-install.test.ts',
        'extension-upgrade-proof.test.ts',
        'uninstall.test.ts',
      ],
      e2eFiles: ['ops.e2e.test.ts', 'ops-reminder.e2e.test.ts'],
    });
    expect(scopedChecks('full', map)).toEqual({ nativeTests: [], e2eFiles: [] });
    expect(scopedChecks('none', map)).toEqual({ nativeTests: [], e2eFiles: [] });
    expect(scopedChecks('office', map)).toEqual({ nativeTests: [], e2eFiles: [] });
  });

  it('rejects scoped checks that are not plain file names', () => {
    for (const bad of ['../ops.test.ts', 'a b.test.ts', 'ops.test.ts;rm', '$HOME']) {
      expect(invalid((value) => (value.components.ops.scopedChecks.nativeTests = [bad]))).toThrow(
        'plain file names'
      );
    }
    expect(invalid((value) => (value.components.ops.scopedChecks.e2eFiles = []))).toThrow(
      'non-empty list'
    );
    expect(invalid((value) => delete value.components.ops.scopedChecks.nativeTests)).toThrow(
      'non-empty list'
    );
  });

  it('lists every test that runs the Ops binary or reads Ops, so the scoped run cannot miss one', () => {
    const files = tracked();
    const checks = scopedChecks('ops', map);
    for (const file of checks.nativeTests) {
      expect(files, file).toContain(`typescript/test/native/${file}`);
    }
    for (const file of checks.e2eFiles) {
      expect(files, file).toContain(`typescript/test/e2e/${file}`);
    }
    // Tests that name Ops without exercising its executable or sources.
    const namesOnly: Record<string, string> = {
      'typescript/test/e2e/driver-consent.e2e.test.ts':
        'host-driver protocol ops operation names, no Ops executable or source',
      'typescript/test/native/api.test.ts':
        'a synthetic owned skill named tmt-ops; no Ops executable or source',
      'typescript/test/native/office-freeze.test.ts':
        'the remaining installable catalog name after Office removal, without Ops execution or sources',
      'typescript/test/native/setup-guided.test.ts': 'the install hint text',
    };
    const mention = /\bops\b|tmt-ops/i;
    const undeclared = files.filter(
      (file) =>
        /^typescript\/test\/(native\/[^/]+\.test\.ts|e2e\/[^/]+\.e2e\.test\.ts)$/.test(file) &&
        mention.test(readFileSync(path.join(repository, file), 'utf8')) &&
        !checks.nativeTests.includes(path.basename(file)) &&
        !checks.e2eFiles.includes(path.basename(file)) &&
        !(file in namesOnly)
    );
    expect(
      undeclared,
      'these tests mention Ops: add them to scopedChecks or to the names-only list'
    ).toEqual([]);
  });

  it('keeps the tooling tests, which the Ops scope skips, from reading Ops sources', () => {
    const files = tracked().filter((file) =>
      /^typescript\/test\/(tooling|support)\/[^/]+\.ts$/.test(file)
    );
    // Code quality runs these on every pull request, so skipping Unit tests does not skip
    // them: they check inputs that Ops-only changes can also change.
    const alwaysRun = [
      'typescript/test/tooling/ci-scope.test.ts',
      'typescript/test/tooling/component-skills.test.ts',
      'typescript/test/tooling/release-cut.test.ts',
      'typescript/test/tooling/release-cut-live.test.ts',
      'typescript/test/tooling/release-rehearsal.test.ts',
      'typescript/test/tooling/release-version-injection.test.ts',
      'typescript/test/tooling/release-workflow.test.ts',
    ];
    const qualityCommand = /vp test run ([^\n]+)/.exec(
      readFileSync(path.join(repository, '.github/workflows/ci.yml'), 'utf8')
    )?.[1];
    for (const file of alwaysRun) {
      expect(qualityCommand, file).toContain(file.replace('typescript/', ''));
    }
    const readers = files.filter(
      (file) =>
        !alwaysRun.includes(file) &&
        /extensions\/tmt-ops|rust\/target\/debug\/tmt-ops/.test(
          readFileSync(path.join(repository, file), 'utf8')
        )
    );
    // support/native-artifact.ts packages the built Ops binary for the native tests
    // (not run by Unit tests); it is not a tooling test.
    expect(readers.filter((file) => file.startsWith('typescript/test/tooling/'))).toEqual([]);
  });

  it('every rule and selectedBy glob still matches a tracked file', () => {
    const files = tracked();
    expect(files.length).toBeGreaterThan(100);
    const globs = [
      ...map.rules.flatMap((rule) => rule.globs.map((glob) => [`rule ${rule.id}`, glob])),
      ...map.components.flatMap((component) =>
        component.selectedBy.map(({ glob }) => [`component ${component.name}`, glob])
      ),
    ];
    for (const [owner, glob] of globs) {
      const pattern = globToRegExp(glob);
      expect(
        files.some((file) => pattern.test(file)),
        `${owner}: ${glob} matches no tracked file`
      ).toBe(true);
    }
    for (const component of map.components) {
      if (isComponentRetired(component.name)) continue;
      for (const root of [...component.owns, ...component.excludes]) {
        expect(
          root === '.' || files.some((file) => file.startsWith(`${root}/`)),
          `${component.name}: root ${root} has no tracked file`
        ).toBe(true);
      }
    }
    for (const file of files) expect(ownerOf(file, map), file).not.toBe('unowned');
  });

  it('keeps prose inert only while no test or build input reads it', () => {
    const files = tracked();
    const inert = explainCiSelection(files, map)
      .filter((row) => row.rule === 'prose')
      .map((row) => row.path);
    expect(inert).toContain('ARCHITECTURE.md');
    // Files that name inert prose without reading it in a job that inert paths would skip:
    // formatter file lists run in Code quality on every change, and the rest only mention
    // a name.
    const mentions: Record<string, string> = {
      'typescript/scripts/format-workspace.mjs':
        'formatter file lists, run by Code quality on every change',
      'typescript/test/tooling/format-workspace.test.ts':
        'selection path fixtures only; never reads prose contents',
      'typescript/test/tooling/ci-scope.test.ts': 'path fixtures for the selector tests',
      'typescript/test/tooling/release-rehearsal.test.ts':
        'path fixtures for the selection tests; never reads prose contents',
      'typescript/test/fixtures/release-cut-history.json':
        'immutable historical path/map data; cut tests compare strings without reading the named prose',
      'scripts/dev-disk-check.sh': 'names DEVELOPMENT.md in a message',
      'typescript/test/tooling/guide-budget.test.ts':
        'counts DEVELOPMENT.md and ARCHITECTURE.md lines; Code quality runs it on every change',
      '.github/components.json': 'the map names the prose in its own rules',
      '.github/repository-layout.json':
        'top-level names only; the layout guard never reads listed prose',
      'rust/crates/tmt-adapters/src/skill_installation/owned_tests.rs':
        'a fixture file name, not the repository README',
      'site/src/chapters/dev-extension.mdx': 'the handbook site links to the contract on GitHub',
    };
    const layout = JSON.parse(
      readFileSync(path.join(repository, '.github/repository-layout.json'), 'utf8')
    ) as { languageExceptions: Record<string, string> };
    // The handbook site links to the contract on GitHub in each listed translation too.
    const translatedDevExtensionMentions = new Set(
      Object.keys(layout.languageExceptions).map((directory) => `${directory}/dev-extension.mdx`)
    );
    // Only files that contain ".md" at all can name prose.
    const candidates = runPackedCommand('git', ['grep', '-l', '-z', '-I', '-F', '.md', '--', '.'], {
      cwd: repository,
      env: process.env,
    })
      .split('\0')
      .filter(
        (file: string) =>
          file &&
          !file.endsWith('.md') &&
          !(file in mentions) &&
          !translatedDevExtensionMentions.has(file)
      );
    expect(candidates.length).toBeGreaterThan(20);
    const readers: string[] = [];
    for (const file of candidates) {
      const text = readFileSync(path.join(repository, file), 'utf8');
      const references = new Set(text.match(/[A-Za-z0-9_./-]+\.md\b/g) ?? []);
      for (const reference of references) {
        if (
          inert.some(
            (prose) =>
              reference === prose ||
              (prose.includes('/')
                ? reference.endsWith(`/${prose}`)
                : /^(\.\.?\/)+/.test(reference) && reference.replace(/^(\.\.?\/)+/, '') === prose)
          )
        ) {
          readers.push(`${file} -> ${reference}`);
        }
      }
    }
    expect(readers, 'these files read or name inert prose; map the prose to its consumer').toEqual(
      []
    );
  });

  it('keeps Office TypeScript and the shared test helpers away from native-only tests', () => {
    const files = tracked().filter(
      (file) =>
        (file.startsWith('extensions/tmt-office/typescript/') && /\.(tsx?|mjs)$/.test(file)) ||
        (/^typescript\/test\/(e2e|support)\/[^/]+\.(ts|mjs)$/.test(file) &&
          !file.endsWith('.e2e.test.ts'))
    );
    expect(files.length).toBeGreaterThan(50);
    for (const file of files) {
      const text = readFileSync(path.join(repository, file), 'utf8');
      expect(
        /\b(?:from|import)\s*\(?\s*['"][^'"]*(?:\.e2e\.test|\/test\/native\/|\/test\/tooling\/|\.\.\/native\/|\.\.\/tooling\/)/.test(
          text
        ),
        `${file} imports a test the Office jobs are not selected for`
      ).toBe(false);
    }
  });
});

describe('remote Rust retains full CI coverage', () => {
  const remote = 'extensions/tmt-remote/rust/tmt-remote/';
  const all = { native: true, office: false, nativeOffice: false };

  it.each(['Cargo.toml', 'src/core.rs', 'tests/door.rs'])(
    'explicitly selects full workspace checks for %s despite private release ownership',
    (suffix) => {
      const files = [remote + suffix];
      expect(explainCiSelection(files)).toMatchObject([
        { owner: 'tmt-remote', rule: 'remote-rust', ...all },
      ]);
      expect(selectCiAreas(files)).toEqual(all);
      expect(selectNativeScope(files)).toBe('full');
      expect(scopedChecks(selectNativeScope(files))).toEqual({ nativeTests: [], e2eFiles: [] });
    }
  );

  it.each([
    'rust/Cargo.toml',
    'rust/Cargo.lock',
    'rust/crates/tmt-cli-style/src/lib.rs',
    'rust/crates/tmt-cli/tests/architecture.rs',
    'extensions/tmt-ops/rust/tmt-ops/src/main.rs',
    'unknown/new-file',
  ])('retains all consumers and full scope with %s', (other) => {
    expect(selectNativeScope([remote + 'src/main.rs', other])).toBe('full');
    expect(selectCiAreas([remote + 'src/main.rs', other])).toEqual(all);
  });

  it.each([
    'extensions/tmt-remote/rust-other/src/main.rs',
    'extensions/tmt-remote-other/rust/src/main.rs',
    'extensions/tmt-remote/typescript/remote-client/src/canonical.ts',
  ])('does not swallow %s into the Rust rule', (file) => {
    expect(explainCiSelection([file])[0].rule).toBe('unmapped');
    expect(selectCiAreas([file])).toEqual(all);
    expect(selectNativeScope([file])).toBe('full');
  });

  it('keeps the browser separate and rejects every missing selected job', () => {
    const browser = 'extensions/tmt-remote/typescript/browser-addon/src/popup.ts';
    expect(explainCiSelection([browser])[0].rule).toBe('browser-addon');
    expect(selectNativeScope([browser])).toBe('none');
    const scope = selectNativeScope([remote + 'tests/door.rs']);
    const results = {
      nativeRust: 'success',
      unitTests: 'success',
      e2eShard1: 'success',
      e2eShard2: 'success',
      runtimeBuild: 'success',
      packedInstall: 'success',
      macosRuntimeBuild: 'success',
      macosPackedInstall: 'success',
    };
    expect(nativeGatePasses(scope, results, 'true')).toBe(true);
    for (const job of Object.keys(results)) {
      for (const result of ['skipped', 'failure', 'cancelled', undefined]) {
        expect(nativeGatePasses(scope, { ...results, [job]: result }, 'true')).toBe(false);
      }
    }
    expect(nativeGatePasses('remote', results, 'true')).toBe(false);
  });

  it.each([
    {
      discovery: 'door_lifecycle: test\n1 test, 0 benchmarks',
      listStatus: 0,
      testStatus: 0,
      status: 0,
    },
    { discovery: '0 tests, 0 benchmarks', listStatus: 0, testStatus: 0, status: 1 },
    { discovery: 'door_lifecycle: test', listStatus: 7, testStatus: 0, status: 7 },
    { discovery: 'door_lifecycle: test', listStatus: 0, testStatus: 8, status: 8 },
  ])('executes the full workflow block with discovery/test status $status', (fixture) => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    const step = workflow
      .split('      - name: Test and build workspace\n')[1]
      ?.split('\n      - name:')[0];
    expect(step).toContain("if: needs.changes.outputs.native_scope == 'full'");
    expect(step).toContain('working-directory: rust');
    const script = step.split('        run: |\n')[1].replace(/^ {10}/gm, '');
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-remote-ci-'));
    try {
      // Exercise the committed shell, including errexit; the real workspace test run is
      // integration evidence, while this injected Cargo proves empty/failure paths.
      writeExecutable(
        path.join(root, 'cargo'),
        `#!/bin/sh
printf '%s\\n' "$*" >> "$CALLS"
case "$*" in
  'test --locked -p tmt-remote -- --list') printf '%s\\n' "$DISCOVERY"; exit "$LIST_STATUS" ;;
  'test --locked --workspace --exclude tmt-office --exclude tmt-office-storage --exclude tmt-office-pairing --exclude tmt-office-service') exit "$TEST_STATUS" ;;
esac
`,
        0o755
      );
      runPackedCommand('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script], {
        cwd: root,
        env: {
          ...process.env,
          PATH: `${root}:${process.env.PATH}`,
          CALLS: path.join(root, 'calls'),
          DISCOVERY: fixture.discovery,
          LIST_STATUS: String(fixture.listStatus),
          TEST_STATUS: String(fixture.testStatus),
        },
        expectedStatus: fixture.status,
      });
      const calls = readFileSync(path.join(root, 'calls'), 'utf8').trim().split('\n');
      const expected = ['test --locked -p tmt-remote -- --list'];
      if (fixture.status === 0 || fixture.testStatus !== 0)
        expected.push(
          'test --locked --workspace --exclude tmt-office --exclude tmt-office-storage --exclude tmt-office-pairing --exclude tmt-office-service'
        );
      if (fixture.status === 0)
        expected.push(
          'build --locked --workspace --exclude tmt-office --exclude tmt-office-storage --exclude tmt-office-pairing --exclude tmt-office-service'
        );
      expect(calls).toEqual(expected);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe('retired Office browser selection', () => {
  it.each([
    'extensions/tmt-office/typescript/apps/office/src/main.tsx',
    'extensions/tmt-office/rust/tmt-office/src/main.rs',
    'extensions/tmt-office/contracts/world-v1.md',
    'extensions/tmt-office/skills/tmt-office/SKILL.md',
    'extensions/tmt-office/docs/architecture.md',
    'typescript/test/native/office-storage.test.ts',
  ])('does not select retired Office path %s', (file) => {
    expect(selectOfficeBrowser([file])).toBe(false);
    expect(selectOfficeBrowser(['rust/Cargo.lock', file])).toBe(false);
  });

  it.each([
    '.github/workflows/office-browser.yml',
    'typescript/scripts/verify-office-emulators.mjs',
    '.dockerignore',
    'extensions/tmt-office/typescript/services/office/Dockerfile',
    'extensions/tmt-office/typescript/apps/office/e2e/native-office-fixture.ts',
  ])('does not revive retired browser machinery: %s', (file) => {
    expect(selectOfficeBrowser([file])).toBe(false);
  });

  it.each([
    '.github/components.json',
    'typescript/package.json',
    'typescript/pnpm-lock.yaml',
    'typescript/pnpm-workspace.yaml',
    'typescript/scripts/ci-scope.mjs',
    'typescript/scripts/ci-scope.d.mts',
    'typescript/test/tooling/ci-scope.test.ts',
    'typescript/test/support/cli-process.ts',
    'typescript/test/e2e/harness.ts',
    'typescript/test/e2e/harness/fixture.ts',
    'rust/crates/tmt-adapters/src/api.rs',
    'rust/Cargo.lock',
    'contracts/remote-channel-v1.md',
    'extensions/tmt-remote/rust/tmt-remote/src/main.rs',
    '.github/workflows/ci.yml',
    'design/cli-style.md',
    'unmapped/new-file',
    'extensions/tmt-office-other/src/main.rs',
    'typescript/test/support-other/fixture.ts',
    'typescript/test/e2e/harness-other/fixture.ts',
    'typescript/test/e2e/binding.e2e.test.ts',
    'extensions/tmt-office-other/docs/architecture.md',
    'extensions/tmt-ops/rust/tmt-ops/src/main.rs',
  ])('does not spend PR browser runners on non-Office path %s', (file) => {
    expect(selectOfficeBrowser([file])).toBe(false);
  });

  it('keeps empty-diff required checks conservative without claiming an Office change', () => {
    expect(selectOfficeBrowser([])).toBe(false);
    expect(selectCiAreas([])).toEqual({ native: true, office: false, nativeOffice: false });
  });
});

describe('CI diff and command integration', () => {
  it('reads actual additions, cross-owner renames and deletions with whitespace-safe paths', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-ci-scope-'));
    const git = (args: string[]): string =>
      runPackedCommand('git', args, {
        cwd: root,
        env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
      });
    const commit = (): string => {
      git(['add', '.']);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'commit',
        '--quiet',
        '-m',
        'Test fixture\n\nCo-authored-by: Codex <codex@openai.com>',
      ]);
      return git(['rev-parse', 'HEAD']).trim();
    };
    try {
      git(['init', '--quiet']);
      writeFileSync(path.join(root, 'README.md'), 'fixture\n');
      const base = commit();
      mkdirSync(path.join(root, 'extensions/tmt-office/docs'), { recursive: true });
      const historicalSource = path.join(root, 'extensions/tmt-office/docs/name with\nnewline.ts');
      writeFileSync(historicalSource, 'export const fixture = true;\n');
      const historical = commit();
      expect(readChangedCiAreas(base, historical, root)).toEqual({
        native: false,
        office: false,
        nativeOffice: false,
      });
      mkdirSync(path.join(root, 'extensions/tmt-office/typescript/apps/office'), {
        recursive: true,
      });
      const source = path.join(
        root,
        'extensions/tmt-office/typescript/apps/office/name with\nnewline.ts'
      );
      renameSync(historicalSource, source);
      const added = commit();
      expect(readChangedCiAreas(historical, added, root)).toEqual({
        native: false,
        office: false,
        nativeOffice: false,
      });
      mkdirSync(path.join(root, 'rust'));
      const target = path.join(root, 'rust/fixture.rs');
      renameSync(source, target);
      const moved = commit();
      expect(readChangedCiAreas(added, moved, root)).toEqual({
        native: true,
        office: false,
        nativeOffice: false,
      });
      rmSync(target);
      const deleted = commit();
      // Removing a core workspace input does not select frozen Office verification.
      expect(readChangedCiAreas(moved, deleted, root)).toEqual({
        native: true,
        office: false,
        nativeOffice: false,
      });
      expect(() => readChangedCiAreas('--help', deleted, root)).toThrow('exact base and head');
      expect(() => readChangedCiAreas('0'.repeat(40), deleted, root)).toThrow(
        'Packed command failed'
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }, 5000);

  it('writes only the step outputs to stdout and the evidence to stderr and the summary', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-ci-scope-run-'));
    const git = (args: string[]): string =>
      runPackedCommand('git', args, {
        cwd: root,
        env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
      });
    const commit = (): string => {
      git(['add', '.']);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'commit',
        '--quiet',
        '-m',
        'Test fixture',
      ]);
      return git(['rev-parse', 'HEAD']).trim();
    };
    const capture = () => {
      let text = '';
      return { write: (chunk: string) => void (text += chunk), text: () => text };
    };
    try {
      git(['init', '--quiet']);
      writeFileSync(path.join(root, 'ARCHITECTURE.md'), 'fixture\n');
      const base = commit();
      mkdirSync(path.join(root, 'rust'));
      writeFileSync(path.join(root, 'rust/fixture.rs'), '// fixture\n');
      writeFileSync(path.join(root, 'ARCHITECTURE.md'), 'changed\n');
      const head = commit();
      // Outside the fixture repository, which commits everything it finds.
      const summaryFile = `${root}-summary.md`;
      const stdout = capture();
      const stderr = capture();
      runCiScope([base, head], { cwd: root, stdout, stderr, summaryFile });
      const outputs = (text: string) =>
        Object.fromEntries(
          text
            .trim()
            .split('\n')
            .map((line) => [line.slice(0, line.indexOf('=')), line.slice(line.indexOf('=') + 1)])
        );
      const full = outputs(stdout.text());
      expect(full).toMatchObject({
        native: 'true',
        office: 'false',
        native_office: 'false',
        office_browser: 'false',
        colab_harness: 'false',
        colab_app: 'false',
        native_scope: 'full',
        scoped_native_tests: '',
      });
      // Every retained scenario file is in exactly one of the two shards.
      const shardFiles = [...full.e2e_shard_1.split(' '), ...full.e2e_shard_2.split(' ')];
      expect(shardFiles.sort()).toEqual(
        listE2eFiles().filter((file) => !RETIRED_E2E_FILES.includes(file))
      );
      expect(stderr.text()).toContain('### CI selection');
      expect(stderr.text()).toContain('native scope full.');
      expect(stderr.text()).toContain('| `rust/fixture.rs` | cli | native-source | native |');
      expect(stderr.text()).toContain('| `ARCHITECTURE.md` | cli | prose | nothing |');
      expect(readFileSync(summaryFile, 'utf8')).toBe(stderr.text());
      // Without a summary file nothing is written there, and a second run appends.
      const quiet = capture();
      runCiScope([base, head], { cwd: root, stdout: capture(), stderr: quiet });
      expect(quiet.text()).toBe(stderr.text());
      runCiScope([base, head], { cwd: root, stdout: capture(), stderr: capture(), summaryFile });
      expect(readFileSync(summaryFile, 'utf8')).toBe(stderr.text() + stderr.text());
      // An Ops-only change is scoped and names the checks its component runs.
      mkdirSync(path.join(root, 'extensions/tmt-ops/rust/tmt-ops/src'), { recursive: true });
      writeFileSync(path.join(root, 'extensions/tmt-ops/rust/tmt-ops/src/main.rs'), '// x\n');
      const opsHead = commit();
      const scoped = capture();
      const scopedLog = capture();
      runCiScope([head, opsHead], { cwd: root, stdout: scoped, stderr: scopedLog });
      expect(outputs(scoped.text())).toEqual({
        native: 'true',
        office: 'false',
        native_office: 'false',
        office_browser: 'false',
        colab_harness: 'false',
        colab_app: 'false',
        native_scope: 'ops',
        native_notices: 'false',
        remote_firestore: 'false',
        scoped_native_tests:
          'ops.test.ts extension-install.test.ts extension-upgrade-proof.test.ts uninstall.test.ts',
        e2e_shard_1: 'ops.e2e.test.ts ops-reminder.e2e.test.ts',
        e2e_shard_2: '',
      });
      expect(scopedLog.text()).toContain('native scope ops.');
      // A real remote-only git diff must drive the full output consumed by the workflow.
      mkdirSync(path.join(root, 'extensions/tmt-remote/rust/tmt-remote/src'), { recursive: true });
      writeFileSync(path.join(root, 'extensions/tmt-remote/rust/tmt-remote/src/main.rs'), '// x\n');
      const remoteHead = commit();
      const remoteOutput = capture();
      const remoteLog = capture();
      runCiScope([opsHead, remoteHead], { cwd: root, stdout: remoteOutput, stderr: remoteLog });
      expect(outputs(remoteOutput.text())).toEqual(full);
      expect(remoteLog.text()).toContain('| tmt-remote | remote-rust | native |');
      // The Rules emulator suite's fixtures select its Unit tests steps and stay a full native scope.
      const rulesDir = 'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules';
      mkdirSync(path.join(root, rulesDir), { recursive: true });
      writeFileSync(path.join(root, rulesDir, 'composed.rules'), '// x\n');
      const rulesHead = commit();
      const rulesOutput = capture();
      runCiScope([remoteHead, rulesHead], { cwd: root, stdout: rulesOutput, stderr: capture() });
      expect(outputs(rulesOutput.text())).toEqual({ ...full, remote_firestore: 'true' });
      const officePath = path.join(root, 'extensions/tmt-office/docs/fixture.md');
      mkdirSync(path.dirname(officePath), { recursive: true });
      writeFileSync(officePath, 'Office fixture\n');
      const officeHead = commit();
      const browserSelected = (baseHead: string, nextHead: string) => {
        const output = capture();
        runCiScope([baseHead, nextHead], { cwd: root, stdout: output, stderr: capture() });
        return outputs(output.text()).office_browser;
      };
      expect(browserSelected(remoteHead, officeHead)).toBe('false');
      mkdirSync(path.join(root, 'docs'), { recursive: true });
      renameSync(officePath, path.join(root, 'docs/moved.md'));
      const movedHead = commit();
      expect(browserSelected(officeHead, movedHead)).toBe('false');
      writeFileSync(officePath, 'Office fixture\n');
      const restoredHead = commit();
      rmSync(officePath);
      const deletedHead = commit();
      expect(browserSelected(restoredHead, deletedHead)).toBe('false');

      // Additions, both rename endpoints and deletions drive the advisory output.
      const colabPath = path.join(root, 'extensions/tmt-colab/contracts/vectors/fixture.json');
      mkdirSync(path.dirname(colabPath), { recursive: true });
      writeFileSync(colabPath, '{}\n');
      const colabHead = commit();
      const colabSelected = (baseHead: string, nextHead: string) => {
        const output = capture();
        runCiScope([baseHead, nextHead], { cwd: root, stdout: output, stderr: capture() });
        return outputs(output.text()).colab_harness;
      };
      expect(colabSelected(deletedHead, colabHead)).toBe('true');
      const movedColab = path.join(root, 'docs/colab-fixture.json');
      renameSync(colabPath, movedColab);
      const movedColabHead = commit();
      expect(colabSelected(colabHead, movedColabHead)).toBe('true');
      renameSync(movedColab, colabPath);
      const restoredColabHead = commit();
      expect(colabSelected(movedColabHead, restoredColabHead)).toBe('true');
      rmSync(colabPath);
      const deletedColabHead = commit();
      expect(colabSelected(restoredColabHead, deletedColabHead)).toBe('true');
      writeFileSync(path.join(root, 'docs/unrelated.md'), 'Unrelated\n');
      const unrelatedHead = commit();
      expect(colabSelected(deletedColabHead, unrelatedHead)).toBe('false');

      expect(() => runCiScope(['only-one'], { cwd: root, stdout, stderr })).toThrow(
        'exact base and head'
      );
      runCiScope(['gate', 'true', 'success'], { cwd: root, stdout, stderr });
      expect(() => runCiScope(['gate', 'true', 'failure'], { cwd: root, stdout, stderr })).toThrow(
        'did not complete successfully'
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
      rmSync(`${root}-summary.md`, { force: true });
    }
  }, 5000);

  it.each([
    'extensions/tmt-colab/typescript/colab-client/src/index.ts',
    'extensions/tmt-colab/typescript/colab-client/test/differential.mjs',
    'extensions/tmt-colab/typescript/colab-client/package.json',
    'extensions/tmt-colab/rust/tmt-colab-model/src/lib.rs',
    'extensions/tmt-colab/rust/tmt-colab-model/examples/browser_authority.rs',
    'extensions/tmt-colab/contracts/vectors/ed25519-829.jsonl',
    '.github/workflows/colab-browser.yml',
    'rust/Cargo.lock',
    'rust/Cargo.toml',
    'typescript/pnpm-lock.yaml',
  ])('selects the advisory Colab harness for %s', (file) => {
    expect(selectColabHarness([file])).toBe(true);
    expect(selectColabHarness(['DEVELOPMENT.md', file])).toBe(true);
  });

  it.each([
    'extensions/tmt-colab/typescript/colab-client-other/src/index.ts',
    'extensions/tmt-colab/rust/tmt-colab-model-other/src/lib.rs',
    'extensions/tmt-colab/contracts/vectors-other/model.json',
    'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
    'extensions/tmt-colab/contracts/colab-v1.md',
    'extensions/tmt-remote/typescript/remote-client/src/index.ts',
    'extensions/tmt-office/typescript/apps/office/src/main.tsx',
    'typescript/scripts/ci-scope.mjs',
    'typescript/pnpm-lock.yaml.backup',
    'rust/Cargo.lock.backup',
    'rust/Cargo.toml.backup',
    '.github/workflows/ci.yml',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select advisory Colab work for %s', (file) => {
    expect(selectColabHarness([file])).toBe(false);
  });

  it.each([
    'extensions/tmt-colab/typescript/app/src/annotation-input.tsx',
    'extensions/tmt-colab/typescript/app/e2e/fold.spec.ts',
    'extensions/tmt-colab/typescript/app/vite.config.ts',
    'extensions/tmt-colab/typescript/colab-client/src/index.ts',
    'design/browser-ui/src/static.css',
    '.github/workflows/colab-browser.yml',
    'typescript/pnpm-lock.yaml',
  ])('selects the advisory Colab app component suite for %s', (file) => {
    expect(selectColabApp([file])).toBe(true);
    expect(selectColabApp(['DEVELOPMENT.md', file])).toBe(true);
  });

  it.each([
    'extensions/tmt-colab/typescript/app-other/src/index.ts',
    'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
    'extensions/tmt-colab/rust/tmt-colab-model/src/lib.rs',
    'extensions/tmt-colab/contracts/colab-v1.md',
    'extensions/tmt-remote/typescript/remote-client/src/index.ts',
    'typescript/pnpm-lock.yaml.backup',
    '.github/workflows/ci.yml',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select the advisory Colab app component suite for %s', (file) => {
    expect(selectColabApp([file])).toBe(false);
  });

  it('requires mapped ownership and does not expand empty Colab app scope', () => {
    expect(selectColabApp([])).toBe(false);
    const map = parseComponentMap(
      JSON.stringify({ components: { cli: { owns: ['.'] } }, rules: [] })
    );
    expect(selectColabApp(['extensions/tmt-colab/typescript/app/src/index.ts'], map)).toBe(false);
  });

  it.each([
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator/suite.mjs',
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator/firebase.json',
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator/Dockerfile',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules/composed.rules',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules/hostile/path-literal.rules',
    'extensions/tmt-remote/rust/tmt-remote/src/rules.rs',
    'extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js',
    '.github/workflows/ci.yml',
  ])('selects the Remote Firestore Rules emulator steps for %s', (file) => {
    expect(selectRemoteFirestore([file])).toBe(true);
    expect(selectRemoteFirestore(['DEVELOPMENT.md', file])).toBe(true);
    // The steps live in the Unit tests job, which runs only for a full native scope: a selected
    // path must never be scoped away, or the selected steps would be skipped unexpectedly.
    expect(selectNativeScope([file])).toBe('full');
  });

  it.each([
    'extensions/tmt-remote/rust/tmt-remote/tests/emulator-other/suite.mjs',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/declarations/colab-firestore.json',
    'extensions/tmt-remote/rust/tmt-remote/tests/fixtures/rules-other/x.rules',
    'extensions/tmt-remote/rust/tmt-remote/src/deploy_plan.rs',
    'extensions/tmt-remote/rust/tmt-remote/assets/settings-v1.js',
    'extensions/tmt-remote/rust/tmt-remote/tests/rules.rs',
    '.github/workflows/colab-browser.yml',
    'DEVELOPMENT.md',
    'unknown/input',
  ])('does not select the Remote Firestore Rules emulator steps for %s', (file) => {
    expect(selectRemoteFirestore([file])).toBe(false);
  });

  it('does not select the Rules emulator steps for an empty diff', () => {
    expect(selectRemoteFirestore([])).toBe(false);
  });

  it('runs the Rules emulator as pinned, fail-closed steps of the Unit tests job', () => {
    const read = (file: string) =>
      readFileSync(new URL(`../../../${file}`, import.meta.url), 'utf8');
    const ci = read('.github/workflows/ci.yml');
    const job = ci.slice(ci.indexOf('\n  unit-tests:'), ci.indexOf('\n  # The Docker E2E suite'));
    const steps = job.slice(
      job.indexOf('# Remote Firestore Rules emulator suite'),
      job.indexOf('- name: Run retained developer tooling tests')
    );
    expect(ci).toContain(
      "      remote_firestore: ${{ steps.scope.outputs.remote_firestore || steps.queue.outputs.remote_firestore || 'false' }}"
    );
    expect(steps.match(/if: needs\.changes\.outputs\.remote_firestore == 'true'/g)).toHaveLength(2);
    expect(steps).toContain('timeout-minutes: 8');
    expect(steps).toContain('test -n "$JAVA_HOME_21_X64"');
    expect(steps).toContain('"$JAVA_HOME_21_X64/bin/java" -version');
    // No third-party action, retry, cache or relaxed failure in the selected steps.
    for (const forbidden of ['uses:', 'continue-on-error', 'retry', 'cache', '|| true']) {
      expect(steps).not.toContain(forbidden);
    }
    // The pins mirror Office's Dockerfile and the suite's own Dockerfile.
    const pin = 'firebase-tools@15.29.0';
    for (const file of [
      'extensions/tmt-office/typescript/services/office/Dockerfile',
      'extensions/tmt-remote/rust/tmt-remote/tests/emulator/Dockerfile',
    ]) {
      expect(read(file)).toContain(pin);
    }
    expect(steps).toContain(pin);
    const javaImage = /eclipse-temurin:21-jre-jammy@sha256:[0-9a-f]{64}/;
    expect(
      read('extensions/tmt-remote/rust/tmt-remote/tests/emulator/Dockerfile').match(javaImage)?.[0]
    ).toBe(
      read('extensions/tmt-office/typescript/services/office/Dockerfile').match(javaImage)?.[0]
    );
  });

  it('requires mapped Colab ownership and does not expand empty advisory scope', () => {
    expect(selectColabHarness([])).toBe(false);
    const map = parseComponentMap(
      JSON.stringify({
        components: { cli: { owns: ['.'] } },
        rules: [],
      })
    );
    expect(
      selectColabHarness(['extensions/tmt-colab/typescript/colab-client/src/index.ts'], map)
    ).toBe(false);
  });

  it('selects cumulative merge-group endpoints and fails closed when the diff is unreadable', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-ci-queue-'));
    const git = (args: string[]) =>
      runPackedCommand('git', args, {
        cwd: root,
        env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
      });
    const commit = (name: string, contents: string) => {
      mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
      writeFileSync(path.join(root, name), contents);
      git(['add', '.']);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'commit',
        '--quiet',
        '-m',
        'Test fixture',
      ]);
      return git(['rev-parse', 'HEAD']).trim();
    };
    const select = (args: string[], cwd = root) => {
      let output = '';
      let evidence = '';
      runCiScope(args, {
        cwd,
        stdout: {
          write: (text) => {
            output += text;
          },
        },
        stderr: {
          write: (text) => {
            evidence += text;
          },
        },
      });
      return {
        evidence,
        outputs: Object.fromEntries(
          output
            .trim()
            .split('\n')
            .map((line) => line.split('='))
        ),
      };
    };
    try {
      git(['init', '--quiet']);
      const base = commit('README.md', 'base');
      git(['update-ref', 'refs/remotes/origin/main', base]);
      commit('.agents/first.md', 'first PR');
      const docs = commit('.agents/second.md', 'second PR');
      const docsRun = select(['merge-group', docs]);
      expect(docsRun.outputs).toMatchObject({
        native: 'false',
        office: 'false',
        native_scope: 'none',
        e2e_shard_1: '',
        e2e_shard_2: '',
      });
      expect(docsRun.evidence).toContain(`${base.slice(0, 12)}..${docs.slice(0, 12)}`);
      expect(docsRun.evidence).toContain('.agents/first.md');
      expect(docsRun.evidence).toContain('.agents/second.md');
      expect(docsRun.outputs).toEqual(select([base, docs]).outputs);
      const ops = commit('extensions/tmt-ops/rust/tmt-ops/src/main.rs', '// ops');
      const opsRun = select(['merge-group', ops]);
      expect(opsRun.outputs).toMatchObject({
        native: 'true',
        office: 'false',
        native_scope: 'ops',
        e2e_shard_2: '',
      });
      expect(opsRun.outputs.e2e_shard_1).toBe('ops.e2e.test.ts ops-reminder.e2e.test.ts');
      expect(opsRun.outputs).toEqual(select([base, ops]).outputs);
      const shared = commit('unknown-input', 'shared');
      const full = select(['seed']).outputs;
      expect(select(['full']).outputs).toMatchObject({
        office: 'false',
        native_office: 'false',
        office_browser: 'false',
      });
      const office = commit('extensions/tmt-office/rust/tmt-office/src/main.rs', '// Office');
      expect(select([shared, office]).outputs).toMatchObject({
        office: 'false',
        native_office: 'false',
        office_browser: 'false',
      });
      expect(select(['merge-group', office]).outputs).toMatchObject({
        office: 'false',
        native_office: 'false',
        office_browser: 'false',
        colab_harness: 'false',
        colab_app: 'false',
      });
      expect(select(['merge-group', shared]).outputs).toEqual(full);
      git(['update-ref', 'refs/remotes/origin/main', docs]);
      const empty = select(['merge-group', docs]);
      git(['update-ref', 'refs/remotes/origin/main', base]);
      expect(empty.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(empty.evidence).toContain('diff is empty; using full verification');
      for (const args of [
        ['merge-group', '0'.repeat(40)],
        ['merge-group', '--help'],
        ['merge-group', base, docs],
        ['merge-group'],
      ]) {
        const fallback = select(args);
        expect(fallback.outputs).toEqual({ ...full, native_notices: 'true' });
        expect(fallback.evidence).toContain('diff unreadable; using full verification');
      }
      // The earlier queued workspace change selects native, even when HEADGREEN's last tip is site-only.
      git(['checkout', '--quiet', '--detach', base]);
      mkdirSync(path.join(root, 'rust'), { recursive: true });
      writeFileSync(
        path.join(root, 'rust/Cargo.toml'),
        '[workspace.package]\nversion = "5.0.0-alpha.35"\n'
      );
      writeFileSync(
        path.join(root, 'rust/Cargo.lock'),
        '[[package]]\nname = "tmt-cli"\nversion = "5.0.0-alpha.35"\n'
      );
      const workspace = commit(
        'rust/Cargo.toml',
        '[workspace.package]\nversion = "5.0.0-alpha.35"\n'
      );
      const siteTip = commit('site/src/chapters/start.mdx', 'site-only tip');
      // The old event-base range contains no native files: this is the causal negative control.
      expect(select([workspace, siteTip]).outputs.native_scope).toBe('none');
      expect(select([workspace, siteTip]).outputs.native_notices).toBe('false');
      const pending = select(['merge-group', siteTip]);
      expect(pending.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(pending.evidence).toContain(`${base.slice(0, 12)}..${siteTip.slice(0, 12)}`);
      for (const file of ['rust/Cargo.toml', 'rust/Cargo.lock']) {
        expect(pending.evidence).toContain(file);
      }
      git(['update-ref', '-d', 'refs/remotes/origin/main']);
      const absentTarget = select(['merge-group', siteTip]);
      expect(absentTarget.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(absentTarget.evidence).toContain('diff unreadable; using full verification');
      git(['update-ref', 'refs/remotes/origin/main', base]);
      // Divergent target/head histories use their common ancestor, excluding target-only work.
      git(['checkout', '--quiet', '--detach', base]);
      const core = commit('rust/fixture.rs', '// core');
      git(['update-ref', 'refs/remotes/origin/main', core]);
      expect(select(['merge-group', docs]).outputs.native_scope).toBe('none');
      expect(select([core, docs]).outputs.native_scope).toBe('none');
      // A real shallow checkout lacks the base object: it must not become an empty/none diff.
      const shallow = path.join(root, 'shallow');
      git([
        '-c',
        'advice.detachedHead=false',
        'clone',
        '--quiet',
        '--depth=1',
        `file://${root}`,
        shallow,
      ]);
      const fallback = select(['merge-group', core], shallow);
      expect(fallback.outputs).toEqual({ ...full, native_notices: 'true' });
      expect(fallback.evidence).toContain('diff unreadable; using full verification');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }, 5000);

  it('propagates gate failure to the command exit instead of only returning a boolean', () => {
    const script = fileURLToPath(new URL('../../scripts/ci-scope.mjs', import.meta.url));
    const options = { cwd: tmpdir(), env: process.env };
    expect(runPackedCommand(process.execPath, [script, 'gate', 'true', 'success'], options)).toBe(
      ''
    );
    expect(() =>
      runPackedCommand(process.execPath, [script, 'gate', 'true', 'skipped'], options)
    ).toThrow('Selected CI work did not complete successfully');
  });
});

describe('frozen Office process selection', () => {
  const repository = fileURLToPath(new URL('../../../', import.meta.url));
  const workflow = readFileSync(path.join(repository, '.github/workflows/ci.yml'), 'utf8');
  const step = workflow
    .split('      - name: Verify native process and shared parser contracts\n')[1]
    .split('      - name:')[0];
  const script = step.split('        run: |\n')[1].replace(/^ {10}/gm, '');

  it('keeps the native exclusion aligned with Office component ownership', () => {
    const map = parseComponentMap(
      readFileSync(path.join(repository, '.github/components.json'), 'utf8')
    );
    const nativeGlobs = map.components
      .find((component) => component.name === 'office')!
      .selectedBy.filter(({ glob }) => glob.startsWith('typescript/test/native/'));
    expect(nativeGlobs.map(({ glob }) => glob)).toEqual(['typescript/test/native/office-*']);
    const exclude = /--exclude '([^']+)'/.exec(script)?.[1];
    expect(exclude).toBe(nativeGlobs[0].glob.replace('typescript/', '') + '.test.ts');
    for (const name of [
      'office-uninstall',
      'office-legacy-skills',
      'office-extension-hooks',
      'office-native-installation',
    ]) {
      const file = `test/native/${name}.test.ts`;
      expect(globToRegExp(exclude!).test(file), file).toBe(true);
      expect(ownerOf('typescript/' + file, map), file).toBe('office');
    }
  });

  it('keeps companion stress discovery aligned with Office ownership and native exclusion', () => {
    const map = parseComponentMap(
      readFileSync(path.join(repository, '.github/components.json'), 'utf8')
    );
    const stressGlobs = map.components
      .find((component) => component.name === 'office')!
      .selectedBy.filter(({ glob }) => glob.startsWith('typescript/test/stress/'));
    expect(stressGlobs.map(({ glob }) => glob)).toEqual(['typescript/test/stress/office-*']);
    const config = readFileSync(
      path.join(repository, 'typescript/test/stress/vitest.config.ts'),
      'utf8'
    );
    expect(config).toContain("include: ['test/stress/**/*.test.ts']");
    expect(config).not.toContain('passWithNoTests');
    const file = 'test/stress/office-native-installation-capacity.test.ts';
    const exclude = /--exclude '([^']+)'/.exec(script)![1];
    expect(globToRegExp(exclude.replace('native/', 'stress/')).test(file)).toBe(true);
    expect(ownerOf('typescript/' + file, map)).toBe('office');
    expect(readdirSync(path.join(repository, 'typescript/test/stress'))).toContain(
      path.basename(file)
    );
  });

  it.each(['true', 'false'])('runs a nonempty native selection with Office=%s', (selected) => {
    const files = readdirSync(path.join(repository, 'typescript/test/native')).filter((file) =>
      file.endsWith('.test.ts')
    );
    const office = files.filter((file) => file.startsWith('office-'));
    const core = files.filter((file) => !file.startsWith('office-'));
    expect(office.length).toBeGreaterThan(0);
    expect(core.length).toBeGreaterThan(0);
    for (const file of office) expect(ownerOf('typescript/test/native/' + file)).toBe('office');
    const config = readFileSync(
      path.join(repository, 'typescript/test/native/vitest.config.ts'),
      'utf8'
    );
    expect(config).toContain("include: ['test/native/**/*.test.ts']");
    expect(config).not.toContain('passWithNoTests');
    const root = mkdtempSync(path.join(tmpdir(), 'tmt-office-ci-'));
    try {
      writeExecutable(
        path.join(root, 'pnpm'),
        '#!/bin/sh\nprintf "%s\\n" "$@" > "$CALLS"\nexit "$TEST_STATUS"\n',
        0o755
      );
      for (const status of [0, 7]) {
        runPackedCommand('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script], {
          cwd: root,
          env: {
            ...process.env,
            PATH: `${root}:${process.env.PATH}`,
            CALLS: path.join(root, 'calls'),
            TEST_STATUS: String(status),
            NATIVE_OFFICE_SELECTED: selected,
          },
          expectedStatus: status,
        });
        const args = readFileSync(path.join(root, 'calls'), 'utf8').trim().split('\n');
        expect(args).toEqual(
          selected === 'true'
            ? ['test:native', '--reporter=verbose']
            : ['test:native', '--reporter=verbose', '--exclude', 'test/native/office-*.test.ts']
        );
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it.each(['', 'unknown'])('fails before executing an unavailable selection %s', (selected) => {
    expect(() =>
      runPackedCommand('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script], {
        cwd: tmpdir(),
        env: { ...process.env, NATIVE_OFFICE_SELECTED: selected },
      })
    ).toThrow('Missing Office native selection');
  });
});

describe('required CI gate', () => {
  it('accepts only successful selected work or explicitly unselected skipped work', () => {
    expect(ciGatePasses('true', ['success', 'success'])).toBe(true);
    expect(ciGatePasses('false', ['skipped', 'skipped'])).toBe(true);
  });

  it.each(['failure', 'cancelled', 'skipped', '', 'unknown'])(
    'rejects selected result %s',
    (result) => {
      expect(ciGatePasses('true', ['success', result])).toBe(false);
    }
  );

  it('rejects missing selection/results and contradictions rather than claiming a pass', () => {
    expect(ciGatePasses('', ['skipped'])).toBe(false);
    expect(ciGatePasses('true', [])).toBe(false);
    expect(ciGatePasses('false', ['success'])).toBe(false);
    expect(ciGatePasses('false', ['failure'])).toBe(false);
  });

  it.each([
    {
      selection: 'Office only',
      office: ['true', ['success', 'success']] as const,
      native: ['false', ['skipped', 'skipped', 'skipped', 'skipped', 'skipped']] as const,
    },
    {
      selection: 'native only',
      office: ['false', ['skipped']] as const,
      native: ['true', ['success', 'success', 'success', 'success', 'success', 'success']] as const,
    },
    {
      selection: 'Office and native',
      office: ['true', ['success', 'success']] as const,
      native: ['true', ['success', 'success', 'success', 'success', 'success', 'success']] as const,
    },
    {
      selection: 'neither',
      office: ['false', ['skipped']] as const,
      native: ['false', ['skipped', 'skipped', 'skipped', 'skipped', 'skipped']] as const,
    },
  ])('accepts the complete $selection partition result', ({ office, native }) => {
    expect(ciGatePasses(office[0], [...office[1]])).toBe(true);
    expect(ciGatePasses(native[0], [...native[1]])).toBe(true);
  });

  /** The results of the eight native jobs, all `success` unless overridden. */
  const native = (overrides: Record<string, string> = {}) => ({
    nativeRust: 'success',
    unitTests: 'success',
    e2eShard1: 'success',
    e2eShard2: 'success',
    runtimeBuild: 'success',
    packedInstall: 'success',
    macosRuntimeBuild: 'success',
    macosPackedInstall: 'success',
    ...overrides,
  });
  const allSkipped = {
    nativeRust: 'skipped',
    unitTests: 'skipped',
    e2eShard1: 'skipped',
    e2eShard2: 'skipped',
    runtimeBuild: 'skipped',
    packedInstall: 'skipped',
    macosRuntimeBuild: 'skipped',
    macosPackedInstall: 'skipped',
  };
  const opsRun = native({
    unitTests: 'skipped',
    e2eShard2: 'skipped',
    runtimeBuild: 'skipped',
    packedInstall: 'skipped',
    macosRuntimeBuild: 'skipped',
    macosPackedInstall: 'skipped',
  });

  it.each([
    ['full', native()],
    ['ops', opsRun],
    ['none', allSkipped],
  ])('accepts exactly the %s native results', (scope, expected) => {
    expect(nativeGatePasses(scope, expected, 'true')).toBe(true);
  });

  it.each([
    ['full', native({ unitTests: 'skipped' }), 'a skipped job that must run'],
    ['full', native({ packedInstall: 'failure' }), 'a failed job'],
    ['full', native({ packedInstall: 'cancelled' }), 'a cancelled job'],
    ['full', native({ e2eShard2: 'skipped' }), 'a skipped second E2E shard'],
    ['full', native({ e2eShard1: 'failure' }), 'a failed first E2E shard'],
    ['ops', { ...opsRun, unitTests: 'success' }, 'unit tests that must be skipped'],
    ['ops', { ...opsRun, e2eShard1: 'skipped' }, 'a skipped E2E shard'],
    ['ops', { ...opsRun, e2eShard2: 'success' }, 'a second E2E shard that must be skipped'],
    ['ops', { ...opsRun, nativeRust: 'skipped' }, 'a skipped native contract'],
    ['ops', { ...opsRun, runtimeBuild: 'success' }, 'a runtime build that must be skipped'],
    ['ops', { ...opsRun, e2eShard1: 'failure' }, 'a failed E2E shard'],
    ['none', { ...allSkipped, nativeRust: 'success' }, 'work when nothing was selected'],
    ['unknown', native(), 'an unknown scope'],
    ['', allSkipped, 'an empty scope'],
    ['office', opsRun, 'a component without scoped checks'],
    ['ops', { ...opsRun, packedInstall: '' }, 'an empty result'],
  ])('rejects %s with %o (%s)', (scope, actual) => {
    expect(nativeGatePasses(scope, actual, 'true')).toBe(false);
  });

  it('allows exactly the macOS omissions on merge groups and rejects missing evidence', () => {
    const queue = native({ macosRuntimeBuild: 'skipped', macosPackedInstall: 'skipped' });
    expect(nativeGatePasses('full', queue, 'false')).toBe(true);
    expect(nativeGatePasses('full', queue, 'true')).toBe(false);
    expect(nativeGatePasses('full', native(), 'false')).toBe(false);
    for (const [job, expected] of Object.entries(queue)) {
      for (const result of ['success', 'skipped', 'failure', 'cancelled', '', undefined]) {
        if (result === expected) continue;
        expect(nativeGatePasses('full', { ...queue, [job]: result }, 'false')).toBe(false);
      }
    }
    for (const macos of ['', 'unknown', undefined]) {
      expect(nativeGatePasses('full', queue, macos as string)).toBe(false);
      expect(nativeGatePasses('none', allSkipped, macos as string)).toBe(false);
      expect(nativeGatePasses('ops', opsRun, macos as string)).toBe(false);
    }
    expect(nativeGatePasses('ops', opsRun, 'false')).toBe(true);
    expect(nativeGatePasses('none', allSkipped, 'false')).toBe(true);
  });

  it('rejects missing native results rather than claiming a pass', () => {
    expect(nativeGatePasses('full', {} as never, 'true')).toBe(false);
    expect(nativeGatePasses('none', undefined as never, 'true')).toBe(false);
    const { e2eShard2: _missing, ...withoutShard } = native();
    expect(nativeGatePasses('full', withoutShard as never, 'true')).toBe(false);
  });

  describe('Native Rust contracts gate', () => {
    const expectedFor = (scope: string, officeSelected: string) =>
      scope === 'none'
        ? Array<string>(5).fill('skipped')
        : [
            'success',
            'success',
            scope === 'full' && officeSelected === 'true' ? 'success' : 'skipped',
            'success',
            'success',
          ];

    it.each(['full', 'ops', 'none'])('requires exactly the selected workers for %s', (scope) => {
      for (const officeSelected of ['true', 'false']) {
        const expected = expectedFor(scope, officeSelected);
        expect(rustGatePasses(scope, expected, officeSelected)).toBe(true);
        for (let index = 0; index < expected.length; index++) {
          for (const other of ['success', 'skipped', 'failure', 'cancelled', '', undefined]) {
            if (other === expected[index]) continue;
            const changed = [...expected];
            changed[index] = other as string;
            expect(
              rustGatePasses(scope, changed, officeSelected),
              `${scope} worker ${index}: ${other}`
            ).toBe(false);
          }
        }
        for (const results of [[], expected.slice(1), [...expected, 'success'], undefined])
          expect(rustGatePasses(scope, results as string[], officeSelected)).toBe(false);
      }
    });

    it.each(['', 'unknown', 'office'])('rejects unavailable/unsupported scope %s', (scope) => {
      expect(rustGatePasses(scope, expectedFor('full', 'true'), 'true')).toBe(false);
      expect(rustGatePasses(scope, expectedFor('none', 'false'), 'false')).toBe(false);
    });

    it.each(['', 'unknown', undefined])('rejects unavailable Office selection %s', (selected) => {
      expect(rustGatePasses('full', expectedFor('full', 'false'), selected as string)).toBe(false);
    });

    it('enforces worker selection and failure in the actual command', () => {
      const script = fileURLToPath(new URL('../../scripts/ci-scope.mjs', import.meta.url));
      const options = { cwd: process.cwd(), env: process.env };
      for (const scope of ['full', 'ops', 'none']) {
        for (const selected of ['true', 'false']) {
          const expected = expectedFor(scope, selected);
          expect(
            runPackedCommand(
              process.execPath,
              [script, 'gate-rust', scope, selected, ...expected],
              options
            )
          ).toBe('');
          for (let index = 0; index < expected.length; index++) {
            const changed = [...expected];
            changed[index] = 'failure';
            expect(() =>
              runPackedCommand(
                process.execPath,
                [script, 'gate-rust', scope, selected, ...changed],
                options
              )
            ).toThrow();
          }
          for (const values of [expected.slice(1), [...expected, 'success']])
            expect(() =>
              runPackedCommand(
                process.execPath,
                [script, 'gate-rust', scope, selected, ...values],
                options
              )
            ).toThrow();
        }
      }
      for (const selection of ['', 'unknown'])
        expect(() =>
          runPackedCommand(
            process.execPath,
            [script, 'gate-rust', 'full', selection, ...expectedFor('full', 'false')],
            options
          )
        ).toThrow();
    });
  });

  describe('Docker E2E gate', () => {
    it.each([
      ['full', 'success', 'success'],
      ['ops', 'success', 'skipped'],
      ['none', 'skipped', 'skipped'],
    ])(
      'passes for %s only with the results the selector implies (%s, %s)',
      (scope, first, second) => {
        expect(e2eGatePasses(scope, { e2eShard1: first, e2eShard2: second })).toBe(true);
      }
    );

    it.each([
      ['full', 'skipped', 'success', 'a selected shard that was skipped'],
      ['full', 'success', 'skipped', 'a selected shard that was skipped'],
      ['full', 'skipped', 'skipped', 'both shards skipped although native work was selected'],
      ['full', 'cancelled', 'success', 'a cancelled shard'],
      ['full', 'success', 'cancelled', 'a cancelled shard'],
      ['full', 'failure', 'success', 'a failed shard'],
      ['full', 'success', '', 'a missing shard result'],
      ['full', '', '', 'both results missing'],
      ['ops', 'skipped', 'skipped', 'the Ops shard skipped'],
      ['ops', 'cancelled', 'skipped', 'the Ops shard cancelled'],
      ['ops', 'success', 'success', 'a second shard that must be skipped'],
      ['ops', 'success', 'failure', 'a second shard that ran and failed'],
      ['none', 'success', 'skipped', 'a shard that ran although nothing was selected'],
      ['none', 'skipped', 'success', 'a shard that ran although nothing was selected'],
      ['none', 'failure', 'failure', 'failed shards although nothing was selected'],
      ['', 'skipped', 'skipped', 'an unavailable selector scope'],
      ['unknown', 'success', 'success', 'an unknown scope'],
    ])('rejects %s with (%s, %s): %s', (scope, first, second) => {
      expect(e2eGatePasses(scope, { e2eShard1: first, e2eShard2: second })).toBe(false);
    });

    it('rejects results that are not there at all', () => {
      expect(e2eGatePasses('full', {} as never)).toBe(false);
      expect(e2eGatePasses('full', { e2eShard1: 'success' } as never)).toBe(false);
      expect(e2eGatePasses('none', undefined as never)).toBe(false);
    });

    it('fails the command with the same rules, and needs exactly two shard results', () => {
      const io = () => ({
        cwd: fileURLToPath(new URL('../../../', import.meta.url)),
        stdout: { write: () => {} },
        stderr: { write: () => {} },
      });
      runCiScope(['gate-e2e', 'full', 'success', 'success'], io());
      runCiScope(['gate-e2e', 'ops', 'success', 'skipped'], io());
      runCiScope(['gate-e2e', 'none', 'skipped', 'skipped'], io());
      for (const args of [
        ['gate-e2e', 'full', 'success', 'skipped'],
        ['gate-e2e', 'full', 'success'],
        ['gate-e2e', 'full', 'success', 'success', 'success'],
        ['gate-e2e', 'none', 'success', 'skipped'],
        ['gate-e2e', 'full', 'success', 'cancelled'],
        ['gate-e2e', '', 'skipped', 'skipped'],
      ]) {
        expect(() => runCiScope(args, io()), args.join(' ')).toThrow('Docker E2E shards');
      }
      const queueResults = [
        'success',
        'success',
        'success',
        'success',
        'success',
        'success',
        'skipped',
        'skipped',
      ];
      runCiScope(['gate-native', 'false', 'full', ...queueResults], io());
      for (const macos of ['true', '', 'unknown']) {
        expect(() => runCiScope(['gate-native', macos, 'full', ...queueResults], io())).toThrow(
          'native CI work'
        );
      }
      expect(() =>
        runCiScope(['gate-native', 'false', 'full', ...queueResults.slice(1)], io())
      ).toThrow('native CI work');
      runCiScope(
        [
          'gate-native',
          'true',
          'ops',
          'success',
          'skipped',
          'success',
          'skipped',
          'skipped',
          'skipped',
          'skipped',
          'skipped',
        ],
        io()
      );
      expect(() =>
        runCiScope(
          [
            'gate-native',
            'true',
            'full',
            'success',
            'success',
            'success',
            'skipped',
            'success',
            'success',
            'success',
            'success',
          ],
          io()
        )
      ).toThrow('native CI work');
    });
  });

  it('fails both stable aggregates when selector output is unavailable', () => {
    expect(ciGatePasses('', ['skipped'])).toBe(false);
    expect(ciGatePasses('', ['skipped', 'skipped', 'skipped', 'skipped', 'skipped'])).toBe(false);
  });

  it('keeps all PR runtime rows but skips only the separate macOS jobs on merge groups', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const job = (name: string) => {
      const start = workflow.indexOf(`\n  ${name}:\n`);
      expect(start).toBeGreaterThan(0);
      const next = workflow.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return workflow.slice(start, next < 0 ? undefined : start + 1 + next);
    };
    const build = job('native-runtime-build');
    const macBuild = job('native-runtime-build-macos');
    const packed = job('packed-native-install');
    const macPacked = job('packed-native-install-macos');
    expect(build.match(/- target: (.+)/g)).toEqual([
      '- target: x86_64-unknown-linux-musl',
      '- target: aarch64-unknown-linux-musl',
    ]);
    expect(macBuild.match(/- target: (.+)/g)).toEqual([
      '- target: x86_64-apple-darwin',
      '- target: aarch64-apple-darwin',
    ]);
    expect(packed.match(/- label: (.+)/g)).toEqual([
      '- label: Linux glibc x64',
      '- label: Linux glibc arm64',
      '- label: Linux musl x64',
      '- label: Linux musl arm64',
    ]);
    expect(macPacked.match(/- label: (.+)/g)).toEqual([
      '- label: macOS x64',
      '- label: macOS arm64',
    ]);
    expect(macBuild).toContain(
      "if: needs.changes.outputs.macos == 'true' && needs.changes.outputs.native_scope == 'full'"
    );
    expect(macPacked).toContain(
      "if: needs.changes.outputs.macos == 'true' && needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_scope == 'full'"
    );
    expect(macPacked).toContain('needs: [changes, native-runtime-build-macos]');
    expect(packed).toContain('needs: [changes, native-runtime-build]');
    expect(build).toContain('steps: &native-runtime-build-steps');
    expect(macBuild).toContain('steps: *native-runtime-build-steps');
    expect(packed).toContain('steps: &packed-native-install-steps');
    expect(macPacked).toContain('steps: *packed-native-install-steps');
    for (const section of [build, macBuild]) {
      expect(section).toContain('name: Build native runtime (${{ matrix.target }})');
    }
    for (const section of [packed, macPacked]) {
      expect(section).toContain('name: Packed install (${{ matrix.label }})');
    }
    const gate = job('native-install-gate');
    expect(gate).toContain('MACOS_SELECTED: ${{ needs.changes.outputs.macos }}');
    for (const name of ['native-runtime-build-macos', 'packed-native-install-macos']) {
      expect(gate).toContain(`        ${name},`);
    }
  });

  it('builds and transfers the driver-owned companion without replacing debug Rust verification', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const start = workflow.indexOf('\n  native-clippy:\n');
    const native = workflow.slice(start, workflow.indexOf('\n  unit-tests:\n', start));
    expect(
      native.match(/cargo build --locked --release -p tmt-cli -p tmt-driver-herdr --bins/g)
    ).toHaveLength(2);
    const runtime = workflow
      .split('\n  native-runtime-build:\n')[1]
      .split('\n  native-runtime-build-macos:\n')[0];
    expect(runtime).toContain(
      "cargo build --locked --release --target '${{ matrix.target }}' -p tmt-cli -p tmt-driver-herdr --bins"
    );
    expect(runtime).toContain('rust/target/${{ matrix.target }}/release/tmt-driver-herdr');
    expect(runtime).toContain(
      "cargo build --locked --release --target '${{ matrix.target }}' -p tmt-test-support --example colab-runtime-fixture"
    );
    expect(runtime).toContain(
      'rust/target/${{ matrix.target }}/release/examples/colab-runtime-fixture'
    );
    const tooling = workflow.split('\n  unit-tests:\n')[1].split('\n  docker-e2e-shard-1:\n')[0];
    expect(tooling).toContain('name: runtime-x86_64-unknown-linux-musl');
    expect(tooling).toContain('path: rust/target/debug');
    expect(tooling).toContain(
      'chmod +x ../rust/target/debug/tmt ../rust/target/debug/tmt-driver-herdr'
    );
    expect(tooling).toContain('../rust/target/debug/examples/colab-runtime-fixture');
    expect(native).toContain('rust/target/release/tmt');
    expect(native).toContain('cargo test --locked');
    expect(native).toContain(
      'cargo clippy --locked --workspace --exclude tmt-office --exclude tmt-office-storage --exclude tmt-office-pairing --exclude tmt-office-service --all-targets -- -D warnings'
    );
    expect(native).toContain(
      'cargo +"$MSRV" check --locked --workspace --exclude tmt-office --exclude tmt-office-storage --exclude tmt-office-pairing --exclude tmt-office-service --all-targets'
    );
    expect(native).not.toMatch(/cargo \+\d/);
    expect(native).toContain('cargo build --locked -p tmt-office');
    expect(native).toContain('rust/target/debug/examples/storage-probe');
    expect(native).toContain('cargo build --locked --example storage-probe');
    expect(native).toContain('cargo fmt --all --check');
    expect(native).toContain('pnpm test:native --reporter=verbose');
  });

  it('keeps browser diagnostics outside required aggregates while required results fail closed', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const office = workflow.slice(
      workflow.indexOf('\n  office:\n'),
      workflow.indexOf('\n  code-quality:\n')
    );
    const codeQuality = workflow.slice(
      workflow.indexOf('\n  code-quality:\n'),
      workflow.indexOf('\n  native-rust:\n')
    );
    const native = workflow.slice(workflow.indexOf('\n  native-install-gate:\n'));

    expect(office).toContain('needs: changes');
    expect(office).toContain('docker build --target browser-tests-base');
    expect(office).not.toContain('office-browser');
    expect(office).not.toContain('native-office-browser');
    expect(codeQuality).toContain('needs: [changes, office]');
    expect(codeQuality).toContain('OFFICE_RESULT: ${{ needs.office.result }}');
    expect(codeQuality).toContain('gate "$OFFICE_SELECTED" "$OFFICE_RESULT"');
    expect(native).toContain('native-rust');
    expect(native).toContain('unit-tests');
    expect(native).toContain('docker-e2e');
    expect(native).toContain('native-runtime-build');
    expect(native).toContain('packed-native-install');
    expect(native).not.toContain('native-office-browser');
    expect(native).not.toContain('BROWSER_RESULT');

    expect(ciGatePasses('true', ['success'])).toBe(true);
    expect(ciGatePasses('true', ['failure'])).toBe(false);
    expect(ciGatePasses('true', ['failure', 'success', 'success', 'success', 'success'])).toBe(
      false
    );
  });

  it('keeps complete disjoint fallback shards and wires merge-group selection with read-only caches', () => {
    const capture = () => {
      let value = '';
      return {
        write: (text: string) => {
          value += text;
        },
        text: () => value,
      };
    };
    const stdout = capture();
    const stderr = capture();
    runCiScope(['full'], { cwd: '/no-diff-required', stdout, stderr });
    const outputs: Record<string, string> = Object.fromEntries(
      stdout
        .text()
        .trim()
        .split('\n')
        .map((line) => line.split('='))
    );
    expect(outputs).toMatchObject({ native: 'true', office: 'false', native_scope: 'full' });
    const first = outputs.e2e_shard_1.split(' ');
    const second = outputs.e2e_shard_2.split(' ');
    expect(first.length).toBeGreaterThan(0);
    expect(second.length).toBeGreaterThan(0);
    expect(first.filter((file) => second.includes(file))).toEqual([]);
    expect([...first, ...second].sort()).toEqual(
      listE2eFiles().filter((file) => !RETIRED_E2E_FILES.includes(file))
    );
    expect(stderr.text()).toContain('Weekly/manual retained-product verification; Office retired.');
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toContain('\n  merge_group:\n');
    expect(workflow).toContain(
      "VERIFY: ${{ github.event_name == 'pull_request' || github.event_name == 'merge_group' }}"
    );
    expect(workflow).toContain("if: steps.event.outputs.verify != 'true'");
    expect(workflow).toContain('verify: ${{ steps.event.outputs.verify }}');
    expect(workflow).toContain('macos: ${{ steps.event.outputs.macos }}');
    expect(workflow).toContain("MACOS: ${{ github.event_name != 'merge_group' }}");
    expect(workflow).toContain('fetch-depth: 0');
    expect(workflow).not.toContain('github.event.merge_group.base_sha');
    expect(workflow).toContain('HEAD_SHA: ${{ github.event.merge_group.head_sha }}');
    expect(workflow).toContain(
      'run: node typescript/scripts/ci-scope.mjs merge-group "$HEAD_SHA" >> "$GITHUB_OUTPUT"'
    );
    const cachePolicies = workflow.match(/^\s+save-if:.*$/gm) ?? [];
    const writers = cachePolicies.filter((line) => line.trim() !== 'save-if: false');
    expect(cachePolicies).toHaveLength(8);
    expect(writers).toHaveLength(4);
    for (const writer of writers) {
      expect(writer.trim()).toBe(
        "save-if: ${{ needs.changes.outputs.verify == 'false' && github.ref == 'refs/heads/main' }}"
      );
    }
    for (const name of ['code-quality', 'docker-e2e', 'native-install-gate']) {
      const body = workflow.split(`\n  ${name}:\n`)[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
      expect(body).toContain("always() && needs.changes.outputs.verify != 'false'");
    }
  });

  it('normalizes every CI cache consumer and Colab through one install owner before restore', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8'
    );
    const sites = [
      'native-notices',
      'native-clippy',
      'native-workspace-tests',
      'native-office-build',
      'native-office',
      'native-process-tests',
      'native-msrv',
      'native-runtime-build',
    ];
    expect(workflow.match(/sh scripts\/install-ci-rust\.sh /g)).toHaveLength(sites.length);
    expect(workflow).not.toContain('add-rust-environment-hash-key: false');
    for (const name of sites) {
      const body = workflow.split(`\n  ${name}:\n`)[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
      const install = body.indexOf('sh scripts/install-ci-rust.sh ');
      expect(install, name).toBeGreaterThan(0);
      expect(install, name).toBeLessThan(body.indexOf('uses: Swatinem/rust-cache@'));
      expect(body).toContain(
        name === 'native-msrv'
          ? 'install-ci-rust.sh "$MSRV" --profile minimal'
          : 'install-ci-rust.sh 1.97.0 --profile minimal'
      );
    }
    expect(workflow).toContain('steps: &native-runtime-build-steps');
    expect(workflow).toContain('steps: *native-runtime-build-steps');
    expect(workflow).toContain(
      "install-ci-rust.sh 1.97.0 --profile minimal --target '${{ matrix.target }}'"
    );
    expect(
      workflow.match(/install-ci-rust\.sh 1\.97\.0 --profile minimal --component rustfmt,clippy/g)
    ).toHaveLength(5);
    const colab = readFileSync(
      new URL('../../../.github/workflows/colab-browser.yml', import.meta.url),
      'utf8'
    );
    const install = colab.indexOf('sh scripts/install-ci-rust.sh 1.97.0 --profile minimal');
    expect(colab.match(/sh scripts\/install-ci-rust\.sh /g)).toHaveLength(2);
    expect(install).toBeGreaterThan(0);
    expect(install).toBeLessThan(colab.indexOf('uses: Swatinem/rust-cache@'));
    expect(colab).toContain('shared-key: native-rust');
    expect(colab).toContain('save-if: false');
  });

  it('keeps Docker dependency writes main-only and shard image preparation restore-only', () => {
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    const writer = readFileSync(
      new URL('../../../.github/workflows/e2e-dependency-cache.yml', import.meta.url),
      'utf8'
    );
    const admitted = (consumer: string, owner: string) => {
      const shard = consumer
        .split('steps: &docker-e2e-steps\n')[1]
        ?.split('\n  docker-e2e-shard-2:')[0];
      const seed = owner.split('\n  seed:\n')[1];
      return (
        shard?.includes('version: v0.29.1') === true &&
        shard.includes('cache-from: type=gha,version=2,scope=tmt-e2e-dependencies') &&
        !shard.includes('cache-to:') &&
        !shard.includes('continue-on-error:') &&
        !shard.includes('if: always()') &&
        shard.includes('load: true') &&
        shard.includes('TMT_NATIVE_BASE=ci-native-base') &&
        shard.includes('            TMT_CI_DEPENDENCIES=1') &&
        shard.includes(
          'TMT_E2E_PREPARED_IMAGE: tmux-team-e2e:ci-${{ github.run_id }}-${{ github.job }}'
        ) &&
        shard.indexOf('name: Build the exact-source Docker E2E image') <
          shard.indexOf('run: pnpm test:e2e') &&
        seed?.includes("if: github.event_name == 'push' && github.ref == 'refs/heads/main'") ===
          true &&
        seed.includes('target: ci-dependencies') &&
        seed.includes('build-args: TMT_CI_DEPENDENCIES=1') &&
        seed.includes('cache-to: type=gha,version=2,scope=tmt-e2e-dependencies,mode=min') &&
        !seed.includes('pnpm test:e2e') &&
        owner.includes('group: e2e-dependency-cache-${{ github.ref }}') &&
        owner.includes('cancel-in-progress: false') &&
        owner.includes("      - '**/Cargo.toml'") &&
        owner.includes('      - rust/Cargo.lock') &&
        owner.includes('      - rust/rust-toolchain.toml') &&
        owner.includes('      - typescript/test/e2e/Dockerfile') &&
        [consumer, owner].every(
          (source) =>
            source.includes(
              'docker/setup-buildx-action@e468171a9de216ec08956ac3ada2f0791b6bd435'
            ) &&
            source.includes('docker/build-push-action@263435318d21b8e681c14492fe198d362a7d2c83')
        )
      );
    };
    expect(admitted(ci, writer)).toBe(true);
    expect(admitted(ci.replace('            TMT_CI_DEPENDENCIES=1', ''), writer)).toBe(false);
    expect(admitted(ci, writer.replace('build-args: TMT_CI_DEPENDENCIES=1', ''))).toBe(false);
    expect(
      admitted(
        ci.replace(
          '          cache-from: type=gha,version=2,scope=tmt-e2e-dependencies',
          '          cache-to: type=gha,version=2,scope=tmt-e2e-dependencies'
        ),
        writer
      )
    ).toBe(false);
    expect(
      admitted(
        ci,
        writer.replace(
          "if: github.event_name == 'push' && github.ref == 'refs/heads/main'",
          "if: github.event_name == 'pull_request'"
        )
      )
    ).toBe(false);
    expect(
      admitted(
        ci,
        writer.replace('scope=tmt-e2e-dependencies,mode=min', 'scope=tmt-e2e-dependencies,mode=max')
      )
    ).toBe(false);
    expect(admitted(ci, writer.replace("      - '**/Cargo.toml'\n", ''))).toBe(false);
    const mainPush = ci.split('\n  push:\n')[1].split('\n  schedule:\n')[0];
    expect(mainPush).not.toContain('**/Cargo.toml');
    expect(writer).not.toMatch(/^  (pull_request|qualification):/m);
  });

  it('gives every native step and job an explicit scope, and gates on exactly those results', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const job = (name: string) => {
      const start = workflow.indexOf(`\n  ${name}:\n`);
      const next = workflow.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return workflow.slice(start, next < 0 ? undefined : start + 1 + next);
    };
    const steps = (body: string) =>
      body
        .split('\n      - ')
        .slice(1)
        .map((step) => ({
          name: /^name: (.*)$/m.exec(step)?.[1] ?? '',
          scope: /^ {8}if: (.*)$/m.exec(step)?.[1],
          body: step,
        }));

    const changes = job('changes');
    for (const output of ['scoped_native_tests', 'e2e_shard_1', 'e2e_shard_2']) {
      expect(changes).toContain(
        `${output}: \${{ steps.scope.outputs.${output} || steps.queue.outputs.${output} }}`
      );
    }
    // A seeding run (no pull request, no diff) builds the whole workspace, so it is the full scope.
    expect(changes).toContain(
      'native_scope: ${{ steps.scope.outputs.native_scope || steps.queue.outputs.native_scope || steps.seed.outputs.native_scope }}'
    );
    expect(changes).toContain('node typescript/scripts/ci-scope.mjs "$SEED_SCOPE"');
    expect(changes).toContain(
      "(github.event_name == 'schedule' || github.event_name == 'workflow_dispatch') && 'full' || 'seed'"
    );
    expect(changes).toContain(
      'native_office: ${{ steps.scope.outputs.native_office || steps.queue.outputs.native_office || steps.seed.outputs.native_office }}'
    );
    // Jobs about the CLI runtime and tooling run only for the full scope; the runtime build also
    // runs for the seeding runs, whose scope is full, and the other two only for verification events.
    expect(job('native-runtime-build')).toContain(
      "if: needs.changes.outputs.native_scope == 'full'"
    );
    for (const name of ['unit-tests', 'packed-native-install']) {
      expect(job(name), name).toContain(
        "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_scope == 'full'"
      );
    }
    // The Rust job runs for every native scope, the E2E job for every native scope of a pull request.
    for (const name of ['native-clippy', 'native-workspace-tests', 'native-msrv']) {
      expect(job(name)).toContain("if: needs.changes.outputs.native == 'true'");
    }
    const rustGate = job('native-rust');
    expect(rustGate.replace(/\s+/g, ' ')).toContain(
      'needs: [ changes, native-clippy, native-workspace-tests, native-office, native-process-tests, native-msrv, ]'
    );
    expect(rustGate).toContain(
      "if: ${{ always() && needs.changes.outputs.native_scope != 'none' }}"
    );
    for (const [variable, worker] of [
      ['CLIPPY_RESULT', 'native-clippy'],
      ['TESTS_RESULT', 'native-workspace-tests'],
      ['OFFICE_RESULT', 'native-office'],
      ['NATIVE_RESULT', 'native-process-tests'],
    ])
      expect(rustGate).toContain(`${variable}: \${{ needs.${worker}.result }}`);
    expect(rustGate).toContain('MSRV_RESULT: ${{ needs.native-msrv.result }}');
    const rustVariables = {
      clippy: 'CLIPPY_RESULT',
      tests: 'TESTS_RESULT',
      office: 'OFFICE_RESULT',
      process: 'NATIVE_RESULT',
      msrv: 'MSRV_RESULT',
    };
    const rustArguments = RUST_WORKERS.map((worker) => `"$${rustVariables[worker]}"`).join(' ');
    expect(rustGate).toContain(
      `ci-scope.mjs gate-rust "$NATIVE_SCOPE" "$NATIVE_OFFICE_SELECTED" ${rustArguments}`
    );
    expect(job('native-msrv')).toContain('shared-key: native-rust-msrv');
    expect(job('native-msrv')).toContain(
      "save-if: ${{ needs.changes.outputs.verify == 'false' && github.ref == 'refs/heads/main' }}"
    );
    expect(job('native-msrv')).toContain('["workspace"]["package"]["rust-version"]');
    expect(job('native-msrv')).toContain('RUSTUP_TOOLCHAIN=%s');
    expect(job('native-msrv')).toContain('sh scripts/install-ci-rust.sh "$MSRV" --profile minimal');
    expect(job('native-msrv')).not.toMatch(/install-ci-rust\.sh \d/);
    expect(job('native-msrv')).toContain(
      'cargo +"$MSRV" check --locked --workspace --exclude tmt-office --exclude tmt-office-storage --exclude tmt-office-pairing --exclude tmt-office-service --all-targets'
    );
    // The E2E suite is two shard jobs; only the first runs for a scoped component, and
    // only the first of a full run also runs the Rust adapter tests.
    expect(job('docker-e2e-shard-1')).toContain(
      "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native == 'true'"
    );
    expect(job('docker-e2e-shard-1')).toContain(
      'TMT_E2E_FILES: ${{ needs.changes.outputs.e2e_shard_1 }}'
    );
    expect(job('docker-e2e-shard-1')).toContain(
      "TMT_E2E_ADAPTER_TESTS: ${{ needs.changes.outputs.native_scope == 'full' && '1' || '0' }}"
    );
    expect(job('docker-e2e-shard-2')).toContain(
      "if: needs.changes.outputs.verify == 'true' && needs.changes.outputs.native_scope == 'full'"
    );
    expect(job('docker-e2e-shard-2')).toContain(
      'TMT_E2E_FILES: ${{ needs.changes.outputs.e2e_shard_2 }}'
    );
    expect(job('docker-e2e-shard-2')).toContain("TMT_E2E_ADAPTER_TESTS: '0'");
    // Both shards run the same steps (a YAML alias), so they cannot drift apart.
    expect(job('docker-e2e-shard-1')).toContain('steps: &docker-e2e-steps');
    expect(job('docker-e2e-shard-2')).toContain('steps: *docker-e2e-steps');
    // `Docker E2E`, the required check, is a gate over exactly the two shards.
    const e2eGate = job('docker-e2e');
    expect(e2eGate).toContain('name: Docker E2E\n');
    expect(e2eGate).toContain('needs: [changes, docker-e2e-shard-1, docker-e2e-shard-2]');
    // Like the other gates, only for verification events: a seeding run has no gate to satisfy.
    expect(e2eGate).toContain("if: ${{ always() && needs.changes.outputs.verify != 'false' }}");
    expect(e2eGate).toContain('SHARD_1_RESULT: ${{ needs.docker-e2e-shard-1.result }}');
    expect(e2eGate).toContain('SHARD_2_RESULT: ${{ needs.docker-e2e-shard-2.result }}');
    expect(e2eGate).toContain(
      'ci-scope.mjs gate-e2e "$NATIVE_SCOPE" "$SHARD_1_RESULT" "$SHARD_2_RESULT"'
    );
    // Each required check name belongs to exactly one job, and no shard reuses one.
    for (const required of [
      'Code quality',
      'Unit tests',
      'Docker E2E',
      'Native Rust contracts',
      'Native package matrix',
    ]) {
      expect(workflow.match(new RegExp(`^ {4}name: ${required}$`, 'gm')), required).toHaveLength(1);
    }
    expect(workflow.match(/^ {4}name: Docker E2E shard \d\/2$/gm)).toHaveLength(2);
    expect(job('unit-tests')).toContain(
      'name: runtime-x86_64-unknown-linux-musl\n          path: rust/target/debug'
    );
    expect(job('native-office')).toContain("if: needs.changes.outputs.native_scope == 'full'");
    expect(job('native-process-tests')).toContain('needs: [changes, native-office-build]');
    expect(job('native-process-tests')).toContain('timeout-minutes: 20');
    expect(job('native-process-tests')).toContain(
      'uses: ./.github/actions/apt-install\n        with:\n          packages: zsh'
    );
    expect(job('native-process-tests')).toContain(
      "needs.changes.outputs.native_office == 'false' || (needs.changes.outputs.native_office == 'true' && needs.native-office-build.result == 'success')"
    );
    expect(job('native-clippy')).toContain('Reject an unknown native scope');
    const scoped = (worker: string, scope: string) =>
      steps(job(worker))
        .filter((step) => step.scope === `needs.changes.outputs.native_scope == '${scope}'`)
        .map((step) => step.name);
    expect(scoped('native-clippy', 'full')).toEqual(['Lint workspace']);
    expect(scoped('native-clippy', 'ops')).toEqual(['Lint Ops']);
    expect(scoped('native-workspace-tests', 'full')).toEqual(['Test and build workspace']);
    expect(scoped('native-workspace-tests', 'ops')).toEqual(['Test Ops and architecture']);
    expect(scoped('native-process-tests', 'full')).toEqual([
      'Build independent native process fixtures',
      'Verify native process and shared parser contracts',
    ]);
    expect(scoped('native-process-tests', 'ops')).toEqual([
      'Build Ops process fixtures',
      'Verify Ops native contracts',
    ]);
    // Both scopes run extension-install, whose archives use these debug executables.
    for (const scope of ['full', 'ops']) {
      const fixtures = steps(job('native-process-tests')).find(
        (step) =>
          step.scope === `needs.changes.outputs.native_scope == '${scope}'` &&
          step.name.startsWith('Build ')
      );
      const debugBuilds = [...(fixtures?.body.matchAll(/cargo build ([^\n]+)/g) ?? [])]
        .map((match) => match[1])
        .filter((args) => !args.includes('--release'));
      for (const product of ['tmt-ops', 'tmt-remote', 'tmt-colab']) {
        expect(
          debugBuilds.some((args) => new RegExp(`(?:^|\\s)-p\\s+${product}(?=\\s|$)`).test(args)),
          `${scope} native process fixtures must build ${product} in debug`
        ).toBe(true);
      }
      expect(
        debugBuilds.some((args) => /(?:^|\s)--example\s+runtime-caller-fixture(?=\s|$)/.test(args)),
        `${scope} native process fixtures must build the shared ancestry launcher`
      ).toBe(true);
      expect(
        debugBuilds.some((args) =>
          /(?:^|\s)-p\s+tmt-test-support\s+--example\s+recording-cli-fixture(?=\s|$)/.test(args)
        ),
        `${scope} native process fixtures must build the native recording driver`
      ).toBe(true);
    }
    // The producer verifies the feature build; native tests consume exactly those bytes.
    expect(job('native-office-build')).toContain('sha256sum tmt-office > tmt-office.sha256');
    expect(job('native-office')).toContain('name: native-office-companion');
    expect(job('native-process-tests')).toContain('name: native-office-companion');
    expect(job('native-process-tests')).toContain('sha256sum --check tmt-office.sha256');
    expect(job('native-process-tests')).toContain('chmod +x tmt-office');
    expect(workflow).toContain('env:\n  CARGO_PROFILE_DEV_DEBUG: 0\n  CARGO_INCREMENTAL: 0');
    // One main-only writer serves the shared dev cache; parallel readers never save.
    for (const worker of ['native-clippy', 'native-office-build', 'native-office']) {
      expect(job(worker)).toContain('shared-key: native-rust');
      expect(job(worker)).toContain('save-if: false');
    }
    for (const worker of [
      'native-workspace-tests',
      'native-runtime-build',
      'native-msrv',
      'native-process-tests',
    ]) {
      expect(job(worker)).toContain(
        "save-if: ${{ needs.changes.outputs.verify == 'false' && github.ref == 'refs/heads/main' }}"
      );
    }
    // The process job seeds its existing dev/release builds in an isolated main-only cache.
    const processCache = job('native-process-tests');
    expect(processCache).toContain('shared-key: native-process-rust');
    expect(processCache).not.toContain('shared-key: native-rust');
    expect(processCache).not.toMatch(/cache-workspace-crates:|cache-on-failure:/);
    const save = /^ {10}save-if: (.+)$/m.exec(processCache)?.[1];
    expect(save).toBeDefined();
    const evaluateSave = new Function(
      'needs',
      'github',
      `return (${save!.replace(/^\$\{\{\s*|\s*\}\}$/g, '')})`
    );
    for (const ref of [
      'refs/heads/main',
      'refs/heads/feature',
      'refs/pull/1/merge',
      'refs/heads/gh-readonly-queue/main/pr-1',
      'refs/tags/v5.0.0-alpha.1',
    ]) {
      for (const verify of ['false', 'true', '']) {
        for (const scope of ['full', 'ops']) {
          expect(
            evaluateSave({ changes: { outputs: { verify, native_scope: scope } } }, { ref }),
            `${ref} verify=${verify} scope=${scope}`
          ).toBe(ref === 'refs/heads/main' && verify === 'false');
        }
      }
    }
    // The gate names the same eight jobs, in the order nativeGatePasses expects.
    const gate = job('native-install-gate');
    expect(gate).toContain('NATIVE_SCOPE: ${{ needs.changes.outputs.native_scope }}');
    expect(gate).toContain(
      'ci-scope.mjs gate-native "$MACOS_SELECTED" "$NATIVE_SCOPE" "$CONTRACT_RESULT" "$UNIT_RESULT" "$SHARD_1_RESULT" "$SHARD_2_RESULT" "$BUILD_RESULT" "$MATRIX_RESULT" "$MACOS_BUILD_RESULT" "$MACOS_MATRIX_RESULT"'
    );
    for (const [variable, jobName] of [
      ['CONTRACT_RESULT', 'native-rust'],
      ['UNIT_RESULT', 'unit-tests'],
      ['SHARD_1_RESULT', 'docker-e2e-shard-1'],
      ['SHARD_2_RESULT', 'docker-e2e-shard-2'],
      ['BUILD_RESULT', 'native-runtime-build'],
      ['MATRIX_RESULT', 'packed-native-install'],
      ['MACOS_BUILD_RESULT', 'native-runtime-build-macos'],
      ['MACOS_MATRIX_RESULT', 'packed-native-install-macos'],
    ]) {
      expect(gate).toContain(`${variable}: \${{ needs.${jobName}.result }}`);
    }
  });

  it('keeps the complete Colab harness separate from required gates and PR cache writes', () => {
    const workflow = readFileSync(
      new URL('../../../.github/workflows/colab-browser.yml', import.meta.url),
      'utf8'
    );
    const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
    expect(ci).not.toContain('colab_harness');
    expect(ci).not.toContain('needs.colab-browser');
    expect(workflow).toContain('permissions: {}');
    expect(workflow).not.toContain('paths:');
    expect(workflow).not.toContain('pull_request_target:');
    expect(workflow).not.toContain('merge_group:');
    expect(workflow).not.toContain('continue-on-error');
    expect(workflow).not.toContain('\n  push:');
    expect(workflow).toContain('workflow_dispatch:');
    expect(workflow).toContain("cron: '23 5 * * 1'");
    expect(workflow).not.toMatch(/^concurrency:/m);
    expect(workflow).toContain('types: [opened, synchronize, reopened, labeled, unlabeled]');
    expect(ci).not.toContain('needs.colab-acceptance');
    const job = (name: string) => {
      const start = workflow.indexOf(`\n  ${name}:\n`);
      const next = workflow.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return workflow.slice(start, next < 0 ? undefined : start + 1 + next);
    };
    for (const name of ['changes', 'colab-browser']) {
      const body = job(name);
      expect(body).toMatch(/^ {4}concurrency:\n {6}group: colab-browser-/m);
      expect(body).toContain("cancel-in-progress: ${{ github.event_name == 'pull_request' }}");
    }
    expect(job('changes')).toContain(
      "github.event.action != 'labeled' && github.event.action != 'unlabeled'"
    );
    expect(job('colab-browser')).toContain('needs: changes');
    const app = job('colab-app');
    expect(app).toContain('needs: changes');
    expect(app).toMatch(/^ {4}concurrency:\n {6}group: colab-browser-app-/m);
    expect(app).toContain("cancel-in-progress: ${{ github.event_name == 'pull_request' }}");
    expect(app).toContain(
      "if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' || needs.changes.outputs.colab_app == 'true'"
    );
    expect(app).toContain('permissions:\n      contents: read');
    expect(app).toContain('pnpm --filter @tmt/colab-app --fail-if-no-match test:browser');
    expect(app).toContain('playwright install --with-deps chromium');
    expect(app).toContain('retention-days: 7');
    expect(app).not.toContain('COLAB_SERVE_EXECUTABLE');
    expect(app).not.toContain('cargo');
    expect(workflow).toContain('ci-scope.mjs "$BASE_SHA" "$HEAD_SHA" >> "$GITHUB_OUTPUT"');
    expect(workflow).toContain(
      "if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' || needs.changes.outputs.colab_harness == 'true'"
    );
    expect(workflow).toContain('playwright install --with-deps chromium firefox webkit');
    expect(workflow).toContain('test:browser --engines chromium 2>&1 | tee');
    expect(workflow).toContain('--example browser_conformance --example browser_authority');
    expect(workflow).toContain('shell: bash');
    expect(workflow).toContain('test:browser 2>&1 | tee');
    expect(workflow).toContain('if: always()');
    expect(workflow).toContain('retention-days: 7');
    expect(workflow).toContain('actions/cache/restore@v4');
    expect(workflow).toContain(
      'colab-playwright-${{ runner.os }}-${{ runner.arch }}-${{ steps.playwright.outputs.version }}'
    );
    expect(workflow).toContain(
      "if: github.ref == 'refs/heads/main' && steps.browsers.outputs.cache-hit != 'true'"
    );
    expect(workflow).toContain('actions/cache/save@v4');
    expect(workflow).toContain('shared-key: native-rust\n          save-if: false');
    const acceptance = job('colab-acceptance');
    expect(acceptance).not.toContain('needs:');
    expect(acceptance).toContain(
      "github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' ||"
    );
    expect(acceptance).toContain(
      "github.event_name == 'pull_request' && github.event.action != 'unlabeled'"
    );
    expect(acceptance).toContain(
      "contains(github.event.pull_request.labels.*.name, 'colab-acceptance')"
    );
    expect(acceptance).toContain(
      "github.event.action != 'labeled' || github.event.label.name == 'colab-acceptance'"
    );
    expect(acceptance).toMatch(/^ {4}concurrency:\n {6}group: colab-acceptance-/m);
    expect(acceptance).toContain('cancel-in-progress: false');
    expect(acceptance).toContain('runs-on: ubuntu-24.04');
    expect(acceptance).toContain('timeout-minutes: 30');
    expect(acceptance).toContain('permissions:\n      contents: read');
    expect(acceptance).not.toContain('CARGO_BUILD_JOBS:');
    expect(acceptance).toContain('cargo build --locked --manifest-path rust/Cargo.toml -j 2');
    expect(acceptance).toContain(
      'pnpm --filter @tmt/colab-app... --fail-if-no-match install --frozen-lockfile --ignore-scripts'
    );
    expect(acceptance).toContain(
      'uses: ./.github/actions/apt-install\n        with:\n          packages: tmux'
    );
    expect(acceptance).toContain('sh scripts/install-ci-rust.sh 1.97.0 --profile minimal');
    expect(acceptance).toContain('shared-key: native-rust\n          save-if: false');
    expect(acceptance).not.toContain('actions/cache/save');
    // A stale PR head may lack helpers added on main, including the always-run report step.
    // Default checkout tests the merge ref, where those workflow inputs are integrated.
    expect(acceptance).not.toMatch(/^\s*ref:/m);
    expect(acceptance).toContain('PR_HEAD_SHA: ${{ github.event.pull_request.head.sha }}');
    expect(acceptance).toContain(
      'printf \'pr-head: %s\\ntested-merge: %s\\n\' "$PR_HEAD_SHA" "$(git rev-parse HEAD)" > "$RUNNER_TEMP/colab-acceptance-results/tested-head.txt"'
    );
    expect(acceptance).toContain('persist-credentials: false');
    const appBuild = acceptance.indexOf('pnpm --filter @tmt/colab-app --fail-if-no-match build');
    const nativeBuild = acceptance.indexOf('cargo build --locked --manifest-path rust/Cargo.toml');
    expect(appBuild).toBeGreaterThan(0);
    expect(nativeBuild).toBeGreaterThan(appBuild);
    expect(acceptance.match(/cargo build /g)).toHaveLength(1);
    expect(acceptance).toContain('-p tmt-cli -p tmt-remote -p tmt-colab --bins');
    expect(acceptance).toContain('test:acceptance --workers=1 --reporter=list,json');
    expect(acceptance).not.toContain('continue-on-error');
    expect(acceptance).not.toContain('|| true');
    const upload = acceptance.slice(
      acceptance.indexOf('      - name: Upload public acceptance evidence')
    );
    expect(upload).toContain('if: always()');
    expect(upload).toContain('path: ${{ runner.temp }}/colab-acceptance-results');
    expect(upload).toContain('retention-days: 7');
    expect(upload).not.toContain('colab-acceptance-private');
    expect(acceptance).toContain('node typescript/scripts/colab-acceptance-report.mjs');
  });

  it('runs the advisory browser partitions in their own workflow from one shared image', () => {
    const read = (name: string) =>
      readFileSync(
        fileURLToPath(new URL(`../../../.github/workflows/${name}`, import.meta.url)),
        'utf8'
      );
    const ci = read('ci.yml');
    const browser = read('office-browser.yml');
    const job = (workflow: string, name: string) => {
      const start = workflow.indexOf(`\n  ${name}:\n`);
      const next = workflow.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return workflow.slice(start, next < 0 ? undefined : start + 1 + next);
    };

    // CI keeps only what its required results depend on, so a red run means a
    // required or selected check failed.
    expect(ci).not.toContain('office-browser:');
    expect(ci).not.toContain('native-office-browser');
    expect(ci).not.toContain('tmt-office-browser');

    // The image is built exactly once, and no partition builds it again.
    expect(browser.match(/docker build /g)).toHaveLength(1);
    expect(job(browser, 'image')).toContain('docker build --target browser-tests');
    expect(job(browser, 'image')).toContain('docker save tmt-office-browser:ci');
    for (const partitions of ['office-emulator', 'office-local', 'native-office-browser']) {
      const body = job(browser, partitions);
      expect(body).toContain('actions/download-artifact');
      expect(body).toContain('docker load');
      expect(body).not.toContain('docker build');
    }
    // The selector decides the emulator partition of a pull request.
    expect(job(browser, 'office-emulator')).toContain('needs: [changes, image]');
    expect(job(browser, 'office-emulator')).toContain(
      "needs.changes.outputs.office_browser == 'true'"
    );
  });

  it('excludes only retired Office product targets while retaining shipped CLI libraries', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const commands = workflow.split('\n').filter((line) => line.includes('--workspace --exclude'));
    expect(commands).toHaveLength(4);
    for (const command of commands) {
      expect([...command.matchAll(/--exclude ([a-z0-9-]+)/g)].map((match) => match[1])).toEqual([
        'tmt-office',
        'tmt-office-storage',
        'tmt-office-pairing',
        'tmt-office-service',
      ]);
      expect(command).not.toMatch(/--exclude tmt-office-(?:command|model)\b/);
    }
    const manifest = readFileSync(
      fileURLToPath(new URL('../../../rust/crates/tmt-cli/Cargo.toml', import.meta.url)),
      'utf8'
    );
    expect(manifest).toContain('tmt-office-command.workspace = true');
    expect(manifest).toContain('tmt-office-model.workspace = true');
  });

  it('retains Office workflow history but disables automatic and manual execution', () => {
    const browser = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/office-browser.yml', import.meta.url)),
      'utf8'
    );
    expect(browser).toMatch(/^on:\n {2}workflow_dispatch:\n/m);
    expect(browser).not.toMatch(/^ {2}(?:pull_request|schedule):/m);
    expect(browser).not.toContain('cron:');
    const job = (name: string) => {
      const start = browser.indexOf(`\n  ${name}:\n`);
      expect(start, name).toBeGreaterThanOrEqual(0);
      const next = browser.slice(start + 1).search(/\n {2}[a-z0-9-]+:\n/);
      return browser.slice(start, next < 0 ? undefined : start + 1 + next);
    };
    expect(job('changes')).toContain(
      "office_browser: ${{ steps.scope.outputs.office_browser || 'false' }}"
    );
    expect(job('changes').match(/if: github\.event_name == 'pull_request'/g)).toHaveLength(3);
    for (const name of ['image', 'office-emulator', 'office-local', 'native-office-browser']) {
      expect(job(name)).toContain("if: needs.changes.outputs.office_browser == 'true'");
    }
    expect(job('office-emulator')).toContain('name: Office browser (emulator)');
    expect(job('office-local')).toContain('name: Office browser (local ${{ matrix.shard }})');
    expect(job('office-emulator')).toContain('name: office-browser-emulator-results');
    expect(job('office-local')).toContain('name: office-browser-${{ matrix.partition }}-results');
  });
});

it.each(['Cargo.toml', 'src/store.rs', 'tests/state.rs'])(
  'keeps released Colab Rust %s in full workspace CI',
  (suffix) => {
    const files = ['extensions/tmt-colab/rust/tmt-colab/' + suffix];
    expect(explainCiSelection(files)).toMatchObject([
      { owner: 'tmt-colab', rule: 'colab-rust', native: true, office: false, nativeOffice: false },
    ]);
    expect(selectNativeScope(files)).toBe('full');
    expect(
      isReleased(
        parseComponentMap(
          readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
        ),
        'tmt-colab'
      )
    ).toBe(true);
  }
);

it('schedules architecture CI for the map, every declaring crate file and release script', () => {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const map = parseComponentMap(readFileSync(`${root}.github/components.json`, 'utf8'));
  const workspace = readCargoWorkspace(root);
  const files: string[] = runPackedCommand('git', ['ls-files', '-z'], {
    cwd: root,
    env: process.env,
  })
    .split('\0')
    .filter(Boolean);
  const protectedPaths = ['.github/components.json', 'scripts/build-native-artifact.sh'];
  for (const component of map.components.filter(
    (c) => c.neverShippedPaths.length || c.generatedInputs.length
  )) {
    const crate = workspace.packages.find((p) => p.name === component.package)!;
    expect(crate).toBeDefined();
    protectedPaths.push(
      ...files.filter((path) => path.startsWith(`${crate.dir}/`)),
      `${crate.dir}/new-input.md`
    );
  }
  for (const path of protectedPaths) {
    expect(selectCiAreas([path], map).native, path).toBe(true);
    expect(selectNativeScope([path], map), path).toBe('full');
  }
  const workflow = readFileSync(`${root}.github/workflows/ci.yml`, 'utf8');
  const job = workflow.split('\n  native-workspace-tests:\n')[1].split(/\n {2}[a-z0-9-]+:\n/)[0];
  expect(job).toContain("if: needs.changes.outputs.native == 'true'");
  expect(job).toContain("if: needs.changes.outputs.native_scope == 'full'");
  expect(job).toContain('cargo test --locked --workspace');
});
