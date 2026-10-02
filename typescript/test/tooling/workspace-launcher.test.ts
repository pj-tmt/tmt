import { mkdirSync, readFileSync, statSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';
import { runCli, withSandbox } from '../support/cli-process.js';

const launcher = fileURLToPath(new URL('../../../scripts/tmt-dev.sh', import.meta.url));

it('preserves arguments, caller directory and exit status in a checkout with spaces', async () => {
  await withSandbox(async (sandbox) => {
    const root = path.join(sandbox.root, 'workspace with spaces');
    mkdirSync(path.join(root, 'scripts'), { recursive: true });
    mkdirSync(path.join(root, 'rust/target/debug'), { recursive: true });
    const script = path.join(root, 'scripts/tmt-dev.sh');
    writeExecutable(script, readFileSync(launcher), statSync(launcher).mode & 0o777);
    writeExecutable(
      path.join(root, 'rust/target/debug/tmt'),
      '#!/bin/sh\npwd -P\nprintf "<%s>\\n" "$@"\nexit 7\n',
      0o755
    );
    const result = await runCli({ ...sandbox, cli: { executable: '/bin/sh', args: [script] } }, [
      'office',
      'two words',
      '',
      '--help',
    ]);
    expect(result.status).toBe(7);
    expect(result.stdout).toContain('\n<office>\n<two words>\n<>\n<--help>\n');
    const cwd = await runCli({ ...sandbox, cli: { executable: '/bin/pwd', args: ['-P'] } }, []);
    expect(result.stdout.split('\n')[0]).toBe(cwd.stdout.trim());
    expect(result.stderr).toBe('');
  });
});

it('fails with build guidance instead of invoking a PATH binary when the checkout is unbuilt', async () => {
  await withSandbox(async (sandbox) => {
    mkdirSync(path.join(sandbox.root, 'scripts'));
    const script = path.join(sandbox.root, 'scripts/tmt-dev.sh');
    writeExecutable(script, readFileSync(launcher), statSync(launcher).mode & 0o777);
    const result = await runCli({ ...sandbox, cli: { executable: '/bin/sh', args: [script] } }, [
      '--version',
    ]);
    expect(result.status).toBe(127);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('cargo build --locked -p tmt-cli');
    expect(result.stderr).toContain('globally installed tmt will not be used');
  });
});
