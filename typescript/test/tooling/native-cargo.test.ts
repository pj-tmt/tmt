import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

const wrapper = fileURLToPath(new URL('../../../scripts/native-cargo.sh', import.meta.url));

interface FakeCargo {
  readonly argsFile: string;
  readonly executable: string;
}

function runWrapper(sandbox: Sandbox, args: string[]) {
  return runCli({ ...sandbox, cli: { executable: wrapper, args: [] } }, args);
}

function createFakeCargo(sandbox: Sandbox, exitCode = 0): FakeCargo {
  const recorder = path.join(sandbox.root, 'fake cargo recorder "quoted".mjs');
  const executable = path.join(sandbox.root, 'fake cargo "quoted"');
  const argsFile = path.join(sandbox.root, 'fake cargo args.json');
  fs.writeFileSync(
    recorder,
    `import { writeFileSync } from 'node:fs';
writeFileSync(process.env.TMT_NATIVE_ARGS_FILE, JSON.stringify(process.argv.slice(2)));
process.exit(Number(process.env.TMT_TEST_STATUS));
`
  );
  fs.writeFileSync(executable, '#!/bin/sh\nexec "$TMT_TEST_NODE" "$TMT_TEST_RECORDER" "$@"\n', {
    mode: 0o755,
  });
  sandbox.env.TMT_TEST_NODE = process.execPath;
  sandbox.env.TMT_TEST_RECORDER = recorder;
  sandbox.env.TMT_NATIVE_ARGS_FILE = argsFile;
  sandbox.env.TMT_TEST_STATUS = String(exitCode);
  return { argsFile, executable };
}

function recordedArgs(argsFile: string): string[] {
  return JSON.parse(fs.readFileSync(argsFile, 'utf8')) as string[];
}

describe('native cargo wrapper', () => {
  it.each(['build', 'metadata'])(
    'is executable and forwards %s arguments before adding locked',
    async (command) => {
      expect(fs.existsSync(wrapper)).toBe(true);
      expect(fs.statSync(wrapper).mode & 0o111).not.toBe(0);

      await withSandbox(async (sandbox) => {
        const fake = createFakeCargo(sandbox);
        const args = [command, '--target', 'target with spaces', '--features', 'quote"value'];
        sandbox.env.TMT_NATIVE_REAL_CARGO = fake.executable;

        const result = await runWrapper(sandbox, args);

        expect(result.status).toBe(0);
        expect(result.signal).toBeNull();
        expect(result.stderr).toBe('');
        expect(recordedArgs(fake.argsFile)).toEqual([...args, '--locked']);
      });
    }
  );

  it.each([
    ['-vV'],
    ['--version'],
    ['check', '--locked'],
    ['metadata', '--locked'],
    ['build', '--frozen'],
  ])('leaves non-build cargo invocation %j unchanged', async (...args: string[]) => {
    await withSandbox(async (sandbox) => {
      const fake = createFakeCargo(sandbox);
      sandbox.env.TMT_NATIVE_REAL_CARGO = fake.executable;

      const result = await runWrapper(sandbox, args);

      expect(result.status).toBe(0);
      expect(result.signal).toBeNull();
      expect(result.stderr).toBe('');
      expect(recordedArgs(fake.argsFile)).toEqual(args);
    });
  });

  it('propagates the real cargo exit status without changing its arguments', async () => {
    await withSandbox(async (sandbox) => {
      const fake = createFakeCargo(sandbox, 17);
      const args = ['-vV'];
      sandbox.env.TMT_NATIVE_REAL_CARGO = fake.executable;

      const result = await runWrapper(sandbox, args);

      expect(result.status).toBe(17);
      expect(result.signal).toBeNull();
      expect(result.stderr).toBe('');
      expect(recordedArgs(fake.argsFile)).toEqual(args);
    });
  });

  it.each([
    ['missing', undefined],
    ['relative', 'cargo'],
  ])('rejects %s real cargo configuration without fallback', async (_label, realCargo) => {
    await withSandbox(async (sandbox) => {
      delete sandbox.env.TMT_NATIVE_REAL_CARGO;
      if (realCargo !== undefined) sandbox.env.TMT_NATIVE_REAL_CARGO = realCargo;

      const result = await runWrapper(sandbox, []);

      expect(result.status).toBe(2);
      expect(result.signal).toBeNull();
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain('TMT_NATIVE_REAL_CARGO must be an absolute executable path.');
    });
  });

  it('rejects an absolute non-executable cargo target', async () => {
    await withSandbox(async (sandbox) => {
      const target = path.join(sandbox.root, 'cargo target with spaces');
      fs.writeFileSync(target, 'not executable\n', { mode: 0o644 });
      sandbox.env.TMT_NATIVE_REAL_CARGO = target;

      const result = await runWrapper(sandbox, []);

      expect(result.status).toBe(2);
      expect(result.signal).toBeNull();
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain(`TMT_NATIVE_REAL_CARGO is not executable: ${target}`);
    });
  });
});
