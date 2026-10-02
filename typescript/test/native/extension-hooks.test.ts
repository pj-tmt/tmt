import {
  chmodSync,
  existsSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { cli } from '../support/extension-hooks.js';

// A non-Office extension: it answers the hook protocol and records what it sees.
const FIXTURE = `#!/bin/sh
if [ "$1 $2 $3" = "__tmt-hooks 1 capabilities" ]; then
  printf '%s' "$FIXTURE_CAPABILITIES"
  exit 0
fi
if [ "$1 $2 $3" = "__tmt-hooks 1 observe" ]; then
  cat >> "$FIXTURE_LOG"
  printf '\\n' >> "$FIXTURE_LOG"
  printf '%s\\n' "$TMT_HOOK_DELIVERY" >> "$FIXTURE_LOG.marker"
  case "$FIXTURE_MODE" in
    fail) exit 1 ;;
    slow) exec sleep 7.25 ;;
    loud) head -c 200000 /dev/zero | tr '\\0' x ;;
    nested) "$TMT_EXECUTABLE" room create "Nested room" --json > /dev/null ;;
  esac
  exit 0
fi
exit 2
`;

interface Fixture {
  bin: string;
  executable: string;
  log: string;
}

function install(sandbox: Sandbox): Fixture {
  const bin = path.join(sandbox.root, 'hooks-bin');
  mkdirSync(bin, { mode: 0o755 });
  chmodSync(bin, 0o755);
  const executable = path.join(bin, 'tmt-fixture');
  writeExecutable(executable, FIXTURE, 0o755);
  const log = path.join(sandbox.root, 'observed.log');
  sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
  sandbox.env.FIXTURE_LOG = log;
  sandbox.env.FIXTURE_CAPABILITIES = 'TMT-HOOKS/1\nlifecycle_observations_v1\n';
  sandbox.env.FIXTURE_MODE = '';
  return { bin, executable, log };
}

function observed(fixture: Fixture): unknown[] {
  if (!existsSync(fixture.log)) return [];
  return readFileSync(fixture.log, 'utf8')
    .split('\n')
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line));
}

async function room(sandbox: Sandbox, name: string) {
  return ((await cli(sandbox, ['room', 'create', name])).room as { id: string }).id;
}

describe('consented extension hooks', () => {
  it('never runs an extension that was only found on PATH, and creates no settings', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = install(sandbox);
      await room(sandbox, 'Quiet');
      expect(observed(fixture)).toEqual([]);
      expect(existsSync(path.join(sandbox.globalDir, 'extension-hooks.json'))).toBe(false);
      expect(await cli(sandbox, ['extension', 'hooks', 'list'])).toEqual({ extensions: [] });
    });
  });

  it('refuses unsafe or incompatible executables at enable time', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = install(sandbox);
      const refuse = async (code: string) => {
        const result = await runCli(sandbox, ['extension', 'hooks', 'enable', 'fixture', '--json']);
        expect(result.status).toBe(1);
        expect(parseWholeStdout(result).error).toMatchObject({ code });
      };
      chmodSync(fixture.executable, 0o775);
      await refuse('EXTENSION_UNSAFE');
      chmodSync(fixture.executable, 0o755);
      chmodSync(fixture.bin, 0o777);
      await refuse('EXTENSION_UNSAFE');
      chmodSync(fixture.bin, 0o755);
      sandbox.env.FIXTURE_CAPABILITIES = 'TMT-HOOKS/2\nlifecycle_observations_v1\n';
      await refuse('EXTENSION_INCOMPATIBLE');
      sandbox.env.FIXTURE_CAPABILITIES = 'TMT-HOOKS/1\nsomething_else_v1\n';
      await refuse('EXTENSION_INCOMPATIBLE');
      const missing = await runCli(sandbox, ['extension', 'hooks', 'enable', 'absent', '--json']);
      expect(parseWholeStdout(missing).error).toMatchObject({ code: 'EXTENSION_NOT_FOUND' });
      expect(existsSync(path.join(sandbox.globalDir, 'extension-hooks.json'))).toBe(false);
    });
  });

  it('delivers typed evidence after commit, without veto, under bounds and without recursion', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = install(sandbox);
      const enabled = await cli(sandbox, ['extension', 'hooks', 'enable', 'fixture']);
      expect(enabled.enabled).toMatchObject({
        name: 'fixture',
        path: realpathSync(fixture.executable),
        capabilities: ['lifecycle_observations_v1'],
      });
      const settings = path.join(sandbox.globalDir, 'extension-hooks.json');
      expect(statSync(settings).mode & 0o777).toBe(0o600);

      const created = await room(sandbox, 'Private design');
      const identity = (
        (await cli(sandbox, ['identity', 'create', 'Secret Agent'])).identity as { id: string }
      ).id;
      await cli(sandbox, ['rm', 'Secret Agent', '--force']);
      expect(observed(fixture)).toEqual([
        {
          version: 1,
          events: [{ kind: 'room.created', roomId: created, revision: 1, retired: false }],
        },
        {
          version: 1,
          events: [
            { kind: 'identity.created', identityId: identity, lifetime: 'saved', retired: false },
          ],
        },
        {
          version: 1,
          events: [
            { kind: 'identity.retired', identityId: identity, lifetime: 'saved', retired: true },
          ],
        },
      ]);
      expect(readFileSync(fixture.log, 'utf8')).not.toMatch(/Private|Secret/);
      expect(readFileSync(`${fixture.log}.marker`, 'utf8').trim().split('\n')).toEqual([
        '1',
        '1',
        '1',
      ]);

      // No veto: a failing observer never changes the command's result.
      sandbox.env.FIXTURE_MODE = 'fail';
      await room(sandbox, 'Still created');

      // One aggregate deadline: a hanging observer is killed and cleaned up.
      sandbox.env.FIXTURE_MODE = 'slow';
      const started = Date.now();
      await room(sandbox, 'Not delayed');
      expect(Date.now() - started).toBeLessThan(4_000);
      expect(execFileSync('ps', ['-Ao', 'args']).toString()).not.toContain('sleep 7.25');

      // Output beyond the bound is cut off without blocking the command.
      sandbox.env.FIXTURE_MODE = 'loud';
      await room(sandbox, 'Loud');

      // Recursion guard: tmt called from inside an observer emits nothing.
      sandbox.env.FIXTURE_MODE = 'nested';
      const before = observed(fixture).length;
      await room(sandbox, 'Outer');
      expect(observed(fixture).length).toBe(before + 1);
      const rooms = await cli(sandbox, ['room', 'list']);
      expect(JSON.stringify(rooms)).toContain('Nested room');
    });
  });

  it('stops trusting a changed, removed or disabled extension', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = install(sandbox);
      await cli(sandbox, ['extension', 'hooks', 'enable', 'fixture']);
      await room(sandbox, 'First');
      expect(observed(fixture).length).toBe(1);

      writeExecutable(fixture.executable, `${FIXTURE}# changed\n`, 0o755);
      await room(sandbox, 'After change');
      expect(observed(fixture).length).toBe(1);

      await cli(sandbox, ['extension', 'hooks', 'enable', 'fixture']);
      await room(sandbox, 'Re-enabled');
      expect(observed(fixture).length).toBe(2);

      rmSync(fixture.executable);
      await room(sandbox, 'Uninstalled');
      expect(observed(fixture).length).toBe(2);

      writeExecutable(fixture.executable, FIXTURE, 0o755);
      await cli(sandbox, ['extension', 'hooks', 'enable', 'fixture']);
      expect(await cli(sandbox, ['extension', 'hooks', 'disable', 'fixture'])).toEqual({
        name: 'fixture',
        changed: true,
      });
      await room(sandbox, 'Disabled');
      expect(observed(fixture).length).toBe(2);
      expect(await cli(sandbox, ['extension', 'hooks', 'list'])).toEqual({ extensions: [] });
    });
  });
});
