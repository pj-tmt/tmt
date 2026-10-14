import { pageAction } from '../test/page-actions.js';
import { text } from '../src/strings.js';
import { expect, test, type Page } from '@playwright/test';
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

// Remote never evicts an attached session (#2566): at the device cap a new page is refused and
// every open page stays live. The refused tab names the cap and the one command that raises it
// (#2625); closing an open tab frees a session for a reload.
test('a device at its session cap refuses a new page and keeps the open pages live', async () => {
  await withWorld(async (world) => {
    run(world, world.binaries.remote, ['settings', 'sessions-per-device', '2']);
    const door = await startDoor(world);
    await world.startAgent('ask-recipient');
    const created = createPage(world, 'Several tabs', '<h1>Several tabs</h1>');
    const browser = await pairBrowser(world, 'one-device');
    const heading = (tab: Page) =>
      tab.frameLocator('iframe').getByRole('heading', { name: 'Several tabs' });
    const tabs: Page[] = [];
    for (let index = 0; index < 2; index++) {
      const tab = await openPage(door, browser, created);
      await expect(heading(tab)).toBeVisible();
      tabs.push(tab);
    }
    const refused = await openPage(door, browser, created);
    await expect(refused.getByRole('alert')).toBeVisible({ timeout: 30_000 });
    await expect(refused.getByText(text.sessionLimitClose)).toBeVisible();
    await expect(refused.getByText(text.sessionLimitReached(2))).toBeVisible();
    await expect(refused.getByText('tmt remote settings sessions-per-device 3')).toBeVisible();
    await expect(refused.getByRole('link', { name: text.remoteSettings })).toBeVisible();
    await expect(refused.getByText(text.registrationFailed)).toHaveCount(0);
    await expect(heading(refused)).toHaveCount(0);
    for (const tab of tabs) await expect(heading(tab)).toBeVisible();
    await expect(tabs[0].getByRole('alert')).toHaveCount(0);
    await tabs[1].close();
    await refused.reload();
    await expect(heading(refused)).toBeVisible({ timeout: 30_000 });
    await expect(heading(tabs[0])).toBeVisible();
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

    await (await pageAction(a, 'Source')).click();
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
      const thread = tab.getByTestId('comment-thread');
      await expect(thread).toHaveAttribute('data-anchor', 'attached');
      await expect(thread.locator('.thread-state')).toHaveAttribute('title', quote);
      await expect(thread).toContainText(message);
    }
    await expect(a.getByRole('button', { name: /^Discussion \(/ })).toBeVisible();
    await (await pageAction(a, 'Comments')).click();
    await expect(a.getByTestId('annotation-row')).toContainText('Quote A.');
    await expect(a.getByTestId('annotation-row')).toHaveCount(1);
    if (!(await c.locator('.tmt-ui-drawer[data-panel=comments][open]').isVisible()))
      await (await pageAction(c, 'Comments')).click();
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
