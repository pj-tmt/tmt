import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
  rmSync,
  renameSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  NATIVE_OFFICE_UNREACHABLE,
  ciGatePasses,
  readChangedCiAreas,
  selectCiAreas,
} from '../../scripts/ci-scope.mjs';

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
      nativeOffice: true,
    });
  });

  it('selects native code and embedded skill consumers without Office', () => {
    expect(
      selectCiAreas(['rust/crates/tmt-adapters/src/setup.rs', 'skills/tmux-team/SKILL.md'])
    ).toEqual({ native: true, office: false, nativeOffice: false });
  });

  it('treats the squad extension as native-only, not a prefix look-alike', () => {
    expect(
      selectCiAreas([
        'extensions/tmt-squad/rust/tmt-squad/src/main.rs',
        'extensions/tmt-squad/skills/tmt-squad/SKILL.md',
      ])
    ).toEqual({ native: true, office: false, nativeOffice: false });
    expect(selectCiAreas(['extensions/tmt-squad-other/file.rs'])).toEqual({
      native: true,
      office: true,
      nativeOffice: true,
    });
  });

  it.each([
    'typescript/pnpm-lock.yaml',
    'typescript/pnpm-workspace.yaml',
    'typescript/package.json',
    '.github/workflows/ci.yml',
    'typescript/scripts/ci-scope.mjs',
    'contracts/office/request.json',
    'extensions/tmt-office/contracts/request.json',
    'extensions/tmt-office/skills/tmt-office/SKILL.md',
    'extensions/tmt-office/rust/tmt-office/src/main.rs',
    'extensions/tmt-office/typescript/services/office/firestore.rules',
    'typescript/apps/office/src/main.tsx',
    'new-owner/file.ts',
  ])('fans out shared or unknown input %s', (file) => {
    expect(selectCiAreas([file])).toEqual({ native: true, office: true, nativeOffice: true });
  });

  it('schedules native Office browser shards only for Office and the core it consumes', () => {
    const coreOnly = { native: true, office: false, nativeOffice: false };
    expect(
      selectCiAreas([
        'extensions/tmt-squad/rust/tmt-squad/src/board.rs',
        'extensions/tmt-squad/skills/tmt-squad/SKILL.md',
      ])
    ).toEqual(coreOnly);
    expect(
      selectCiAreas([
        'rust/crates/tmt-adapters/src/setup/providers.rs',
        'rust/crates/tmt-cli/src/main.rs',
        'rust/crates/tmt-cli/src/install_command.rs',
      ])
    ).toEqual(coreOnly);
    for (const consumed of [
      'rust/crates/tmt-core/src/request.rs',
      'rust/crates/tmt-core/src/room.rs',
      'rust/crates/tmt-adapters/src/room.rs',
      'rust/crates/tmt-adapters/src/api.rs',
      'rust/crates/tmt-adapters/src/delivery.rs',
      'rust/crates/tmt-adapters/src/pane_badge.rs',
      'rust/crates/tmt-adapters/src/skill_installation/owned.rs',
      'rust/crates/tmt-adapters/src/runtime_like.rs',
      'rust/crates/tmt-adapters/src/runtime/claude.rs',
      'rust/crates/tmt-adapters/src/runtime_caller/probe.rs',
      'rust/crates/tmt-adapters/Cargo.toml',
      'rust/crates/tmt-command-output/src/lib.rs',
      'rust/crates/tmt-future/src/lib.rs',
      'rust/crates/tmt-cli/src/api_command.rs',
      'rust/crates/tmt-cli/src/native_upgrade_command.rs',
      'rust/crates/tmt-adapters/src/storage/migrations.rs',
      'rust/crates/tmt-adapters/src/office_service.rs',
      'rust/crates/tmt-adapters/src/office_companion/process.rs',
      'rust/crates/tmt-adapters/src/native_install/archive.rs',
      'rust/crates/tmt-core/src/native_install.rs',
      'rust/crates/tmt-cli/src/office_facade.rs',
      'rust/Cargo.lock',
      'rust/rust-toolchain.toml',
    ]) {
      expect(selectCiAreas(['extensions/tmt-squad/rust/tmt-squad/src/board.rs', consumed])).toEqual(
        { native: true, office: false, nativeOffice: true }
      );
    }
    // Prose, core-only suites and E2E scenarios fan out to native and Office
    // checks, but the native Office image never reads them.
    expect(
      selectCiAreas([
        'ARCHITECTURE.md',
        'docs/extension-api.md',
        'typescript/test/native/api.test.ts',
        'typescript/test/tooling/ci-scope.test.ts',
        'typescript/test/e2e/squad.e2e.test.ts',
        'typescript/test/e2e/Dockerfile',
      ])
    ).toEqual({ native: true, office: true, nativeOffice: false });
    for (const read of [
      'docs/office/architecture.md',
      'extensions/tmt-office/README.md',
      'typescript/test/e2e/harness.ts',
      'typescript/test/support/office-world.ts',
      'contracts/office/profile-v1.md',
      'docs/office.md.orig',
    ]) {
      expect(selectCiAreas(['ARCHITECTURE.md', read]).nativeOffice).toBe(true);
    }
    expect(selectCiAreas(['extensions/tmt-office/rust/tmt-office/src/main.rs'])).toEqual({
      native: true,
      office: true,
      nativeOffice: true,
    });
    expect(selectCiAreas(['extensions/tmt-office/typescript/apps/office/src/main.tsx'])).toEqual({
      native: false,
      office: true,
      nativeOffice: true,
    });
  });

  it('denies only crate modules unreachable from the Office crates and the API', () => {
    const repository = fileURLToPath(new URL('../../../', import.meta.url));
    const read = (file: string) => readFileSync(path.join(repository, file), 'utf8');
    const rustFiles = (directory: string): string[] =>
      readdirSync(path.join(repository, directory), { recursive: true, encoding: 'utf8' })
        .filter((file) => file.endsWith('.rs'))
        .map((file) => path.join(directory, file));
    // Every shared workspace crate; tmt-cli keeps its explicit path list instead.
    const crates = readdirSync(path.join(repository, 'rust/crates')).filter(
      (crate) => crate !== 'tmt-cli'
    );
    const identifier = (crate: string) => crate.replaceAll('-', '_');
    const moduleOf = (file: string, crate: string) =>
      path.relative(`rust/crates/${crate}/src`, file).split(path.sep)[0].replace(/\.rs$/, '');
    // Top-level modules named by `prefix::module` or a (nested) `prefix::{...}` group.
    const referenced = (text: string, prefix: string) =>
      [...text.matchAll(new RegExp(`\\b${prefix}::(\\{|\\w+)`, 'g'))].flatMap((match) => {
        if (match[1] !== '{') return [match[1]];
        const names: string[] = [];
        let depth = 0;
        let item = '';
        for (const character of text.slice(match.index + match[0].length - 1)) {
          if (character === '{' && depth++ === 0) continue;
          if (character === '}' && --depth === 0) break;
          if (depth === 1 && character === ',') {
            names.push(item);
            item = '';
          } else if (depth === 1) item += character;
        }
        return [...names, item].map((part) => /\w+/.exec(part)?.[0] ?? '').filter(Boolean);
      });
    const sources = (crate: string, name: string) =>
      rustFiles(`rust/crates/${crate}/src`)
        .filter((file) => moduleOf(file, crate) === name)
        .map(read)
        .join('\n');
    const workspaceReferences = (text: string, self?: string) => [
      ...(self ? referenced(text, 'crate').map((name) => [self, name] as [string, string]) : []),
      ...crates.flatMap((crate) =>
        referenced(text, identifier(crate)).map((name) => [crate, name] as [string, string])
      ),
    ];
    const officeCrates = readdirSync(path.join(repository, 'extensions/tmt-office/rust'));
    // Office declares workspace crates as `name.workspace = true` (or inline).
    const workspaceCrates = readdirSync(path.join(repository, 'rust/crates'));
    const workspaceDependencies = officeCrates.flatMap((crate) =>
      [
        ...read(`extensions/tmt-office/rust/${crate}/Cargo.toml`).matchAll(
          /^([\w-]+)(?:\.workspace\s*=\s*true|\s*=\s*\{[^}]*\bworkspace\s*=\s*true)/gm
        ),
      ]
        .map((match) => match[1])
        .filter((name) => workspaceCrates.includes(name))
    );
    expect(workspaceDependencies).toContain('tmt-command-output');
    expect(workspaceDependencies).not.toContain('tmt-cli');
    for (const dependency of workspaceDependencies) expect(crates).toContain(dependency);
    const office = rustFiles('extensions/tmt-office/rust').map(read).join('\n');
    // Crate roots re-export items, and Office reaches the API module at
    // runtime through `tmt api`.
    const pending: [string, string][] = [
      ...crates.map((crate) => [crate, 'lib'] as [string, string]),
      ['tmt-adapters', 'api'],
      ...workspaceReferences(office),
    ];
    const reached = new Set<string>();
    for (let next = pending.pop(); next; next = pending.pop()) {
      const [crate, name] = next;
      if (reached.has(`${crate}/${name}`)) continue;
      const text = sources(crate, name);
      if (!text) continue;
      reached.add(`${crate}/${name}`);
      pending.push(...workspaceReferences(text, crate));
    }
    expect(reached).toContain('tmt-adapters/storage');
    expect(reached).toContain('tmt-core/room');
    for (const [crate, names] of Object.entries(NATIVE_OFFICE_UNREACHABLE)) {
      expect(crates).toContain(crate);
      for (const name of names) {
        expect(sources(crate, name), `${crate}/${name} exists`).not.toBe('');
        expect(reached.has(`${crate}/${name}`), `${crate}/${name} is reachable`).toBe(false);
      }
    }
  });

  it('does not confuse similar prefixes and fails closed on an empty diff', () => {
    expect(selectCiAreas(['extensions/tmt-office/typescript/apps/office-other/file.ts'])).toEqual({
      native: true,
      office: true,
      nativeOffice: true,
    });
    expect(selectCiAreas([])).toEqual({ native: true, office: true, nativeOffice: true });
  });

  it('unions mixed paths including both sides of a no-renames diff', () => {
    expect(
      selectCiAreas(['extensions/tmt-office/typescript/apps/office/removed.ts', 'rust/new.rs'])
    ).toEqual({
      native: true,
      office: true,
      nativeOffice: true,
    });
    expect(selectCiAreas(['extensions/tmt-office/typescript/apps/office/deleted.ts'])).toEqual({
      native: false,
      office: true,
      nativeOffice: true,
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
      expect(readChangedCiAreas(base, historical, root)).toEqual({
        native: false,
        office: true,
        nativeOffice: true,
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
        office: true,
        nativeOffice: true,
      });
      mkdirSync(path.join(root, 'rust'));
      const target = path.join(root, 'rust/fixture.rs');
      renameSync(source, target);
      const moved = commit();
      expect(readChangedCiAreas(added, moved, root)).toEqual({
        native: true,
        office: true,
        nativeOffice: true,
      });
      rmSync(target);
      const deleted = commit();
      // A workspace file outside the crates is a native Office build input.
      expect(readChangedCiAreas(moved, deleted, root)).toEqual({
        native: true,
        office: false,
        nativeOffice: true,
      });
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
