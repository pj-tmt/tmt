import fs from 'node:fs';
import path from 'node:path';
import { expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { withE2EFixture, type E2EFixture } from './harness.js';

// A provider hook in tmux inside an external host's pane (#3583). The fake
// host's caller variable is set, and its driver names a pane whose shell is
// not an ancestor of the hook (the tmux server daemonizes, so a real outer
// pane never is). On one pinned ancestry the tmux pane owns the caller, and
// the hook records the session on it. A driver that cannot answer is doubt:
// the hook refuses and never falls back to tmux. Real tmux on the fixture's
// private socket; the external host is a shell-script driver.

const CAPABILITIES = JSON.stringify({
  ok: {
    protocols: [1],
    kind: 'host',
    name: 'fake',
    version: '0.0.0-test',
    ops: ['caller', 'server', 'snapshot', 'publish', 'clear'],
    paneId: { prefix: 'fake-' },
    target: 'f{n}',
    callerEnv: ['FAKE_PANE_ID'],
  },
});

const UNAVAILABLE = 'tmt: lifecycle context unavailable; continuing without context.\n';

/** An approved driver whose `caller` prints the file `caller` beside it. */
async function approveDriver(fixture: E2EFixture, caller: string): Promise<string> {
  const directory = fixture.createWorkspace('nested-driver');
  fs.chmodSync(directory, 0o755);
  const driver = path.join(directory, 'tmt-driver-fake');
  writeExecutable(
    driver,
    [
      '#!/bin/sh',
      'd=$(dirname "$0")',
      'echo "$3" >> "$d/calls"',
      'cat > /dev/null',
      'case "$3" in',
      '  capabilities) cat "$d/capabilities" ;;',
      '  caller) cat "$d/caller" ;;',
      '  *) printf \'{"error":{"code":"unsupported","message":"no %s"}}\' "$3" ;;',
      'esac',
      '',
    ].join('\n'),
    0o755
  );
  fs.writeFileSync(path.join(directory, 'capabilities'), CAPABILITIES);
  fs.writeFileSync(path.join(directory, 'caller'), caller);
  const installed = await fixture.runCli(['driver', 'install', driver, '--yes']);
  expect(installed.code, installed.stderr).toBe(0);
  return directory;
}

type StepResult = { code: number; stdout: string; stderr: string };

/** Names the pane's identity, runs SessionStart and lists, from a
 * stand-in Claude in a tmux pane with the fake host's variable set. */
async function hookInPane(fixture: E2EFixture, session: string): Promise<StepResult[]> {
  const steps = [
    { args: ['name', 'Nested Reader', '-s', '--json'] },
    {
      args: ['__hook', 'claude'],
      input: { hook_event_name: 'SessionStart', session_id: session, source: 'startup' },
    },
    { args: ['ls', '--json'] },
  ];
  const scenario = path.join(fixture.root, 'nested-scenario.json');
  const report = path.join(fixture.root, 'nested-report.json');
  fs.writeFileSync(scenario, JSON.stringify(steps));
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  const command = [
    'env',
    `TMT_HOME=${fixture.globalDir}`,
    'FAKE_PANE_ID=f1',
    '/opt/tmt-tests/claude',
    fixture.executables.cli.executable,
    scenario,
    report,
  ]
    .map(quote)
    .join(' ');
  const pane = fixture.createShellPane('nested').pane;
  fixture.tmux(['send-keys', '-t', pane, '-l', command]);
  fixture.tmux(['send-keys', '-t', pane, 'Enter']);
  await fixture.waitFor(() => fs.existsSync(report), 15000, 'nested hook report');
  const results = JSON.parse(fs.readFileSync(report, 'utf8')) as StepResult[];
  expect(results).toHaveLength(steps.length);
  return results;
}

function resumeSession(listed: string): unknown {
  return JSON.parse(listed).identities.find(
    (item: { name: string }) => item.name === 'Nested Reader'
  ).resume?.session;
}

it('a hook in tmux inside an external pane records its session on the tmux pane', async () => {
  await withE2EFixture(async (fixture) => {
    const driver = await approveDriver(
      fixture,
      '{"ok":{"pane":{"id":"fake-1","socket":"/tmp/fake.sock","shellPid":999999}}}'
    );
    const session = '33333333-3333-4333-8333-333333333333';
    const [named, hook, listed] = await hookInPane(fixture, session);
    expect(named.code, named.stderr).toBe(0);
    expect(hook).toMatchObject({ code: 0, stderr: '' });
    expect(JSON.parse(hook.stdout).hookSpecificOutput.hookEventName).toBe('SessionStart');
    expect(listed.code, listed.stderr).toBe(0);
    expect(resumeSession(listed.stdout)).toBe(session);
    // The nested proof asked the external host, which proved its pane absent.
    expect(fs.readFileSync(path.join(driver, 'calls'), 'utf8')).toContain('caller');
  });
});

it('a hook whose external driver cannot answer refuses instead of using tmux', async () => {
  await withE2EFixture(async (fixture) => {
    await approveDriver(fixture, '{"error":{"code":"failed","message":"host is wedged"}}');
    const [named, hook, listed] = await hookInPane(fixture, '44444444-4444-4444-8444-444444444444');
    expect(named.code, named.stderr).toBe(0);
    expect(hook).toMatchObject({ code: 0, stdout: '', stderr: UNAVAILABLE });
    expect(listed.code, listed.stderr).toBe(0);
    expect(resumeSession(listed.stdout) ?? null).toBeNull();
  });
});
