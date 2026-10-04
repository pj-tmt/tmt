import { expect, test } from '@playwright/test';
import { createPage, openPage } from './harness/ask.js';
import { doorAnswers, openColab, pairBrowser, startServe } from './harness/browser.js';
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
