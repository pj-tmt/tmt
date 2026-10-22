import { expect, test } from '@playwright/test';
import { createPage, openPage, run } from './harness/ask.js';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
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

// #2839: the receipt-bound driver represents a second version; the frozen production binaries
// own the real Remote restart, paired browser, page write and rendering.
test('an installed Colab upgrade restarts its owned door on the same link and admits the new CLI', async () => {
  await withWorld(async (world) => {
    world.linkExtensions();
    const lib = path.join(world.root, 'lib/tmt-colab');
    const old = path.join(lib, 'releases/old');
    const next = path.join(lib, 'releases/next');
    fs.mkdirSync(old, { recursive: true });
    fs.mkdirSync(next);
    fs.copyFileSync(world.binaries.colab, path.join(old, 'tmt-colab'));
    fs.chmodSync(path.join(old, 'tmt-colab'), 0o755);
    fs.writeFileSync(
      path.join(old, 'receipt.json'),
      JSON.stringify({ version: 'running-fixture' }),
    );
    fs.symlinkSync('releases/old', path.join(lib, 'current'));
    const colab = world.spawn('installed-colab', path.join(old, 'tmt-colab'), [
      'serve',
      '--json',
      '--no-open',
    ]);
    const first = await colab.event((value) => value.state === 'mounted');
    expect(first.door).toBe('started');
    world.recordColabServe();
    const address = String(first.url).replace(/\/x\/colab\/$/, '');
    const origin = String(first.origin);
    const device = await pairBrowser(world, 'upgrade-device');
    const created = createPage(world, 'Upgrade', '<p id="body">Before upgrade</p>');
    const page = await openPage({ address }, device, created);
    await expect(page.frameLocator('iframe').locator('#body')).toHaveText('Before upgrade');
    const prepared = JSON.parse(
      run(world, world.binaries.colab, ['serve', '--upgrade-check', '--json', '--no-open']),
    ) as Record<string, unknown>;
    prepared.version = '9.9.9-upgrade-proof';
    const word = (value: string) => "'" + value.replaceAll("'", "'\"'\"'") + "'";
    const program = `#!/bin/sh
if [ "$2" = '--upgrade-check' ]; then printf '%s\\n' ${word(JSON.stringify(prepared))}; exit 0; fi
exec ${word(world.binaries.colab)} "$@"
`;
    fs.writeFileSync(path.join(next, 'tmt-colab'), program, { mode: 0o755 });
    fs.writeFileSync(
      path.join(next, 'receipt.json'),
      JSON.stringify({
        version: prepared.version,
        file_sha256: { 'tmt-colab': createHash('sha256').update(program).digest('hex') },
      }),
    );
    fs.unlinkSync(path.join(lib, 'current'));
    fs.symlinkSync('releases/next', path.join(lib, 'current'));
    let ready: Record<string, unknown> | undefined;
    await until(
      () => {
        const mounted = colab.lines.flatMap((line) => {
          try {
            const value = JSON.parse(line) as Record<string, unknown>;
            return value.state === 'mounted' ? [value] : [];
          } catch {
            return [];
          }
        });
        ready = mounted[1];
        return ready !== undefined;
      },
      'replacement Colab readiness',
      30_000,
    );
    expect(ready).toMatchObject({ door: 'started', origin, url: first.url, opened: false });
    world.recordColabServe();
    run(
      world,
      world.binaries.colab,
      ['page', 'write', created.pageId, '--file', '-', '--json'],
      '<p id="body">After upgrade</p>',
    );
    expect(
      JSON.parse(run(world, world.binaries.colab, ['page', 'read', created.pageId, '--json']))
        .source,
    ).toBe('<p id="body">After upgrade</p>');
    const reopened = await openPage({ address }, device, created);
    await expect(reopened.frameLocator('iframe').locator('#body')).toHaveText('After upgrade');
    expect(await doorAnswers(origin)).toBe(true);
    const stopped = await world.tmt(['colab', 'stop', '--json']);
    expect(stopped.code, stopped.stdout + stopped.stderr).toBe(0);
    expect(JSON.parse(stopped.stdout)).toEqual({ state: 'stopped', door: 'started' });
    await colab.exited;
    await until(async () => !(await doorAnswers(origin)), 'the replacement owned door closing');
  });
});

// #1584: from a clean state one `tmt-colab serve` plus pairing opens a page, and stopping it
// closes both. Real core, Remote and Colab binaries and a real Chromium device; Colab reaches
// Remote only through the core's public CLI.
test('one command starts the door; pairing opens a page; stopping it closes both', async () => {
  await withWorld(async (world) => {
    const door = await startServe(world);
    expect(door.state).toBe('started');
    expect(door.url).toBe(`${door.origin}/colab/`);
    // A clean state: nothing paired and no page yet, and the status says what to do next.
    expect(door.status).toMatchObject({
      origin: door.origin,
      paired: false,
      pages: 0,
      page: null,
      next: ['tmt remote pair', 'tmt colab page create --title <title> --layout column'],
    });
    const device = await pairBrowser(world, 'one-command');
    // The printed link itself opens as the paired device.
    const printed = await device.context.newPage();
    expect((await printed.goto(door.url))?.status()).toBe(200);
    await openColab(door, device);
    // #2385: JSON emits the same short link in both link fields. Both open
    // through the real Remote door as the paired device without a hand-built URL.
    const printed2 = await world.tmt(
      [
        'colab',
        'page',
        'create',
        '--title',
        'Linked',
        '--layout',
        'column',
        '--file',
        '-',
        '--json',
        '--no-open',
      ],
      { stdin: '<p id="body">Opened from the printed link.</p>' },
    );
    expect(printed2.code, printed2.stdout + printed2.stderr).toBe(0);
    const linked = JSON.parse(printed2.stdout) as {
      link: string;
      shortLink: string;
      path: string;
      pageId: string;
      paired: boolean;
    };
    expect(linked.link).toBe(new URL(linked.path, door.origin).href);
    expect(linked.paired).toBe(true);
    expect(linked.shortLink).toBe(`${door.origin}/colab/p/${linked.pageId.slice(0, 8)}`);
    const linkedPage = await device.context.newPage();
    expect((await linkedPage.goto(linked.link))?.status()).toBe(200);
    // The link renders the page, not just an HTTP 200: the sandboxed frame shows its body.
    await expect(linkedPage.locator('iframe')).toBeVisible();
    await expect(linkedPage.frameLocator('iframe').locator('#body')).toHaveText(
      'Opened from the printed link.',
    );
    const shortPage = await device.context.newPage();
    expect((await shortPage.goto(linked.shortLink))?.status()).toBe(200);
    await expect(shortPage.frameLocator('iframe').locator('#body')).toHaveText(
      'Opened from the printed link.',
    );
    expect(shortPage.url()).toBe(new URL(`/colab/pages/${linked.pageId}`, door.origin).href);
    await shortPage.reload();
    await expect(shortPage.frameLocator('iframe').locator('#body')).toHaveText(
      'Opened from the printed link.',
    );
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
