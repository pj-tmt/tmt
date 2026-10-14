import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it, vi } from 'vite-plus/test';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';

vi.mock('../../scripts/colab-runtime-proof.mjs', () => ({
  verifyColabApp: vi.fn(),
  verifyColabHosting: vi.fn(),
}));
import { runCli, withSandbox } from '../support/cli-process.js';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const rawVerifier = path.join(repositoryRoot, 'scripts/verify-native-runtime.mjs');
const { assertMacOsArchitecture, nativeHostTarget, verifyNativeRuntime } = (await import(
  pathToFileURL(path.join(repositoryRoot, 'scripts/native-runtime-proof.mjs')).href
)) as unknown as {
  nativeHostTarget: () => string;
  assertMacOsArchitecture: (
    executable: string,
    target: string,
    options: { cwd: string; env: NodeJS.ProcessEnv },
    inspect?: (executable: string, args: string[], options: unknown) => string
  ) => void;
  verifyNativeRuntime: (options: {
    executable: string;
    product?: 'colab' | 'digest';
    digestSkill?: string;
    colabSkill?: string;
    colabReferences?: Record<string, string>;
    colabApp?: string;
    notices?: string;
    target: string;
    version: string;
    skill: string;
    inboxSkill: string;
    profileContent: string;
    subject: string;
  }) => Promise<void>;
};

describe('native runtime proof boundary', () => {
  it('proves relocated Digest read-only commands and rejects version, help, skill and state drift', async () => {
    await withSandbox(async (sandbox) => {
      const executable = path.join(sandbox.root, 'tmt-digest');
      writeExecutable(executable, '#!/bin/sh\nexit 99\n', 0o755);
      const skill = 'Exact bundled Digest instructions\n';
      const calls: string[][] = [];
      const roots = new Set<string>();
      let fault = '';
      const inspect = vi.spyOn(childProcess, 'spawnSync').mockImplementation(((
        command,
        args,
        options
      ) => {
        const words = args as string[];
        const { env, cwd } = options as { env: NodeJS.ProcessEnv; cwd: string };
        let stdout = '';
        if (command === executable) {
          calls.push(words);
          expect(env.TMT_AGENT).toBe('1');
          expect(env.BROWSER).toBe('/usr/bin/true');
          expect(env.TMUX).toBeUndefined();
          expect(env.TMT_HOME).toBeUndefined();
          expect(cwd).not.toContain(repositoryRoot);
          expect(path.dirname(env.HOME!)).toBe(path.dirname(cwd));
          expect(path.dirname(env.PATH!)).toBe(path.dirname(cwd));
          roots.add(path.dirname(cwd));
          if (words[0] === '--version')
            stdout = `digest ${fault === 'version' ? '0.0.0' : '0.1.0'}\n`;
          else if (words[0] === '--help')
            stdout = `Usage: tmt digest <MEMBER> <VALUE>\n  tick  Deliver\n${fault === 'help' ? '' : '  skill  Read instructions\n'}`;
          else if (words[0] === 'tick') {
            expect(words).toEqual(['tick', '--help']);
            stdout = fault === 'tick help' ? 'Wrong command\n' : 'Usage: tmt digest tick\n';
          } else {
            expect(words).toEqual(['skill']);
            stdout = fault === 'skill' ? skill + 'drift' : skill;
            if (fault === 'config') mkdirSync(env.XDG_CONFIG_HOME!);
            if (fault === 'home') writeFileSync(path.join(env.HOME!, 'unexpected'), 'state');
            if (fault === 'workspace') writeFileSync(path.join(cwd, 'unexpected'), 'state');
          }
        } else if (command === '/usr/bin/xcrun') stdout = `/mock/${words[1]}\n`;
        else if (command === '/mock/lipo')
          stdout = process.arch === 'arm64' ? 'arm64\n' : 'x86_64\n';
        else if (command === '/mock/otool')
          stdout = `${executable}:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n`;
        else expect(String(command)).toMatch(/readelf$/);
        return { pid: 0, status: 0, signal: null, stdout, stderr: '', output: [null, stdout, ''] };
      }) as typeof childProcess.spawnSync);
      syncBuiltinESMExports();
      const prove = (digestSkill: string | undefined) =>
        verifyNativeRuntime({
          executable,
          target: nativeHostTarget(),
          version: '0.1.0',
          product: 'digest',
          digestSkill,
          skill: '',
          inboxSkill: '',
          profileContent: '',
          subject: 'Digest archive fixture',
        });
      try {
        await prove(skill);
        expect(calls).toEqual([['--version'], ['--help'], ['tick', '--help'], ['skill']]);
        for (const [failure, message] of [
          ['version', 'version mismatch'],
          ['help', 'missing skill'],
          ['tick help', 'tick help mismatch'],
          ['skill', 'embedded Digest skill mismatch'],
          ['config', 'must not initialize config state'],
          ['home', 'must not create home state'],
          ['workspace', 'must not create workspace state'],
        ]) {
          fault = failure;
          await expect(prove(skill)).rejects.toThrow(message);
        }
        await expect(prove(undefined)).rejects.toThrow('canonical skill');
      } finally {
        inspect.mockRestore();
        syncBuiltinESMExports();
        for (const root of roots) expect(existsSync(root)).toBe(false);
      }
    });
  });

  it('rejects arm64 and universal bytes in an Intel proof even when the process target matches', () => {
    const options = { cwd: '/', env: {} };
    const inspected: string[][] = [];
    const inspect = (architecture: string) => (command: string, args: string[]) => {
      inspected.push([command, ...args]);
      if (command === '/usr/bin/xcrun') return '/selected/lipo\n';
      return `${architecture}\n`;
    };
    expect(() =>
      assertMacOsArchitecture('/candidate/tmt', 'x86_64-apple-darwin', options, inspect('x86_64'))
    ).not.toThrow();
    for (const architecture of ['arm64', 'x86_64 arm64', '']) {
      expect(() =>
        assertMacOsArchitecture(
          '/candidate/tmt',
          'x86_64-apple-darwin',
          options,
          inspect(architecture)
        )
      ).toThrow('Executable must contain exactly x86_64');
    }
    expect(inspected).toEqual(
      Array(4)
        .fill([
          ['/usr/bin/xcrun', '--find', 'lipo'],
          ['/selected/lipo', '-archs', '/candidate/tmt'],
        ])
        .flat()
    );
    expect(() =>
      assertMacOsArchitecture('/candidate/tmt', 'aarch64-apple-darwin', options, inspect('arm64'))
    ).not.toThrow();
    expect(() =>
      assertMacOsArchitecture('/candidate/tmt', 'aarch64-apple-darwin', options, inspect('x86_64'))
    ).toThrow('Executable must contain exactly arm64');
    expect(() =>
      assertMacOsArchitecture('/candidate/tmt', 'x86_64-apple-darwin', options, () => {
        throw new Error('lipo failed');
      })
    ).toThrow('lipo failed');
  });

  it('loads the CLI verifier from exactly the minimal image inputs without Colab tooling', async () => {
    await withSandbox(async (sandbox) => {
      const root = path.join(repositoryRoot, '..');
      const dockerfile = readFileSync(
        path.join(repositoryRoot, 'test/native/runtime.Dockerfile'),
        'utf8'
      );
      for (const line of dockerfile.split('\n').filter((line) => line.startsWith('COPY '))) {
        const entries = line.split(/\s+/).slice(1);
        const destination = entries.pop()!;
        for (const source of entries) {
          const target = path.join(
            sandbox.root,
            destination,
            ...(destination.endsWith('/') ? [path.basename(source)] : [])
          );
          mkdirSync(path.dirname(target), { recursive: true });
          copyFileSync(path.join(root, source), target);
        }
      }
      const verifier = path.join(sandbox.root, 'typescript/scripts/verify-native-runtime.mjs');
      const invoke = () =>
        runCli({ ...sandbox, cli: { executable: process.execPath, args: [verifier] } }, []);
      const valid = await invoke();
      expect(valid.status).toBe(1);
      expect(valid.stderr).toContain('--executable is required');
      expect(valid.stderr).not.toContain('ERR_MODULE_NOT_FOUND');
      // The database observer is part of the same minimal runtime import closure.
      const observer = path.join(sandbox.root, 'typescript/scripts/native-database-layout.mjs');
      const observerBytes = readFileSync(observer);
      unlinkSync(observer);
      const missingObserver = await invoke();
      expect(missingObserver.status).toBe(1);
      expect(missingObserver.stderr).toContain('ERR_MODULE_NOT_FOUND');
      expect(missingObserver.stderr).toContain('native-database-layout.mjs');
      writeFileSync(observer, observerBytes);
      // The exact prior eager-import defect must fail in this same valid image closure.
      const proof = path.join(sandbox.root, 'typescript/scripts/native-runtime-proof.mjs');
      writeFileSync(proof, `${readFileSync(proof, 'utf8')}\nimport './colab-runtime-proof.mjs';\n`);
      const broken = await invoke();
      expect(broken.status).toBe(1);
      expect(broken.stderr).toContain('ERR_MODULE_NOT_FOUND');
      expect(broken.stderr).not.toContain('--executable is required');
    });
  });

  it('compares archive reference bytes with embedded output and rejects drift, missing and unknown references', async () => {
    await withSandbox(async (sandbox) => {
      const executable = path.join(sandbox.root, 'colab-archive');
      writeExecutable(executable, '#!/bin/sh\nexit 99\n', 0o755);
      const skill = 'Read `tmt colab skill page-style`.\n';
      const embedded = 'Exact shipped reference bytes\n';
      const calls: string[][] = [];
      const inspect = vi.spyOn(childProcess, 'spawnSync').mockImplementation(((command, args) => {
        const words = args as string[];
        let stdout = '';
        if (command === executable) {
          calls.push(words);
          stdout = words.length === 1 ? skill : embedded;
        } else if (command === '/usr/bin/xcrun') {
          stdout = `/mock/${words[1]}\n`;
        } else if (command === '/mock/lipo') {
          stdout = process.arch === 'arm64' ? 'arm64\n' : 'x86_64\n';
        } else if (command === '/mock/otool') {
          stdout = `${executable}:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n`;
        } else {
          expect(String(command)).toMatch(/readelf$/);
        }
        return { pid: 0, status: 0, signal: null, stdout, stderr: '', output: [null, stdout, ''] };
      }) as typeof childProcess.spawnSync);
      syncBuiltinESMExports();
      const prove = (references: Record<string, string>) =>
        verifyNativeRuntime({
          executable,
          target: nativeHostTarget(),
          version: '0.1.0',
          product: 'colab',
          colabSkill: skill,
          colabReferences: references,
          colabApp: sandbox.root,
          notices: '',
          skill: '',
          inboxSkill: '',
          profileContent: '',
          subject: 'Colab archive fixture',
        });
      try {
        await prove({ 'page-style': embedded });
        expect(calls).toEqual([['skill'], ['skill', 'page-style']]);
        await expect(prove({ 'page-style': embedded + 'drift' })).rejects.toThrow(
          'embedded skill reference mismatch: page-style'
        );
        await expect(prove({})).rejects.toThrow('missing skill reference: page-style');
        await expect(prove({ 'page-style': embedded, unknown: embedded })).rejects.toThrow(
          'unknown skill reference: unknown'
        );
      } finally {
        inspect.mockRestore();
        syncBuiltinESMExports();
      }
    });
  });

  it('requires explicit raw runtime inputs', async () => {
    await withSandbox(async (sandbox) => {
      const result = await runCli(
        {
          ...sandbox,
          cli: { executable: process.execPath, args: [rawVerifier] },
        },
        []
      );
      expect(result.status).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain('--executable is required');
    });
  });

  it('rejects a mismatched target before executing an arbitrary file', async () => {
    await withSandbox(async (sandbox) => {
      const executable = path.join(sandbox.root, 'not-native');
      const marker = path.join(sandbox.root, 'executed');
      writeExecutable(executable, `#!/bin/sh\n: > ${JSON.stringify(marker)}\n`, 0o755);
      const calibrated = await runCli(
        {
          ...sandbox,
          cli: { executable: '/bin/sh', args: [executable] },
        },
        []
      );
      expect(calibrated.status).toBe(0);
      expect(existsSync(marker)).toBe(true);
      unlinkSync(marker);
      const skill = path.join(sandbox.root, 'SKILL.md');
      writeFileSync(skill, 'skill\n');
      const result = await runCli(
        {
          ...sandbox,
          cli: { executable: process.execPath, args: [rawVerifier] },
        },
        [
          '--executable',
          executable,
          '--target',
          'unsupported-target',
          '--version',
          '5.0.0-test',
          '--skill',
          skill,
        ]
      );
      expect(result.status).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain('requires a matching native host');
      expect(existsSync(marker)).toBe(false);
    });
  });

  it('shared proof rejects a script instead of treating it as native support', async () => {
    await withSandbox(async (sandbox) => {
      const executable = path.join(sandbox.root, 'not-native');
      const marker = path.join(sandbox.root, 'executed');
      writeExecutable(executable, `#!/bin/sh\n: > ${JSON.stringify(marker)}\n`, 0o755);
      await expect(
        verifyNativeRuntime({
          executable,
          target: nativeHostTarget(),
          version: '5.0.0-test',
          skill: 'skill\n',
          inboxSkill: 'inbox skill\n',
          profileContent: 'proof',
          subject: 'Native executable',
        })
      ).rejects.toThrow(
        process.platform === 'darwin' ? 'lipo' : 'Native executable linkage inspection failed'
      );
      expect(existsSync(marker)).toBe(false);
    });
  });
});
