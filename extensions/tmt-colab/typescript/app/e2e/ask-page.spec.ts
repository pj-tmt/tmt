import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: string) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page) {
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('#selected')
    .evaluate((node) => {
      const range = document.createRange();
      range.selectNodeContents(node);
      const selected = node.ownerDocument.getSelection()!;
      selected.removeAllRanges();
      selected.addRange(range);
    });
  await expect(page.getByRole('button', { name: 'Ask agent', exact: true })).toBeEnabled();
}
async function compose(page: Page) {
  await page.getByRole('button', { name: 'Ask agent', exact: true }).click();
  await expect(page.getByRole('radio')).toHaveCount(5);
  await page.getByRole('radio').first().check();
  await page.getByLabel('Question or instruction').fill('Explain exactly');
}

test('live Page selection opens a parent picker, freezes preview through sync and requires trusted Send', async ({
  page,
}) => {
  await mount(page);
  expect((await run(page, 'proof')).sends).toEqual([]);
  await compose(page);
  await expect(
    page.locator('.ask-compose').getByText('Delivery status unavailable', { exact: false }),
  ).toHaveCount(0);
  await expect(
    page.locator('.ask-compose').getByText('My machine · Online', { exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
  const message = await page.getByLabel('Exact message').textContent();
  expect(message).toContain('Exact selected text');
  expect(message).toContain('Explain exactly');
  await run(page, 'change', '<p id="selected">Changed selected text</p>');
  await expect(page.getByRole('heading', { name: 'Changed live title' })).toBeVisible();
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
  expect(await page.getByLabel('Exact message').textContent()).toBe(message);
  await page.evaluate(() =>
    [...document.querySelectorAll<HTMLButtonElement>('button')]
      .find((b) => b.textContent === 'Send')!
      .click(),
  );
  expect((await run(page, 'proof')).sends).toEqual([]);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.ask-preview [role=status]')).toContainText('Accepted');
  expect((await run(page, 'proof')).sends).toHaveLength(1);
  await expect(page.getByText('No asks on this page yet.')).toBeVisible();
  await run(page, 'syncRecords');
  await expect(
    page.getByRole('region', { name: 'Page asks' }).getByText('<script>inert ask</script>'),
  ).toBeVisible();
  await expect(page.locator('.ask-panel script, .ask-panel img')).toHaveCount(0);
  await expect(page.getByText('The agent returned an empty reply.')).toBeVisible();
  await expect(page.getByTestId('ask-reply-attribution')).toContainText('Reply from Agent 2');
  await expect(page.getByTestId('ask-reply-attribution')).toContainText('Alex’s browser');
  await expect(page.getByTestId('ask-reply-attribution')).toContainText('5 minutes ago');
  await expect(page.getByTestId('ask-state').nth(1)).toHaveText('Accepted by your machine.');
  await page.getByRole('button', { name: 'Re-check delivery' }).click();
  expect((await run(page, 'proof')).actions).toEqual([
    'recheck:00000000-0000-4000-8000-000000000021',
  ]);
  expect((await run(page, 'proof')).sends).toHaveLength(1);
  await page.getByRole('button', { name: 'Abandon tracking' }).click();
  await run(page, 'syncRecords', 'abandoned');
  await expect(
    page.getByText('Tracking abandoned. The work may still have been delivered.'),
  ).toBeVisible();
  await page.locator('#ask-page-fixture iframe').scrollIntoViewIfNeeded();
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/ask-page-light.png',
    fullPage: true,
  });
  await page.getByRole('button', { name: 'Change color theme' }).click();
  await page.locator('#ask-page-fixture iframe').scrollIntoViewIfNeeded();
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('body')
    .evaluate(
      (body) =>
        new Promise<void>((resolve) => {
          const view = body.ownerDocument.defaultView!;
          view.requestAnimationFrame(() => view.requestAnimationFrame(() => resolve()));
        }),
    );
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/ask-page-dark.png',
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/ask-page-mobile.png',
    fullPage: true,
  });
});

test('closed preparation never reopens preview, and lost page connection disables Send', async ({
  page,
}) => {
  await mount(page);
  await compose(page);
  await run(page, 'pausePrepare');
  await page.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
  await page.getByRole('button', { name: 'Close preview' }).click();
  await run(page, 'resumePrepare');
  await expect(page.getByRole('region', { name: 'Ask agent — preview' })).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
  await compose(page);
  await page.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
  await run(page, 'block');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  await expect(page.locator('.ask-preview')).toContainText('offline');
  expect((await run(page, 'proof')).sends).toEqual([]);
});

test('expired preview visibly refuses Send and does not dispatch', async ({ page }) => {
  await page.clock.install();
  await mount(page);
  await compose(page);
  await page.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
  await page.clock.fastForward(60 * 60 * 1000 + 1);
  await expect(page.locator('.ask-preview')).toContainText('This preview has expired');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  expect((await run(page, 'proof')).sends).toEqual([]);
});

test('verified refusal reasons use actionable copy without exposing a resend', async ({ page }) => {
  await mount(page);
  for (const [reason, copy] of [
    ['REMOTE_SCOPE_DENIED', 'Your Remote permission does not allow this ask.'],
    ['REMOTE_INPUT_INVALID', 'Remote rejected this message. Create a new preview.'],
    ['REMOTE_RATE_LIMITED', 'Remote is busy. Review a fresh preview later.'],
    [
      'REMOTE_INTENT_CONFLICT',
      'This operation already has a different message. Create a new preview.',
    ],
    ['REMOTE_CLOSED', 'Remote is closed. Reconnect and create a new preview.'],
    [
      'REMOTE_SESSION_ENDED',
      'Your Remote session ended. Create a fresh preview after the page reconnects.',
    ],
  ]) {
    await run(page, 'syncRefusal', reason);
    await expect(page.getByTestId('ask-state').first()).toHaveText(copy);
    await expect(page.getByRole('button', { name: 'Abandon tracking' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Re-check delivery' })).toHaveCount(0);
  }
  expect((await run(page, 'proof')).sends).toEqual([]);
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/ask-refused-session.png',
    fullPage: true,
  });
});

test('navigation drops a pending preview without dispatch', async ({ page }) => {
  await mount(page);
  await compose(page);
  await run(page, 'pausePrepare');
  await page.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
  await page.getByRole('link', { name: 'Space home', exact: true }).click();
  await run(page, 'resumePrepare');
  await expect(page.locator('#ask-page-fixture .page')).toHaveCount(0);
  await expect(page.getByTestId('ask-preview')).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
});

test('read refusals show ephemeral copy without changing the admitted operation or dispatching', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'syncRecords');
  for (const [code, copy] of [
    ['REMOTE_INPUT_TOO_LARGE', 'Remote rejected the message size. Create a shorter preview.'],
    ['REMOTE_STATE_UNAVAILABLE', 'Remote cannot read this operation yet. Re-check delivery later.'],
    ['REMOTE_CORE_UNAVAILABLE', 'The agent service is unavailable. Re-check delivery later.'],
  ]) {
    await run(page, 'refuseRead', code);
    await page.getByRole('button', { name: 'Re-check delivery' }).click();
    await expect(page.locator('.ask-panel [role=alert]')).toHaveText(copy);
    await expect(page.getByTestId('ask-state').first()).toHaveAttribute('data-state', 'uncertain');
    expect((await run(page, 'proof')).sends).toEqual([]);
  }
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/ask-read-refused.png',
    fullPage: true,
  });
});
