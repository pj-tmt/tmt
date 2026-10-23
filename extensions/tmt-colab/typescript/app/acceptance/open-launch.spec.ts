import fs from 'node:fs';
import { expect, test } from '@playwright/test';
import { createPage, freePort, run } from './harness/ask.js';
import { freshBrowser, startDoor } from './harness/browser.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

/** Consume the intentional stand-in record before an assertion can fail. Any later opener call
 * still leaves a new record for the world's unchanged leak check. */
function takeLaunches(file: string): string[] {
  if (!fs.existsSync(file)) return [];
  const lines = fs.readFileSync(file, 'utf8').trim().split('\n');
  fs.rmSync(file);
  return lines;
}

test.afterEach(disposeActiveWorlds);
test('tmt colab open connects an unpaired browser through Remote and prints only the short link', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const created = createPage(world, 'Launch proof', '<h1 id="proof">Launch route content</h1>');
    const shown = JSON.parse(
      run(world, world.binaries.colab, ['show', created.pageId, '--json']),
    ) as { shortLink: string };
    const opens = () =>
      fs.existsSync(world.openerLog)
        ? fs.readFileSync(world.openerLog, 'utf8').trim().split('\n')
        : [];

    // Suppressed opening creates no offer: the opener stand-ins are never called.
    for (const flag of ['--no-open', '--json'])
      run(world, world.binaries.colab, ['open', created.pageId, flag]);
    expect(opens()).toEqual([]);

    // The world is an agent run, so `--open` asks for the browser explicitly.
    const printed = run(world, world.binaries.colab, ['open', created.pageId, '--open']);
    expect(printed).toContain(`opened in your browser: ${shown.shortLink}`);
    expect(printed).not.toMatch(/auto=|\/pair/);
    const launches = takeLaunches(world.openerLog);
    expect(launches).toHaveLength(1);
    // Only the harness's opener stand-in sees the private launch link; Colab never printed it.
    const launch = launches[0].replace(/^\S+ /, '');
    expect(launch).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/remote\/pair#auto=/);

    const browser = await freshBrowser(world, 'launched');
    await browser.page.goto(launch);
    await expect(browser.page.frameLocator('iframe').locator('#proof')).toHaveText(
      'Launch route content',
    );
    expect(browser.page.url()).toBe(new URL(`/colab/pages/${created.pageId}`, door.origin).href);
    expect(door.address).toContain('/r/');
  });
});

test('page create opens the committed page through Remote with the same short-only output', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    world.linkExtensions();
    const created = JSON.parse(
      run(
        world,
        world.binaries.colab,
        [
          'page',
          'create',
          '--title',
          'Created launch',
          '--layout',
          'column',
          '--file',
          '-',
          '--open',
          '--json',
        ],
        '<h1 id="proof">Created route content</h1>',
      ),
    ) as { opened: boolean; shortLink: string; pageId: string };
    const launches = takeLaunches(world.openerLog);
    expect(created.opened).toBe(true);
    expect(created.shortLink).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/colab\/p\//);
    expect(launches).toHaveLength(1);
    expect(JSON.stringify(created)).not.toMatch(/auto=|\/pair/);
    const launch = launches[0].replace(/^\S+ /, '');
    const browser = await freshBrowser(world, 'created');
    await browser.page.goto(launch);
    await expect(browser.page.frameLocator('iframe').locator('#proof')).toHaveText(
      'Created route content',
    );
    expect(browser.page.url()).toBe(new URL(`/colab/pages/${created.pageId}`, door.origin).href);
  });
});
