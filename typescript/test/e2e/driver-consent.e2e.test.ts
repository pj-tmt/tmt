import { writeExecutable } from '../support/executable-fixture.mjs';
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { spawnRealTmuxCli } from './real-tmux-caller.js';

// `tmt driver install` asks before approving a host driver (#570 slice 6).
// The Rust e2e covers --yes and the non-interactive refusal; this answers
// the question on a real terminal.

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

/** A shell-script driver that answers `capabilities` from the file beside it. */
function writeDriver(fixture: E2EFixture): string {
  const directory = fixture.createWorkspace('fake-driver');
  fs.chmodSync(directory, 0o755);
  const driver = path.join(directory, 'tmt-driver-fake');
  writeExecutable(driver, '#!/bin/sh\ncat "$(dirname "$0")/capabilities"\n', 0o755);
  fs.writeFileSync(path.join(directory, 'capabilities'), CAPABILITIES);
  return driver;
}

async function answer(fixture: E2EFixture, driver: string, reply: string): Promise<string> {
  const process = await spawnRealTmuxCli(fixture, ['driver', 'install', driver], {
    name: `driver-consent-${reply || 'empty'}`,
    json: false,
    terminal: true,
  });
  fs.writeFileSync(process.releasePath, 'run');
  await fixture.waitForCapture(
    (text) => text.includes('Approve host driver fake? [y/N]'),
    process.pane
  );
  expect(fs.existsSync(process.exitPath)).toBe(false);
  if (reply) fixture.tmux(['send-keys', '-t', process.pane, '-l', reply]);
  fixture.tmux(['send-keys', '-t', process.pane, 'Enter']);
  await fixture.waitFor(
    () => fs.existsSync(process.exitPath) && fs.readFileSync(process.exitPath, 'utf8') === '0',
    5_000,
    'driver consent exit'
  );
  return fixture.tmux(['capture-pane', '-p', '-J', '-S', '-', '-t', process.pane]);
}

describe('host driver consent on a real terminal', () => {
  it.each(['', 'n', 'no'])('declining with %j approves nothing', async (reply) => {
    await withE2EFixture(async (fixture) => {
      const driver = writeDriver(fixture);
      const screen = await answer(fixture, driver, reply);
      expect(screen).toContain('Host driver fake');
      expect(screen).toContain('version     0.0.0-test');
      expect(screen).toContain('Host driver fake was not approved; nothing changed.');
      const listed = await fixture.runJsonCli(['driver', 'ls']);
      expect(listed.json).toEqual({ drivers: [] });
    });
  });

  it('accepting approves the driver that was shown', async () => {
    await withE2EFixture(async (fixture) => {
      const driver = writeDriver(fixture);
      const screen = await answer(fixture, driver, 'y');
      expect(screen).toContain('Approved host driver fake. Remove it with: tmt driver rm fake');
      const listed = await fixture.runJsonCli(['driver', 'ls']);
      expect(listed.json).toMatchObject({
        drivers: [{ name: 'fake', state: 'ok', path: fs.realpathSync(driver) }],
      });
    });
  });
});
