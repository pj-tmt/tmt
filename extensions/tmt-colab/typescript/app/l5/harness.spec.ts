import fs from 'node:fs';
import { expect, test } from '@playwright/test';
import { openColab, pairBrowser, startDoor } from './harness/browser.js';
import { withWorld } from './harness/with-world.js';

// Proves the harness itself, independent of the Ask agent code: real binaries,
// real pairing of two browser devices, a recipient whose durable counter and
// real `tmt reply` carry the work, and a world that leaves nothing behind.
test('harness: two paired browsers, a counted recipient and a clean teardown', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world);
    const recipient = await world.startAgent('l5-recipient');
    const asker = await world.startAgent('l5-asker');
    const first = await pairBrowser(world, 'viewer-one');
    const second = await pairBrowser(world, 'viewer-two');
    expect(first.profile).not.toBe(second.profile);
    await openColab(door, first);
    await openColab(door, second);

    // Real core delivery to the recipient: one wake, one counted receipt, one real reply.
    const talk = await world.tmt(['talk', recipient.name, 'harness probe', '--json'], {
      pane: asker.pane,
    });
    expect(talk.code, talk.stdout + talk.stderr).toBe(0);
    const received = recipient.received();
    expect(received).toHaveLength(1);
    expect(received[0]).toMatchObject({ source: 'talk', message: 'harness probe' });
    const replied = recipient.rows().filter((row) => row.event === 'replied');
    expect(replied).toHaveLength(1);
    expect(recipient.rows().filter((row) => row.event === 'failure')).toEqual([]);
  });
});

// Sensitivity of the teardown guard: an untracked survivor naming the world's
// root must be reported, so a green cleanup cannot come from a blind check.
test('harness: the leak report detects a process the world did not stop', async () => {
  const { spawn } = await import('node:child_process');
  const { L5World } = await import('./harness/world.js');
  const world = new L5World();
  await world.start();
  const stray = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1e6)', world.root], {
    detached: true,
    stdio: 'ignore',
  });
  try {
    const leaks = await world.dispose();
    expect(leaks.filter((leak) => leak.startsWith('process remains'))).toHaveLength(1);
  } finally {
    process.kill(stray.pid ?? 0, 'SIGKILL');
  }
});

// An assertion failing mid-scenario must still stop every child, the tmux server
// and the browsers, and must surface the scenario's own error, not a cleanup one.
test('harness: an injected scenario failure still tears the world down', async () => {
  const { withWorld } = await import('./harness/with-world.js');
  const { openColab, pairBrowser, startDoor } = await import('./harness/browser.js');
  let root = '';
  await expect(
    withWorld(async (world) => {
      root = world.root;
      const door = await startDoor(world);
      await openColab(door, await pairBrowser(world, 'viewer-failing'));
      throw new Error('injected scenario failure');
    }),
  ).rejects.toThrow('injected scenario failure');
  expect(fs.existsSync(root)).toBe(false);
  const { execFileSync } = await import('node:child_process');
  expect(execFileSync('/bin/ps', ['-axo', 'command='], { encoding: 'utf8' })).not.toContain(root);
});
