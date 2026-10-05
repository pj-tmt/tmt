import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  annotationInput,
  clientId,
  createPage,
  openPage,
  run,
  selectInRenderer,
} from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

interface SdkWindow {
  session?: unknown;
  ops?: { listAgents(): Promise<unknown[]> };
}

// The served SDK keeps each tab on its own signed operation lane. Opening a
// ninth session evicts the oldest under Remote's default limit of eight.
test('Remote keeps concurrent sessions per device and reports the evicted limit', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world);
    await world.startAgent('ask-recipient');
    const browser = await pairBrowser(world, 'one-device');
    await browser.context.route('**/colab/assets/*.js', (route) => route.abort());
    const tabs = await Promise.all(Array.from({ length: 9 }, () => browser.context.newPage()));
    for (const tab of tabs) await tab.goto(`${door.mounts}colab/`);

    const open = (page: (typeof tabs)[number]) =>
      page.evaluate(async () => {
        const sdk = (await import('/sdk/remote-v1.js' as string)) as {
          reopenSession(): Promise<unknown>;
          operations(session: unknown): { listAgents(): Promise<unknown[]> };
        };
        const w = window as unknown as SdkWindow;
        w.session = await sdk.reopenSession();
        w.ops = sdk.operations(w.session);
        return (await w.ops.listAgents()).length;
      });
    const list = (page: (typeof tabs)[number]) =>
      page.evaluate(async () => {
        try {
          return { agents: (await (window as unknown as SdkWindow).ops!.listAgents()).length };
        } catch (error) {
          const refused = error as { code?: string; limit?: number; settingsUrl?: string };
          return { code: refused.code, limit: refused.limit, settingsUrl: refused.settingsUrl };
        }
      });

    expect(await open(tabs[0])).toBe(1);
    expect(await open(tabs[1])).toBe(1);
    expect(await list(tabs[0])).toEqual({ agents: 1 });
    expect(await list(tabs[1])).toEqual({ agents: 1 });
    for (const tab of tabs.slice(2)) expect(await open(tab)).toBe(1);
    expect(await list(tabs[0])).toMatchObject({ code: 'REMOTE_SESSION_EVICTED', limit: 8 });
    for (const tab of tabs.slice(1)) expect(await list(tab)).toEqual({ agents: 1 });
  });
});

test('an evicted Colab tab shows the cap while the other tabs stay live', async () => {
  await withWorld(async (world) => {
    run(world, world.binaries.remote, ['settings', 'sessions-per-device', '2']);
    const door = await startDoor(world);
    await world.startAgent('ask-recipient');
    const created = createPage(world, 'Several tabs', '<h1>Several tabs</h1>');
    const browser = await pairBrowser(world, 'one-device');
    const tabs = [];
    for (let index = 0; index < 3; index++) {
      const tab = await openPage(door, browser, created);
      await expect(
        tab.frameLocator('iframe').getByRole('heading', { name: 'Several tabs' }),
      ).toBeVisible();
      tabs.push(tab);
    }
    await expect(tabs[0].getByRole('alert')).toContainText('limit of 2 Remote sessions');
    await expect(tabs[0].getByRole('alert')).toContainText(
      'tmt remote settings sessions-per-device 3',
    );
    const captures = process.env.COLAB_1768_CAPTURE_DIR;
    if (captures) {
      mkdirSync(captures, { recursive: true });
      for (const [tab, state] of [
        [tabs[0], 'evicted'],
        [tabs[2], 'live'],
      ] as const)
        for (const theme of ['light', 'dark']) {
          await tab.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
          for (const width of [1440, 390]) {
            await tab.setViewportSize({ width, height: 900 });
            await tab.screenshot({
              path: `${captures}/${state}-${width}-${theme}.png`,
              fullPage: true,
            });
          }
        }
    }
    for (const tab of tabs.slice(1)) {
      await expect(
        tab.frameLocator('iframe').getByRole('heading', { name: 'Several tabs' }),
      ).toBeVisible();
    }
    await tabs[1].close();
    await expect(
      tabs[2].frameLocator('iframe').getByRole('heading', { name: 'Several tabs' }),
    ).toBeVisible();
    await expect(tabs[0].getByRole('alert')).toContainText('limit of 2 Remote sessions');
  });
});

test('three tabs on two pages sync edits and annotations, then all lose a revoked device', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world);
    const agent = await world.startAgent('several-tabs-agent');
    const browser = await pairBrowser(world, 'several-tabs-device');
    const first = createPage(
      world,
      'First page',
      '<h1>First page</h1><p id="quote">Quote A.</p>',
      agent.pane,
    );
    const second = createPage(
      world,
      'Second page',
      '<h1>Second page</h1><p id="quote">Quote B.</p>',
      agent.pane,
    );
    const a = await openPage(door, browser, first);
    const b = await openPage(door, browser, first);
    const c = await openPage(door, browser, second);
    for (const tab of [a, b])
      await expect(
        tab.frameLocator('iframe').getByRole('heading', { name: 'First page' }),
      ).toBeVisible();
    await expect(
      c.frameLocator('iframe').getByRole('heading', { name: 'Second page' }),
    ).toBeVisible();

    await a.getByRole('button', { name: 'Source', exact: true }).click();
    await a.getByRole('textbox').fill('<h1>Edited in A</h1><p id="quote">Quote A.</p>');
    await a.getByRole('button', { name: 'Save source' }).click();
    await expect(
      b.frameLocator('iframe').getByRole('heading', { name: 'Edited in A' }),
    ).toBeVisible();
    await expect(
      c.frameLocator('iframe').getByRole('heading', { name: 'Second page' }),
    ).toBeVisible();

    for (const [tab, quote, message] of [
      [b, 'Quote A.', 'From the second tab.'],
      [c, 'Quote B.', 'From the other page.'],
    ] as const) {
      await selectInRenderer(tab, '#quote');
      await tab.getByTestId('selection-ask').click();
      const input = await annotationInput(tab.getByTestId('annotation-compose'), agent.name);
      await input.fill(`@${agent.name} ${message}`);
      await input.press('Enter');
      await expect(tab.getByTestId('comment-thread')).toContainText(quote);
    }
    await expect(a.getByTestId('comments-toggle')).toBeVisible();
    await a.getByTestId('comments-toggle').click();
    await expect(a.getByTestId('annotation-row')).toContainText('Quote A.');
    await expect(a.getByTestId('annotation-row')).toHaveCount(1);
    if ((await c.getByTestId('comments-toggle').getAttribute('aria-expanded')) === 'false')
      await c.getByTestId('comments-toggle').click();
    await expect(c.getByTestId('annotation-row')).toHaveCount(1);
    await expect(c.getByTestId('annotation-row')).toContainText('Quote B.');

    run(world, world.binaries.remote, [
      'devices',
      'revoke',
      clientId(world, 'several-tabs-device'),
    ]);
    for (const tab of [a, b, c]) {
      await tab.reload();
      await expect(tab.locator('#colab-guidance')).toBeVisible();
    }
  });
});
