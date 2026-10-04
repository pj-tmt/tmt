import { expect, test } from '@playwright/test';
import { createPage, openPage } from './harness/ask.js';
import {
  doorAnswers,
  openColab,
  pairBrowser,
  startRemoteOnly,
  startServe,
} from './harness/browser.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

// #1584: from a clean state one `tmt-colab serve` plus pairing opens a page, and stopping it
// closes both. Real core, Remote and Colab binaries and a real Chromium device; Colab reaches
// Remote only through the core's public CLI.
test('one command starts the door; pairing opens a page; stopping it closes both', async () => {
  await withWorld(async (world) => {
    const door = await startServe(world);
    expect(door.state).toBe('started');
    expect(door.url).toBe(`${door.address}/x/colab/`);
    // A clean state: nothing paired and no page yet, and the status says what to do next.
    expect(door.status).toMatchObject({
      origin: door.origin,
      paired: false,
      pages: 0,
      page: null,
      next: ['tmt remote pair', 'tmt colab page create --title <title>'],
    });
    const device = await pairBrowser(world, 'one-command');
    // The printed link itself opens as the paired device.
    const printed = await device.context.newPage();
    expect((await printed.goto(door.url))?.status()).toBe(200);
    await openColab(door, device);
    // `page create` prints the full link (#1614): the door address plus the page path, and it
    // opens as the paired device without any hand-built URL.
    const printed2 = await world.tmt(['colab', 'page', 'create', '--title', 'Linked', '--json']);
    expect(printed2.code, printed2.stdout + printed2.stderr).toBe(0);
    const linked = JSON.parse(printed2.stdout) as { link: string; path: string; paired: boolean };
    expect(linked.link).toBe(`${door.address}/${linked.path}`);
    expect(linked.paired).toBe(true);
    const linkedPage = await device.context.newPage();
    expect((await linkedPage.goto(linked.link))?.status()).toBe(200);
    const created = createPage(world, 'One command', '<p id="body">Opened by one command.</p>');
    const page = await openPage(door, device, created);
    await expect(page.locator('iframe')).toBeVisible();
    await expect(page.frameLocator('iframe').locator('#body')).toHaveText('Opened by one command.');

    // Stopping Colab stops the door it started; nothing is left listening.
    expect(await doorAnswers(door.origin)).toBe(true);
    await door.colab.stop();
    await until(async () => !(await doorAnswers(door.origin)), 'the started door closing');
  });
});

// #1594: `tmt colab stop` asks the serving Colab over its owner-only socket; the door it started
// closes with it, pairings stay, and a second stop is a clear success.
test('tmt colab stop ends Colab and the door it started, and is idempotent', async () => {
  await withWorld(async (world) => {
    const door = await startServe(world);
    expect(door.state).toBe('started');
    const device = await pairBrowser(world, 'stop-device');
    expect(await doorAnswers(door.origin)).toBe(true);

    const stopped = await world.tmt(['colab', 'stop', '--json']);
    expect(stopped.code, stopped.stdout + stopped.stderr).toBe(0);
    expect(JSON.parse(stopped.stdout)).toEqual({ state: 'stopped', door: 'started' });
    await door.colab.exited;
    expect(await doorAnswers(door.origin)).toBe(false);

    const again = await world.tmt(['colab', 'stop', '--json']);
    expect(again.code).toBe(0);
    expect(JSON.parse(again.stdout)).toEqual({ state: 'not-running', door: null });

    // Pairings survive: the same device is still listed by Remote.
    const devices = await world.tmt(['remote', 'devices', '--json']);
    expect(devices.code, devices.stderr).toBe(0);
    expect(JSON.parse(devices.stdout).devices.map((d: { name: string }) => d.name)).toContain(
      device.name,
    );
  });
});

// A door started outside Colab is attached to, never started or stopped by Colab (#1584, #1594).
test('Colab attaches to a running door; stop and exit leave that door running', async () => {
  await withWorld(async (world) => {
    const remote = await startRemoteOnly(world);
    const door = await startServe(world);
    expect(door.state).toBe('attached');
    expect(door.address).toBe(remote.address);
    const stopped = await world.tmt(['colab', 'stop', '--json']);
    expect(stopped.code, stopped.stdout + stopped.stderr).toBe(0);
    expect(JSON.parse(stopped.stdout)).toEqual({ state: 'stopped', door: 'attached' });
    await door.colab.exited;
    expect(await doorAnswers(door.origin)).toBe(true);
    await remote.remote.stop();
    expect(await doorAnswers(door.origin)).toBe(false);
  });
});
