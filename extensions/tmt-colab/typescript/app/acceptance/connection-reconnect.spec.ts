import { expect, test } from '@playwright/test';
import { pairBrowser, restartRemote, startDoor } from './harness/browser.js';
import {
  clientId,
  composeChat,
  createPage,
  freePort,
  openChat,
  openPage,
  run,
  sendChat,
} from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

// #2572 (#2559 C1a): a Remote restart ends the open page's tunnel and Remote session while the
// device is still paired and serve is running. The page reconnects on its own: no Reconnect
// click, no reload, then it reads the next write and the composer reaches the agent.
test('an open owner page survives a Remote restart without a click or a reload', async () => {
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('reconnect-agent');
    const browser = await pairBrowser(world, 'reconnect-owner', { talk: true });
    const created = createPage(world, 'Remote restart', '<h1 id="h">Before</h1>', agent.pane);
    const page = await openPage(door, browser, created);
    await expect(page.locator('.status.live .status-label')).toHaveText('Live');
    await openChat(page);
    const navigations: string[] = [];
    page.on('framenavigated', (frame) => {
      if (frame === page.mainFrame()) navigations.push(frame.url());
    });

    // Remote dies and comes back on the same port, as after a tmt upgrade.
    await door.remote.kill();
    // While it is down the page says so quietly: no card, no button, the page stays.
    await expect(page.locator('.status')).toContainText('Reconnecting…', { timeout: 30_000 });
    await expect(page.getByRole('heading', { name: 'Connection lost', exact: true })).toHaveCount(
      0,
    );
    await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);
    await expect(page.frameLocator('iframe').locator('#h')).toHaveText('Before');
    await new Promise((resolve) => setTimeout(resolve, 3000));
    await restartRemote(world, door);

    await expect(page.locator('.status.live .status-label')).toHaveText('Live', {
      timeout: 90_000,
    });
    await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);

    // It reads: a write after the restart reaches the open tab.
    run(
      world,
      world.binaries.colab,
      ['page', 'write', created.pageId, '--file', '-', '--json'],
      '<h1 id="h">After</h1>',
    );
    await expect(page.frameLocator('iframe').locator('#h')).toHaveText('After', {
      timeout: 60_000,
    });

    // It talks: the composer reaches the agent with the exact text.
    const composed = await composeChat(page, agent, 'Back after the restart?');
    await sendChat(page, composed);
    await expect
      .poll(
        () => {
          try {
            return composed.delivered();
          } catch {
            return '';
          }
        },
        { timeout: 60_000 },
      )
      .toContain('Back after the restart?');
    expect(navigations).toEqual([]);
  });
});

// #2572 (#2559 C1b-2): a grant the owner revokes while the page is open is a refused fresh open,
// not a lost connection. The page does not retry forever: it ends with the access sentence and
// the one action that fits, pairing again.
test('a device revoked while its page is open ends with the access card and Pair again', async () => {
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'revoked-owner');
    const created = createPage(world, 'Revoked while open', '<h1 id="h">Revoked</h1>');
    const page = await openPage(door, browser, created);
    await expect(page.locator('.status.live .status-label')).toHaveText('Live');

    run(world, world.binaries.remote, [
      'devices',
      'revoke',
      clientId(world, 'revoked-owner'),
      '--json',
    ]);

    const card = page.locator('.tmt-ui-notice');
    await expect(card).toContainText('Your access to this page ended.', { timeout: 90_000 });
    await expect(card).toContainText('Pair again');
    await expect(card).toContainText('tmt remote pair');
    await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);
  });
});

// A modal drawer makes the page-level card inert, so an ended connection also speaks inside the
// open drawer: its composer owns the same sentence and one action (#2573).
test('a device revoked while its Chat drawer is open shows the ended notice in the drawer', async ({
  browserName,
}, testInfo) => {
  expect(browserName).toBe('chromium');
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'revoked-drawer');
    const created = createPage(world, 'Revoked with Chat open', '<h1 id="h">Revoked</h1>');
    const page = await openPage(door, browser, created);
    await expect(page.locator('.status.live .status-label')).toHaveText('Live');
    await openChat(page);
    const drawer = page.locator('.tmt-ui-drawer[data-panel=chat]');
    await expect(drawer).toBeVisible();
    await expect(drawer.locator('.drawer-recovery')).toHaveCount(0);

    run(world, world.binaries.remote, [
      'devices',
      'revoke',
      clientId(world, 'revoked-drawer'),
      '--json',
    ]);

    const notice = drawer.getByTestId('annotation-compose').getByRole('status');
    await expect(drawer.locator('.drawer-recovery')).toHaveCount(0);
    await expect(notice).toContainText('Your access to this page ended.', { timeout: 90_000 });
    await expect(notice).toContainText('Pair again');
    await expect(notice).toContainText('tmt remote pair');
    await expect(
      drawer.getByText('Your access to this page ended. Open it again to continue.', {
        exact: true,
      }),
    ).toHaveCount(1);
    await expect(notice.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);
    // Nothing can be published from an ended page: the composer is not editable and Send is off.
    await expect(
      drawer.getByRole('combobox', { name: 'Message', exact: true }),
    ).not.toHaveAttribute('contenteditable', 'true');
    await expect(drawer.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
    for (const [width, theme] of [
      [1440, 'light'],
      [390, 'dark'],
    ] as const) {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await expect(notice).toBeVisible();
      await page.screenshot({ path: testInfo.outputPath(`ended-drawer-${theme}-${width}.png`) });
    }
  });
});
