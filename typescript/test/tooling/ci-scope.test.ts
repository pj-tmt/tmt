import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, renameSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { ciGatePasses, readChangedCiAreas, selectCiAreas } from '../../scripts/ci-scope.mjs';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);

describe('CI area selection', () => {
  it('selects Office without the native matrix for app-only changes', () => {
    expect(
      selectCiAreas([
        'extensions/tmt-office/typescript/apps/office/src/main.tsx',
        'docs/office/architecture.md',
      ])
    ).toEqual({
      native: false,
      office: true,
    });
  });

  it('selects native code and embedded skill consumers without Office', () => {
    expect(selectCiAreas(['rust/crates/tmt-core/src/lib.rs', 'skills/tmux-team/SKILL.md'])).toEqual(
      { native: true, office: false }
    );
  });

  it.each([
    'typescript/pnpm-lock.yaml',
    'typescript/pnpm-workspace.yaml',
    'typescript/package.json',
    '.github/workflows/ci.yml',
    'typescript/scripts/ci-scope.mjs',
    'typescript/test/e2e/Dockerfile',
    'contracts/office/request.json',
    'extensions/tmt-office/rust/tmt-office/src/main.rs',
    'extensions/tmt-office/typescript/services/office/firestore.rules',
    'typescript/apps/office/src/main.tsx',
    'new-owner/file.ts',
  ])('fans out shared or unknown input %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: true, office: true });
  });

  it('does not confuse similar prefixes and fails closed on an empty diff', () => {
    expect(selectCiAreas(['extensions/tmt-office/typescript/apps/office-other/file.ts'])).toEqual({
      native: true,
      office: true,
    });
    expect(selectCiAreas([])).toEqual({ native: true, office: true });
  });

  it('unions mixed paths including both sides of a no-renames diff', () => {
    expect(
      selectCiAreas(['extensions/tmt-office/typescript/apps/office/removed.ts', 'rust/new.rs'])
    ).toEqual({
      native: true,
      office: true,
    });
    expect(selectCiAreas(['extensions/tmt-office/typescript/apps/office/deleted.ts'])).toEqual({
      native: false,
      office: true,
    });
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
      mkdirSync(path.join(root, 'apps/office'), { recursive: true });
      const historicalSource = path.join(root, 'apps/office/name with\nnewline.ts');
      writeFileSync(historicalSource, 'export const fixture = true;\n');
      const historical = commit();
      expect(readChangedCiAreas(base, historical, root)).toEqual({ native: false, office: true });
      mkdirSync(path.join(root, 'extensions/tmt-office/typescript/apps/office'), {
        recursive: true,
      });
      const source = path.join(
        root,
        'extensions/tmt-office/typescript/apps/office/name with\nnewline.ts'
      );
      renameSync(historicalSource, source);
      const added = commit();
      expect(readChangedCiAreas(historical, added, root)).toEqual({ native: false, office: true });
      mkdirSync(path.join(root, 'rust'));
      const target = path.join(root, 'rust/fixture.rs');
      renameSync(source, target);
      const moved = commit();
      expect(readChangedCiAreas(added, moved, root)).toEqual({ native: true, office: true });
      rmSync(target);
      const deleted = commit();
      expect(readChangedCiAreas(moved, deleted, root)).toEqual({ native: true, office: false });
      expect(() => readChangedCiAreas('--help', deleted, root)).toThrow('exact base and head');
      expect(() => readChangedCiAreas('0'.repeat(40), deleted, root)).toThrow(
        'Packed command failed'
      );
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

  it('fails both stable aggregates when selector output is unavailable', () => {
    expect(ciGatePasses('', ['skipped'])).toBe(false);
    expect(ciGatePasses('', ['skipped', 'skipped', 'skipped', 'skipped', 'skipped'])).toBe(false);
  });

  it('builds the selected release CLI without replacing debug Rust verification', () => {
    const workflow = readFileSync(
      fileURLToPath(new URL('../../../.github/workflows/ci.yml', import.meta.url)),
      'utf8'
    );
    const start = workflow.indexOf('\n  native-rust:\n');
    const native = workflow.slice(start, workflow.indexOf('\n  unit-tests:\n', start));
    expect(native).toContain('cargo build --locked --release -p tmt-cli');
    expect(native).toContain('rust/target/release/tmt');
    expect(native).toContain('cargo test --locked');
    expect(native).toContain('cargo clippy --locked --all-targets -- -D warnings');
    expect(native).toContain('cargo +1.88.0 build --locked');
    expect(native).toContain('cargo build --locked -p tmt-office');
    expect(native).toContain('rust/target/debug/examples/storage-probe');
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
});
