import { expect, test, type Locator } from '@playwright/test';
import { mount, openChat, attach, send } from '../test/attachment-page.js';
import { pageAction } from '../test/page-actions.js';

// #4778: the Colab app's leftover square boxes and literal radii take the rounded tokens
// (--tmt-ui-radius-s 6px, --tmt-ui-radius 8px, --tmt-ui-radius-l 12px). A row of a ruled list stays square.
const radius = (locator: Locator) =>
  locator.evaluate((node) => {
    const style = getComputedStyle(node);
    return [
      style.borderTopLeftRadius,
      style.borderTopRightRadius,
      style.borderBottomRightRadius,
      style.borderBottomLeftRadius,
    ];
  });
const all = (px: string) => [px, px, px, px];

for (const width of [1440, 390])
  test(`attachments, the mention list and the viewer use the radius tokens at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    const { panel, input } = await openChat(page);
    await input.fill('Image with a message.');
    await attach(page, [
      {
        name: 'image-1.png',
        mimeType: 'image/png',
        buffer: Buffer.from(
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
          'base64',
        ),
      },
    ]);
    await send(page).click();
    const thumbnail = panel.locator('.attachment-thumbnail').first();
    await expect(thumbnail).toBeVisible();
    expect(await radius(thumbnail)).toEqual(all('6px'));
    await thumbnail.click();
    const viewer = page.locator('.attachment-viewer');
    await expect(viewer).toBeVisible();
    expect(await radius(viewer)).toEqual(all('12px'));
    await page.keyboard.press('Escape');
    await input.click();
    await input.pressSequentially('@');
    const list = page.locator('.tmt-listbox-input-list:visible').first();
    await expect(list).toBeVisible();
    expect(await radius(list)).toEqual(all('8px'));
    expect(await radius(list.locator('.tmt-listbox-option').first())).toEqual(all('6px'));
  });

test('the selection hint and its Esc key are rounded', async ({ page }) => {
  await mount(page);
  await page.getByRole('button', { name: 'Selection mode', exact: true }).click();
  expect(await radius(page.locator('.selection-mode-hint'))).toEqual(all('8px'));
  expect(await radius(page.locator('.selection-mode kbd'))).toEqual(all('6px'));
});

test('the New messages chip is rounded', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  const fixture = '/test/ask-page-browser.tsx';
  const run = (method: string, argument?: unknown) =>
    page.evaluate(
      async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
      {
        fixture,
        method,
        argument,
      },
    );
  await run('mount', { history: 'chat' });
  const host = page.locator('#ask-page-fixture');
  await expect(host.locator('.status')).toContainText('Live');
  await (await pageAction(host, 'Chat')).click();
  const window = page.getByTestId('chat-panel');
  const history = window.locator('.conversation-messages');
  await expect
    .poll(() => history.evaluate((node) => node.scrollHeight - node.clientHeight))
    .toBeGreaterThan(200);
  await history.evaluate((node) => {
    node.scrollTop = 100;
  });
  await history.evaluate(
    () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );
  await run('scrollArrival');
  await expect(window.getByRole('button', { name: 'New messages', exact: true })).toBeVisible();
  expect(await radius(window.locator('.conversation-arrivals'))).toEqual(all('8px'));
});

test('the group marker is a pill', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  const fixture = '/test/ask-page-browser.tsx';
  await page.evaluate(async (path) => (await import(path)).mount({ creator: true }), fixture);
  await page.evaluate(async (path) => (await import(path)).annotationLook(), fixture);
  const group = page
    .frameLocator('#ask-page-fixture iframe')
    .locator('.annotation-number[data-colab-group]')
    .first();
  await expect(group).toBeVisible();
  expect((await radius(group))[0]).toBe('999px');
});
