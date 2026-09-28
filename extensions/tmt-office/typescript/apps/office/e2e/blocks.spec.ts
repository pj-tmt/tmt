import { expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import { test, signIn } from './browser-session.js';
import {
  setTester,
  readBlockDocument,
  TRANSACTION_FAILURE_TIMEOUT_MS,
} from './firestore-fixture.js';

async function prepareStudio(page: Page) {
  await signIn(page, 'Decorator');
  const uid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
  await setTester(uid, true);
  await page.getByLabel('World name').fill('The quiet studio');
  await page.getByRole('button', { name: 'Create world', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'The quiet studio' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add desk', exact: true })).toBeVisible();
  const worldId = new URL(page.url()).pathname.split('/').at(-1)!;
  return { uid, worldId };
}

test('owner arranges, explicitly saves, reopens and loses block access on revocation', async ({
  openSession,
}) => {
  const { page } = await openSession();
  const { uid, worldId } = await prepareStudio(page);
  await page.getByRole('button', { name: 'Add rug', exact: true }).click();
  await page.getByLabel('Tile X', { exact: true }).fill('12');
  await page.getByLabel('Tile Y', { exact: true }).fill('12');
  await page.getByRole('button', { name: 'Add desk', exact: true }).click();
  await page.getByRole('button', { name: 'Rotate clockwise' }).click();
  await page.getByRole('button', { name: 'Add chair', exact: true }).click();
  await page.getByLabel('Tile X', { exact: true }).fill('17');
  await page.getByRole('button', { name: 'Add plant', exact: true }).click();
  await page.getByLabel('Tile X', { exact: true }).fill('21');
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible();
  expect(await readBlockDocument(worldId)).toBeNull();
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect(page.getByText('Saved · revision 1', { exact: true })).toBeVisible();
  expect(await readBlockDocument(worldId)).toMatchObject({
    fields: {
      revision: { integerValue: '1' },
      objects: {
        arrayValue: {
          values: [
            { stringValue: 'r0cc' },
            { stringValue: 'd1ee' },
            { stringValue: 'c0he' },
            { stringValue: 'p0le' },
          ],
        },
      },
    },
  });
  await page.getByRole('link', { name: 'Office', exact: true }).click();
  await page.getByLabel('World ID', { exact: true }).fill(worldId);
  await page.getByRole('button', { name: 'Open world', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Desk 2', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Desk 2', exact: true }).click();
  await expect(page.getByLabel('Tile X', { exact: true })).toHaveValue('14');
  await page.getByLabel('Tile X', { exact: true }).focus();
  await page.keyboard.press('ArrowUp');
  await expect(page.getByLabel('Tile X', { exact: true })).toHaveValue('15');
  const destination = await page.locator('.block-scene').evaluate((element) => {
    const matrix = (element as SVGSVGElement).getScreenCTM()!;
    const point = new DOMPoint(10.5, 10.5).matrixTransform(matrix);
    return { x: point.x, y: point.y };
  });
  await page.mouse.click(destination.x, destination.y);
  await expect(page.getByLabel('Tile X', { exact: true })).toHaveValue('10');
  await expect(page.getByLabel('Tile Y', { exact: true })).toHaveValue('10');
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect(page.getByText('Saved · revision 2', { exact: true })).toBeVisible();
  await page
    .locator('.block-editor')
    .screenshot({ path: test.info().outputPath('block-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await page
    .locator('.block-editor')
    .screenshot({ path: test.info().outputPath('block-mobile.png') });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true
  );
  await page.getByRole('button', { name: 'Remove selected', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Desk 2', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect(page.getByText('Saved · revision 3', { exact: true })).toBeVisible();
  await setTester(uid, false);
  await expect(page.getByRole('heading', { name: 'Waiting for access' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Office block editor' })).toHaveCount(0);
});

test('save confirmation uses the available request channel while watches reconnect', async ({
  openSession,
}) => {
  const { page, context } = await openSession();
  const { worldId } = await prepareStudio(page);
  await page.getByRole('button', { name: 'Add desk', exact: true }).click();
  let blockedWatches = 0;
  // GAPI probes connectivity after an offline event; block the optional probe.
  await context.route('https://www.google.com/images/cleardot.gif*', (route) => route.abort());
  await context.setOffline(true);
  await context.route('http://127.0.0.1:8080/**', (route) => {
    if (route.request().url().includes('/Listen/')) {
      blockedWatches++;
      return route.abort('internetdisconnected');
    }
    return route.continue();
  });
  await context.setOffline(false);
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect
    .poll(() => readBlockDocument(worldId))
    .toMatchObject({
      fields: { revision: { integerValue: '1' } },
    });
  expect(blockedWatches).toBeGreaterThan(0);
  await expect(page.getByText('Saved · revision 1', { exact: true })).toBeVisible();
});

test('a disconnected save keeps a local draft and reconnect cannot silently publish it', async ({
  openSession,
}) => {
  const { page, context } = await openSession();
  const { worldId } = await prepareStudio(page);
  await page.getByRole('button', { name: 'Add desk', exact: true }).click();
  const transport = 'http://127.0.0.1:8080/**';
  let abortedReads = 0;
  await context.route(transport, (route) => {
    if (route.request().url().includes(':batchGet')) abortedReads++;
    return route.abort('internetdisconnected');
  });
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Save could not be confirmed', {
    timeout: TRANSACTION_FAILURE_TIMEOUT_MS,
  });
  expect(abortedReads).toBeGreaterThan(0);
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible();
  expect(await readBlockDocument(worldId)).toBeNull();
  await context.unroute(transport);
  expect(await readBlockDocument(worldId)).toBeNull();
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect(page.getByText('Saved · revision 1', { exact: true })).toBeVisible();
  expect(await readBlockDocument(worldId)).toMatchObject({
    fields: {
      revision: { integerValue: '1' },
      objects: { arrayValue: { values: [{ stringValue: 'd0ee' }] } },
    },
  });
});
