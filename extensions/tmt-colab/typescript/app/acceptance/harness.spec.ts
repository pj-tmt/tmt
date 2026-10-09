import fs from 'node:fs';
import { expect, test } from '@playwright/test';
import { openColab, pairBrowser, startDoor } from './harness/browser.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

// Proves the harness itself, independent of the Ask agent code: real binaries,
// real pairing of two browser devices, a recipient whose durable counter and
// real `tmt reply` carry the work, and a world that leaves nothing behind.
test('harness: two paired browsers, a counted recipient and a clean teardown', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world);
    const recipient = await world.startAgent('ask-recipient');
    const asker = await world.startAgent('ask-asker');
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
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld();
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

// A test that times out is abandoned, not cancelled (#2327): its afterEach disposes the world
// while the scenario may still be running. These cases drive that path without waiting for a
// real Playwright timeout.
const idle = ['-e', 'setInterval(() => {}, 1e6)'];
const processes = async () =>
  (await import('node:child_process')).execFileSync('/bin/ps', ['-axo', 'command='], {
    encoding: 'utf8',
  });

test('harness: the timeout cleanup reports what a timed-out world leaked', async () => {
  const { spawn } = await import('node:child_process');
  let release = () => {};
  const parked = new Promise<void>((resolve) => (release = resolve));
  let root = '';
  let stray: ReturnType<typeof spawn> | undefined;
  const scenario = withWorld(async (world) => {
    root = world.root;
    stray = spawn(process.execPath, [...idle, world.root], { detached: true, stdio: 'ignore' });
    await parked;
  });
  await expect.poll(() => root).not.toBe('');
  try {
    // What the afterEach of a timed-out test does: dispose and surface the leak report.
    await expect(disposeActiveWorlds()).rejects.toThrow(/leaked[\s\S]*process remains/);
  } finally {
    process.kill(stray?.pid ?? 0, 'SIGKILL');
    release();
    await scenario.catch(() => {});
  }
  expect(fs.existsSync(root)).toBe(false);
});

test('harness: a scenario that outlives its dispose cannot add a child, a tmux call or a closer', async () => {
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld();
  await world.start();
  const root = world.root;
  expect(await world.dispose()).toEqual([]);
  expect(() => world.spawn('late', process.execPath, idle)).toThrow(/disposed/);
  expect(() => world.tmux(['list-sessions'])).toThrow(/disposed/);
  expect(() => world.onDispose(async () => {})).toThrow(/disposed/);
  await expect(world.tmt(['--version'])).rejects.toThrow(/disposed/);
  expect(fs.existsSync(root)).toBe(false);
  expect(await processes()).not.toContain(root);
});

test('harness: a timeout in the middle of spawning leaves no child behind', async () => {
  let root = '';
  let spawned = () => {};
  const first = new Promise<void>((resolve) => (spawned = resolve));
  const scenario = withWorld(async (world) => {
    root = world.root;
    world.spawn('first', process.execPath, idle);
    spawned();
    // The abandoned scenario is still working when the cleanup runs.
    await new Promise((resolve) => setTimeout(resolve, 300));
    world.spawn('second', process.execPath, idle);
  });
  await first;
  await disposeActiveWorlds();
  await expect(scenario).rejects.toThrow(/disposed/);
  expect(fs.existsSync(root)).toBe(false);
  expect(await processes()).not.toContain(root);
});

test('harness: a closer that never finishes is bounded and reported, and the rest is still torn down', async () => {
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld(undefined, { closerBoundMs: 300 });
  await world.start();
  world.spawn('kept', process.execPath, idle);
  world.onDispose((stage) => {
    stage('closing the browser context');
    return new Promise<void>(() => {});
  });
  const started = Date.now();
  const leaks = await world.dispose();
  expect(Date.now() - started).toBeLessThan(15_000);
  expect(leaks).toEqual([
    expect.stringContaining('did not finish within 300 ms (in closing the browser context)'),
  ]);
  expect(fs.existsSync(world.root)).toBe(false);
  const listing = await processes();
  expect(listing).not.toContain(world.root);
  expect(listing).not.toContain(` -L ${world.tmuxName}`);
});
