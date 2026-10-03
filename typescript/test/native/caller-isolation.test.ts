import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { expectError, runCli, withSandbox } from '../support/cli-process.js';

it('isolates inherited shared-runtime ancestry while a direct CLI remains fenced', async () => {
  await withSandbox(async (sandbox) => {
    // Reuse Docker's existing real process-shape fixture: a binary named codex
    // with app-server in its argv, without a provider, model or host tmux server.
    const codex = path.join(sandbox.root, 'codex');
    const fixture = fileURLToPath(
      new URL('../../../rust/target/debug/examples/runtime-caller-fixture', import.meta.url)
    );
    writeExecutable(codex, readFileSync(fixture));
    const parent = path.join(sandbox.root, 'shared-runtime.mjs');
    const launcher = fileURLToPath(new URL('../support/neutral-parent.mjs', import.meta.url));
    writeExecutable(
      parent,
      `import { spawn } from 'node:child_process';
const [, mode, executable, ...args] = process.argv.slice(2);
const child = mode === 'neutral'
  ? spawn(${JSON.stringify(process.execPath)}, [${JSON.stringify(launcher)}, 'relay', executable, ...args], { stdio: ['inherit', 'inherit', 'inherit', 'pipe'] })
  : spawn(executable, args, { stdio: 'inherit' });
child.stdio[3]?.resume();
child.once('error', (error) => { console.error(error); process.exit(1); });
child.once('exit', (status, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(status);
});
`,
      0o644
    );
    // Keep the same session marker on both paths: clearing it cannot mask the
    // observed shared-host boundary, and does not explain the neutral result.
    sandbox.env.CODEX_THREAD_ID = '11111111-1111-4111-8111-111111111111';
    const invoke = (mode: string) =>
      runCli(
        {
          ...sandbox,
          cli: {
            executable: codex,
            args: [
              'app-server',
              process.execPath,
              parent,
              'app-server',
              mode,
              sandbox.cli.executable,
              ...sandbox.cli.args,
            ],
          },
        },
        ['identity', 'meta', 'list', '--json']
      );
    const direct = await invoke('direct');
    expect(direct.status).toBe(1);
    expectError(direct, 'CALLER_IDENTITY_AMBIGUOUS');
    expect(existsSync(sandbox.database)).toBe(false);
    const isolated = await invoke('neutral');
    expect(isolated.status).toBe(1);
    expectError(isolated, 'IDENTITY_REQUIRED');
    expect(existsSync(sandbox.database)).toBe(false);
    // Repeat the positive control after isolation to prove no process-global
    // guard override or environment mutation was introduced.
    expectError(await invoke('direct'), 'CALLER_IDENTITY_AMBIGUOUS');
  });
});
