import { test, expect, type Page } from '@playwright/test';

async function mount(page: Page) {
  await page.clock.install({ time: new Date('2026-01-01T00:00:00Z') });
  await page.clock.pauseAt(new Date('2026-01-01T00:00:01Z'));
  await page.goto('/');
  await page.evaluate(async () => {
    const fixture = '/test/browser-notifications.tsx';
    (await import(fixture)).mount();
  });
}
const cards = (page: Page) => page.locator('.tmt-ui-notification');
const lines = (page: Page) => page.locator('.tmt-ui-notification-line');

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`manual popover preserves focus, top layer and plain text ${width} ${theme}`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    await page.evaluate((value) => {
      document.documentElement.dataset.theme = value;
    }, theme);
    const typing = page.getByRole('textbox', { name: 'Retained focus' });
    await typing.focus();
    // A fake source arrives independently of the user's focused control.
    await page
      .getByRole('button', { name: 'Admit one', exact: true })
      .evaluate((node: HTMLButtonElement) => node.click());
    await expect(cards(page)).toHaveCount(1);
    await expect(typing).toBeFocused();
    const layer = page.locator('.tmt-ui-notifications');
    expect(await layer.evaluate((node) => node.matches(':popover-open'))).toBe(true);
    expect(await layer.evaluate((node) => node.matches(':modal'))).toBe(false);
    await expect(cards(page).first()).toHaveCSS('border-width', '1px');
    await expect(cards(page).first()).toHaveCSS('border-radius', '0px');
    await expect(cards(page).first()).toHaveCSS('box-shadow', 'none');
    await expect(cards(page).first()).toHaveCSS('animation-name', 'none');
    await expect(cards(page).first()).toHaveCSS('transition-duration', '0s');
    await page.screenshot({ path: info.outputPath(`one-${width}-${theme}.png`) });
    await page.keyboard.press('Tab');
    await page.keyboard.press('Tab');
    await expect(
      page.getByRole('button', { name: 'Dismiss Chat reply', exact: true }),
    ).toBeFocused();
    await page
      .getByRole('button', { name: 'Admit five', exact: true })
      .evaluate((node: HTMLButtonElement) => node.click());
    await expect(cards(page)).toHaveCount(3);
    await typing.focus();
    await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
    await expect(page.getByText('2 more waiting', { exact: true })).toBeVisible();
    const quote = page.locator('.tmt-ui-notification-quote');
    await expect(quote).toHaveText(
      'The discussion stays attached to the quoted passage, including this complete long quotation for the accessible text.',
    );
    const box = await layer.boundingBox();
    const header = await page
      .getByTestId('notification-fixture')
      .locator('.tmt-ui-header')
      .boundingBox();
    expect(box!.y).toBeGreaterThan(header!.y + header!.height);
    expect(box!.x + box!.width).toBeLessThanOrEqual(width - (width === 390 ? 4 : 8));
    expect(
      await page.evaluate(
        ({ x, y }) => document.elementFromPoint(x, y)?.closest('.tmt-ui-notification') !== null,
        { x: box!.x + 20, y: box!.y + 20 },
      ),
    ).toBe(true);
    await page.screenshot({ path: info.outputPath(`three-overflow-${width}-${theme}.png`) });
    await cards(page).nth(1).getByRole('button', { name: 'Open Thread', exact: true }).focus();
    await page.screenshot({ path: info.outputPath(`focus-thread-${width}-${theme}.png`) });
    await page.keyboard.press('Shift+Escape');
    await expect(cards(page)).toHaveCount(3);
    await page.keyboard.press('Escape');
    await expect(lines(page)).not.toContainText([
      'I checked the proposed wording and added one example.',
    ]);
    await expect(page.getByTestId('outer-escapes')).toHaveText('1');
    // Only the modified Escape fell through to the host; the plain key was consumed.
    await expect(page.getByText('1 more waiting', { exact: true })).toBeVisible();
    await page
      .frameLocator('iframe')
      .getByRole('button', { name: 'Author button' })
      .click({ position: { x: 20, y: 100 } });
    await expect(page.frameLocator('iframe').locator('body')).toHaveAttribute('data-presses', '1');
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(cards(page).first()).toHaveCSS('animation-name', 'none');
    await page.getByRole('button', { name: 'Open drawer', exact: true }).click();
    await expect(cards(page)).toHaveCount(0);
    await page.screenshot({ path: info.outputPath(`suspended-${width}-${theme}.png`) });
    await page.getByRole('dialog', { name: 'Notification suspension' }).press('Escape');
    await expect(cards(page)).toHaveCount(3);
    await page.screenshot({ path: info.outputPath(`resumed-${width}-${theme}.png`) });
  });
}

for (const key of ['Escape', 'Enter']) {
  test(`focused ${key === 'Escape' ? 'Escape' : 'Dismiss'} restores the field focus origin`, async ({
    page,
  }) => {
    await mount(page);
    // This case puts the field directly before the surface in the Tab route.
    // The other scenarios retain the authored frame's normal keyboard/pointer behavior.
    await page.locator('iframe').evaluate((node: HTMLIFrameElement) => {
      node.tabIndex = -1;
    });
    const typing = page.getByRole('textbox', { name: 'Retained focus' });
    await typing.focus();
    await page
      .getByRole('button', { name: 'Admit one', exact: true })
      .evaluate((node: HTMLButtonElement) => node.click());
    await page.keyboard.press('Tab');
    await expect(
      page.getByRole('button', { name: 'Dismiss Chat reply', exact: true }),
    ).toBeFocused();
    await page.keyboard.press(key);
    await expect(cards(page)).toHaveCount(0);
    await expect(typing).toBeFocused();
    await expect(page.getByTestId('activations')).toBeEmpty();
    await expect(page.getByTestId('outer-escapes')).toHaveText('0');
  });
}

test('FIFO expiry, deduplication, trusted activation and page lifetime', async ({ page }) => {
  await mount(page);
  await page.getByRole('button', { name: 'Admit five', exact: true }).click();
  await page.mouse.move(1, 880);
  const status = page.getByRole('status').and(page.locator('.tmt-ui-notification-status'));
  await expect(status).toContainText('Chat: The revised layout');
  await expect(status).toContainText('Thread: I checked');
  await expect(status).toContainText('Answer: Your answer');
  await expect(status).toHaveAttribute('aria-live', 'polite');
  await page
    .getByRole('button', { name: 'Open Chat', exact: true })
    .evaluate((node: HTMLButtonElement) => node.click());
  await expect(page.getByTestId('activations')).toBeEmpty();
  await expect(cards(page)).toHaveCount(3);
  const reply = await lines(page).first().boundingBox();
  // The card's native action hit target covers the reply text, not only its label.
  await page.mouse.click(reply!.x + 2, reply!.y + 2);
  await expect(page.getByTestId('activations')).toHaveText('1:true');
  await expect(lines(page)).toHaveText([
    'I checked the proposed wording and added one example.',
    'Your answer has a follow-up question.',
    'The proposal review is complete.',
  ]);
  await page.mouse.move(1, 880);
  await page.clock.runFor(5999);
  await expect(cards(page)).toHaveCount(3);
  await page.clock.runFor(1);
  await expect(lines(page)).toHaveText(['The last reply is waiting in the queue.']);
  await expect(status).toHaveText('Chat: The last reply is waiting in the queue.\n');
  await page.clock.runFor(6000);
  await expect(cards(page)).toHaveCount(0);
  await page.getByRole('button', { name: 'Admit five', exact: true }).click();
  await expect(cards(page)).toHaveCount(0);
  await page.getByRole('button', { name: 'New page', exact: true }).click();
  await expect(cards(page)).toHaveCount(3);
  await page.getByRole('button', { name: 'Toggle notifications', exact: true }).click();
  await expect(page.locator('.tmt-ui-notifications')).toHaveCount(0);
  await expect(
    page.getByRole('status').and(page.locator('.tmt-ui-notification-status')),
  ).toHaveCount(0);
  await page.clock.runFor(30000);
  await page.getByRole('button', { name: 'Toggle notifications', exact: true }).click();
  await expect(cards(page)).toHaveCount(3);
});

test('hover and focus pause independently and preserve remaining time', async ({ page }) => {
  await mount(page);
  await page.getByRole('button', { name: 'Admit one', exact: true }).click();
  await page.mouse.move(1, 880);
  await page.clock.runFor(2000);
  await cards(page).first().hover();
  await page.getByRole('button', { name: 'Open Chat', exact: true }).focus();
  await page.clock.runFor(10000);
  await page.mouse.move(1, 880);
  await page.clock.runFor(10000);
  await expect(cards(page)).toHaveCount(1);
  await page.getByRole('textbox', { name: 'Retained focus' }).focus();
  await page.clock.runFor(3999);
  await expect(cards(page)).toHaveCount(1);
  await page.clock.runFor(1);
  await expect(cards(page)).toHaveCount(0);
});

test('nested modal suspension is silent and rechecks all five before showing three', async ({
  page,
}, info) => {
  await mount(page);
  await page.getByRole('button', { name: 'Open drawer', exact: true }).click();
  const drawer = page.getByRole('dialog', { name: 'Notification suspension' });
  await drawer.getByRole('button', { name: 'Admit five', exact: true }).click();
  await expect(cards(page)).toHaveCount(0);
  await expect(
    page.getByRole('status').and(page.locator('.tmt-ui-notification-status')),
  ).toBeEmpty();
  await page.clock.runFor(30000);
  await drawer.getByRole('button', { name: 'Withdraw one and four', exact: true }).click();
  await drawer.getByRole('button', { name: 'Drawer menu', exact: true }).click();
  await page.getByRole('menu', { name: 'Drawer menu' }).press('Escape');
  await expect(drawer).toBeVisible();
  await expect(cards(page)).toHaveCount(0);
  await expect(page.getByTestId('transitions')).toHaveText('true,false');
  await page.screenshot({ path: info.outputPath('modal-suspended.png') });
  await drawer.press('Escape');
  await expect(lines(page)).toHaveText([
    'I checked the proposed wording and added one example.',
    'Your answer has a follow-up question.',
    'The last reply is waiting in the queue.',
  ]);
  await expect(page.locator('.tmt-ui-notification-waiting')).toHaveCount(0);
  const status = page.getByRole('status').and(page.locator('.tmt-ui-notification-status'));
  await expect(status).toContainText('Thread: I checked');
  await expect(status).not.toContainText('revised layout');
  await expect(status).not.toContainText('proposal review');
  await page.screenshot({ path: info.outputPath('modal-resumed.png') });
});

test('existing entries survive Menu and document-hidden suspension without replay announcements', async ({
  page,
}) => {
  await mount(page);
  await page.getByRole('button', { name: 'Admit one', exact: true }).click();
  await page.mouse.move(1, 880);
  await page.clock.runFor(2000);
  await page.getByRole('button', { name: 'Header menu', exact: true }).click();
  await expect(cards(page)).toHaveCount(0);
  await expect(
    page.getByRole('status').and(page.locator('.tmt-ui-notification-status')),
  ).toBeEmpty();
  await page.clock.runFor(10000);
  await page.getByRole('menuitem', { name: 'One', exact: true }).click();
  await expect(cards(page)).toHaveCount(1);
  await expect(
    page.getByRole('status').and(page.locator('.tmt-ui-notification-status')),
  ).toBeEmpty();
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, value: true });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect(cards(page)).toHaveCount(0);
  await page.clock.runFor(10000);
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect(cards(page)).toHaveCount(1);
  await expect(
    page.getByRole('status').and(page.locator('.tmt-ui-notification-status')),
  ).toBeEmpty();
  await page.clock.runFor(3999);
  await expect(cards(page)).toHaveCount(1);
  await page.clock.runFor(1);
  await expect(cards(page)).toHaveCount(0);
});
