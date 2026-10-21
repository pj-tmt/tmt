// Invariant: locale variables are never changed by production tmux execution.
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { installTmuxTrace } from './tmux-trace.js';
import { spawnRealTmuxCli, releaseRealTmuxCli, readRealTmuxCli } from './real-tmux-caller.js';
import { expectError, runCli, withSandbox } from '../support/cli-process.js';
import { writeExecutable } from '../support/executable-fixture.mjs';

const inputLog = { mode: 'input-log' } as const;

interface Capture {
  target: string;
  pane: string;
  identity?: { name: string; canonicalName: string };
  lines: number;
  output: string;
}

async function seedDiagnostic(fixture: E2EFixture, message: string): Promise<void> {
  fixture.tmux(['send-keys', '-l', '-t', fixture.pane, '--', message]);
  fixture.tmux(['send-keys', '-t', fixture.pane, 'Enter']);
  await fixture.waitForEvent(
    (event) => event.event === 'input' && event.pid === fixture.panePid && event.line === message
  );
  // Capture is intentionally terminal diagnostics, not a final reply assertion.
  await fixture.waitForCapture((output) => output.includes(message));
}

describe('recorded-recipient diagnostic routing', { concurrent: false }, () => {
  it('captures names, stable panes and tmux targets with configured and explicit line counts', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', 'Alice'])).toMatchObject({
        code: 0,
        json: { bound: true },
      });
      const marker = 'native-check-causal-marker';
      await seedDiagnostic(fixture, marker);
      fs.writeFileSync(
        path.join(fixture.globalDir, 'config.json'),
        JSON.stringify({ defaults: { captureLines: 0 } })
      );
      const metadata = fixture.paneMetadata();
      for (const target of ['aLiCe', fixture.pane, fixture.paneTarget(fixture.pane)]) {
        const captured = await fixture.runJsonCli<Capture>(['check', target]);
        expect(captured).toMatchObject({
          code: 0,
          stderr: '',
          json: {
            target,
            pane: fixture.pane,
            identity: { name: 'Alice', canonicalName: 'alice' },
            lines: 0,
            output: expect.stringContaining(marker),
          },
        });
        expect(Object.keys(captured.json ?? {}).sort()).toEqual([
          'identity',
          'lines',
          'output',
          'pane',
          'target',
        ]);
      }
      const maximum = await fixture.runJsonCli<Capture>(['read', 'Alice', '--lines=2147483647']);
      expect(maximum).toMatchObject({
        code: 0,
        json: { lines: 2147483647, output: expect.stringContaining(marker) },
      });
      expect(fixture.paneMetadata()).toBe(metadata);
      const human = await fixture.runCli(['check', 'Alice', '--lines', '0']);
      expect(human.code).toBe(0);
      expect(human.stderr).toBe('');
      expect(human.stdout).toContain(`OUTPUT Alice ${fixture.pane}\n`);
      expect(human.stdout).toContain(marker);
      expect(await fixture.runJsonCli(['unbind'])).toMatchObject({ code: 0 });
      const unbound = await fixture.runJsonCli<Capture>(['read', fixture.pane, '0']);
      expect(unbound).toMatchObject({
        code: 0,
        json: { pane: fixture.pane, output: expect.stringContaining(marker) },
      });
      expect(unbound.json).not.toHaveProperty('identity');
      const missing = await fixture.runJsonCli(['check', '%999999']);
      expect(missing).toMatchObject({ code: 3, json: { error: { code: 'PANE_NOT_FOUND' } } });
    }, inputLog);
  });

  it('reports typed storage denial after bound-caller preflight without launching or rebinding', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', '-s', 'Existing'])).toMatchObject({ code: 0 });
      const before = durableState(fixture);
      const metadata = fixture.paneMetadata();
      const marker = path.join(fixture.workspace, 'must-not-launch');
      fs.chmodSync(fixture.globalDir, 0o500);
      try {
        for (const args of [['whoami'], ['name', 'NewIdentity']]) {
          expect(await fixture.runJsonCli(args)).toMatchObject({
            code: 1,
            stderr: '',
            json: { error: { code: 'STORAGE_NOT_WRITABLE' } },
          });
        }
        const launch = await fixture.runCli([
          'run',
          'NewIdentity',
          '/bin/sh',
          '-c',
          `touch '${marker}'`,
        ]);
        expect(launch.code).toBe(1);
        expect(launch.stdout).toBe('');
        expect(launch.stderr).toContain(fixture.globalDir);
        expect(launch.stderr).toContain('No command was launched');
        expect(fs.existsSync(marker)).toBe(false);
        expect(fixture.paneMetadata()).toBe(metadata);
        expect(durableState(fixture)).toEqual(before);
      } finally {
        fs.chmodSync(fixture.globalDir, 0o700);
      }
      expect(await fixture.runJsonCli(['whoami'])).toMatchObject({ code: 0 });
    }, inputLog);
  });

  it.each(['C', 'fr_FR.UTF-8'])(
    'keeps UTF-8 capture byte-identical and classifies real socket denial under locale %s',
    async (locale) => {
      const fixtures: E2EFixture[] = [];
      await withE2EFixture(async (fixture) => {
        fixtures.push(fixture);
        const marker = 'UTF-8: 日本語 café 🦀';
        await seedDiagnostic(fixture, marker);
        const baseline = await fixture.runJsonCli<Capture>(['check', fixture.pane, '0']);
        expect(baseline.code).toBe(0);
        expect(baseline.json?.output).toContain(marker);
        const before = durableState(fixture);
        await withSandbox(async (sandbox) => {
          // Pin the owned socket across the root/nobody UID change; -L selects
          // a different tmux UID directory for each effective user.
          const wrapperDir = path.join(sandbox.cwd, 'bin');
          fs.mkdirSync(wrapperDir);
          writeExecutable(
            path.join(wrapperDir, 'tmux'),
            '#!/bin/sh\nexec "$TMT_TEST_REAL_TMUX" -S "$TMT_TEST_SOCKET" "$@"\n'
          );
          const env = {
            ...sandbox.env,
            PATH: `${wrapperDir}${path.delimiter}${sandbox.env.PATH}`,
            TMT_TEST_REAL_TMUX: fixture.tmuxPath,
            TMT_TEST_SOCKET: fixture.socketPath,
            TMUX: `${fixture.socketPath},${fixture.serverPid},0`,
            TMUX_PANE: fixture.pane,
            LANG: locale,
            LC_ALL: locale,
          };
          const selected = { ...sandbox, env };
          const outside = { ...selected, env: { ...env, TMUX: undefined, TMUX_PANE: undefined } };
          const captured = await runCli(outside, ['check', fixture.pane, '0', '--json']);
          expect(captured.status).toBe(0);
          expect(captured.stderr).toBe('');
          expect(Buffer.from(JSON.parse(captured.stdout).output, 'utf8')).toEqual(
            Buffer.from(baseline.json!.output, 'utf8')
          );
          const missing = await runCli(outside, ['check', '%999999', '--json']);
          expect(missing.status).toBe(3);
          expectError(missing, 'PANE_NOT_FOUND');
          // Root can bypass socket mode bits. In the isolated Docker fixture only,
          // run the refusal as nobody so OS permission evidence is exercised.
          const unprivileged = process.getuid!() === 0;
          for (const directory of [
            sandbox.root,
            sandbox.globalDir,
            fixture.root,
            fixture.socketRoot,
          ])
            fs.chmodSync(directory, 0o755);
          const socketMode = fs.statSync(fixture.socketPath).mode & 0o777;
          fs.chmodSync(fixture.socketPath, 0o000);
          try {
            const denied = await runCli(
              {
                ...selected,
                cli: unprivileged
                  ? {
                      executable: '/usr/bin/setpriv',
                      args: [
                        '--reuid=65534',
                        '--regid=65534',
                        '--clear-groups',
                        sandbox.cli.executable,
                        ...sandbox.cli.args,
                      ],
                    }
                  : sandbox.cli,
              },
              ['name', 'DeniedSocket', '--json']
            );
            expect(denied.status, JSON.stringify(denied)).toBe(1);
            expect(denied.stderr).toBe('');
            expectError(denied, 'TMUX_PERMISSION_DENIED');
          } finally {
            fs.chmodSync(fixture.socketPath, socketMode);
          }
          expect(durableState(fixture)).toEqual(before);
          const recovered = await runCli(outside, ['check', fixture.pane, '0', '--json']);
          expect(recovered.status).toBe(0);
          expect(Buffer.from(JSON.parse(recovered.stdout).output, 'utf8')).toEqual(
            Buffer.from(baseline.json!.output, 'utf8')
          );
        });
      }, inputLog);
      for (const fixture of fixtures) {
        expect(fs.existsSync(fixture.socketRoot)).toBe(false);
        expect(fixture.serverProcessIsRunning()).toBe(false);
        expect(fixture.mockProcessIsRunning()).toBe(false);
      }
    }
  );

  it.each([false, true])(
    'routes the recorded socket rather than an equal local pane ID (copied UUID: %s)',
    async (copyUuid) => {
      await withE2EFixture(async (first) => {
        expect(await first.runJsonCli(['name', 'Foreign'])).toMatchObject({ code: 0 });
        await seedDiagnostic(first, 'recorded-socket-only');
        const serverId = JSON.parse(first.paneMetadata()).globalIdentity.serverId;
        await withE2EFixture(
          async (second) => {
            expect(second.pane).toBe(first.pane);
            if (copyUuid) second.tmux(['set-option', '-s', '@tmt.server-id', serverId]);
            expect(await second.runJsonCli(['list', 'Foreign'])).toMatchObject({
              code: 0,
              json: { identity: { name: 'Foreign' }, presence: 'active' },
            });
            const before = durableState(first);
            const trace = installTmuxTrace(second);
            const result = await second.runJsonCli<Capture>(['check', 'Foreign'], {
              outsideTmux: true,
            });
            expect(result).toMatchObject({
              code: 0,
              stderr: '',
              json: { pane: first.pane, output: expect.stringContaining('recorded-socket-only') },
            });
            const captures = trace.invocations().filter((line) => line.includes('capture-pane'));
            expect(captures).toHaveLength(1);
            expect(captures[0]).toContain(first.socketPath);
            expect(captures[0]).not.toContain(second.socketPath);
            const after = durableState(first);
            for (const binding of after.bindings) {
              const previous = before.bindings.find((row) => row.id === binding.id)!;
              expect(Date.parse(String(binding.last_verified_at))).toBeGreaterThanOrEqual(
                Date.parse(String(previous.last_verified_at))
              );
              // Reconciliation may refresh only this observation timestamp.
              binding.last_verified_at = previous.last_verified_at;
            }
            expect(after).toEqual(before);
            expect(
              await second.runJsonCli(['talk', 'Foreign', 'recorded-recipient', '--detach'])
            ).toMatchObject({ code: 0 });
            await first.waitForEvent(
              (event) =>
                event.event === 'input' && event.line?.includes('recorded-recipient') === true
            );
            expect(second.paneMetadata()).toBe('');
          },
          { ...inputLog, globalDir: first.globalDir }
        );
      }, inputLog);
    }
  );

  it('keeps selected-name inspection bounded among unrelated unbound panes', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', 'Scoped'])).toMatchObject({ code: 0 });
      await seedDiagnostic(fixture, 'scoped-native-check');
      const trace = installTmuxTrace(fixture);
      const capture = async () => {
        trace.clear();
        const result = await fixture.runJsonCli<Capture>(['check', 'Scoped'], {
          outsideTmux: true,
        });
        expect(result).toMatchObject({
          code: 0,
          json: { output: expect.stringContaining('scoped-native-check') },
        });
        return trace.invocations();
      };
      const small = await capture();
      for (let index = 0; index < 30; index++)
        fixture.tmux(['new-window', '-d', '-t', 'e2e:', '-n', `unbound-${index}`, 'sleep', '300']);
      const large = await capture();
      expect(large).toHaveLength(small.length);
      expect(small).toHaveLength(2);
      expect(large.filter((line) => line.includes('capture-pane'))).toHaveLength(1);
      const paneQueries = large.filter((line) => line.includes('list-panes'));
      expect(paneQueries).toHaveLength(1);
      expect(paneQueries[0]).toContain(' -f ');
      expect(paneQueries[0]).toContain(fixture.pane);
    }, inputLog);
  });

  it.each(['exit', 'overflow', 'timeout'] as const)(
    'returns failure without partial output after capture %s',
    async (fault) => {
      await withE2EFixture(async (fixture) => {
        const result = await fixture.runJsonCli<Capture>(['check', fixture.pane], {
          captureFault: fault,
        });
        expect(result).toMatchObject({ code: 1, stderr: '', json: { error: { code: 'ERROR' } } });
        expect(result.json).not.toHaveProperty('output');
        expect(result.stdout).not.toContain('\u0000');
        expect(fixture.paneMetadata()).toBe('');
      }, inputLog);
    }
  );
});

// The driver names only a private test pane. Its caller process is the shell
// created by the existing harness; no real Herdr or provider is involved.
describe('external caller to a recorded tmux recipient', () => {
  it('checks and sends by name after the caller tmux context is removed', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', 'Reverse'])).toMatchObject({ code: 0 });
      await seedDiagnostic(fixture, 'reverse-host-capture');
      const directory = fixture.createWorkspace('external-caller-driver');
      const driver = path.join(directory, 'tmt-driver-fake');
      fs.writeFileSync(path.join(directory, 'server'), String(fixture.serverPid));
      fs.writeFileSync(
        path.join(directory, 'capabilities'),
        JSON.stringify({
          ok: {
            protocols: [1],
            kind: 'host',
            name: 'fake',
            version: '0.0.0-test',
            ops: ['caller', 'server', 'resolve-target', 'snapshot', 'publish', 'clear'],
            paneId: { prefix: 'fake-' },
            target: 'f{n}',
            callerEnv: ['FAKE_PANE_ID'],
          },
        })
      );
      writeExecutable(
        driver,
        `#!/bin/sh
 d=$(dirname "$0")
 echo "$3" >> "$d/calls"
 cat >/dev/null
 case "$3" in
 capabilities) cat "$d/capabilities" ;;
 server) printf '{"ok":{"server":{"socket":"/tmp/fake-caller.sock","pid":%s,"startTime":"driver"}}}' "$(cat "$d/server")" ;;
 caller) printf '{"ok":{"pane":{"id":"fake-1","socket":"/tmp/fake-caller.sock","shellPid":%s}}}' "$(cat "$d/pid")" ;;
 snapshot) printf '{"ok":{"panes":[{"id":"fake-1","target":"f1","cwd":"/tmp","command":"sh","panePid":%s,"suggestedName":null,"marker":null}]}}' "$(cat "$d/pid")" ;;
 *) printf '{"error":{"code":"unsupported","message":"not used"}}' ;;
 esac
`
      );
      expect(await fixture.runJsonCli(['driver', 'install', driver, '--yes'])).toMatchObject({
        code: 0,
      });
      fixture.tmux(['set-environment', '-g', 'FAKE_PANE_ID', 'f1']);
      for (const [index, args] of [
        ['check', 'Reverse'],
        ['talk', 'Reverse', 'reverse-host-send', '--detach'],
      ].entries()) {
        const process = await spawnRealTmuxCli(fixture, args, {
          name: `external-caller-${index}`,
          stripTmux: true,
          stripPane: true,
        });
        fs.writeFileSync(path.join(directory, 'pid'), String(process.panePid));
        await releaseRealTmuxCli(fixture, process);
        const result = readRealTmuxCli<Capture>(process);
        expect(result.code).toBe(0);
        expect(result.stderr).toBe('');
        if (index === 0)
          expect(result.stdout).toMatchObject({
            pane: fixture.pane,
            output: expect.stringContaining('reverse-host-capture'),
          });
      }
      await fixture.waitForEvent(
        (event) => event.event === 'input' && event.line?.includes('reverse-host-send') === true
      );
      expect(fs.readFileSync(path.join(directory, 'calls'), 'utf8')).toContain('caller');
    }, inputLog);
  });
});
