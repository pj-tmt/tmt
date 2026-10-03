import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
const fixture = '/test/ask-browser.tsx';
async function mount(page: Page, options: Record<string, unknown> = {}) {
  await page.evaluate(async ({ path, options }) => (await import(path)).mount(options), {
    path: fixture,
    options,
  });
  await expect(page.getByRole('region', { name: 'Ask agent — preview' })).toBeVisible();
}
async function proof(page: Page) {
  return page.evaluate(async (path) => (await import(path)).proof(), fixture);
}

test('trusted preview is inert, shows exact frozen/control bytes, and only an explicit click signs', async ({
  page,
}) => {
  await page.goto('/');
  await mount(page, { delivery: 'channel' });
  await expect(page.getByText('Channel ready', { exact: false })).toBeVisible();
  await expect(
    page.getByText('Your ask and the agent’s reply are visible', { exact: false }),
  ).toBeVisible();
  const initial = await proof(page);
  expect(initial.sends).toEqual([]);
  expect(initial.draft).toBeUndefined();
  expect(await page.getByLabel('Exact message').textContent()).toBe(initial.message);
  expect(initial.message).not.toContain('#secret');
  expect(await page.locator('#ask-fixture script').count()).toBe(0);
  await page.getByText('Show hidden characters').click();
  await expect(page.locator('#ask-fixture details pre')).toContainText(
    '\\u{d}\\u{a}😀\\u{0}\\u{202e}',
  );
  // Neither a page-message nor a programmatic click is the trusted user gesture.
  await page.evaluate(() => {
    window.postMessage({ type: 'dispatch', operationId: 'forged' }, '*');
    const send = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent === 'Send',
    )!;
    send.click();
  });
  expect((await proof(page)).draft).toBeUndefined();
  await page.evaluate(async (path) => (await import(path)).editLive(), fixture);
  expect(await page.getByLabel('Exact message').textContent()).toBe(initial.message);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Accepted; awaiting the agent’s reply.');
  const final = await proof(page);
  expect(final.sends).toEqual([
    {
      operationId: initial.operationId,
      agentId: '00000000-0000-4000-8000-000000000006',
      message: initial.message,
    },
  ]);
  expect(Object.keys(final.draft).sort()).toEqual(['input', 'signature']);
  expect(JSON.stringify(final.draft)).not.toContain(JSON.stringify(initial.message));
  expect(final.verified).toBe(true);
  expect(final.keyExportDenied).toBe(true);
  expect(final.reads).toEqual([]);
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/ask-preview-desktop.png',
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/ask-preview-mobile.png',
    fullPage: true,
  });
  await page.getByRole('button', { name: 'Close preview' }).click();
  await expect(page.getByRole('region', { name: 'Ask agent — preview' })).toHaveCount(0);
});

test('a persisted draft survives reload without dispatch; explicit adoption cannot resend it', async ({
  page,
}) => {
  await page.goto('/');
  await mount(page, { delivery: 'paste' });
  await expect(page.getByText('Paste only', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Accepted');
  const first = await proof(page);
  await page.reload();
  await mount(page, {
    operationId: first.operationId,
    issuedAt: first.issuedAt,
    delivery: 'paste',
  });
  expect((await proof(page)).sends).toEqual([]);
  expect((await proof(page)).draft).toEqual(first.draft);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Delivery is uncertain');
  expect((await proof(page)).sends).toEqual([]);
});

test('status is remote-supplied, hold is explicit, and absent runtime never signs', async ({
  page,
}) => {
  await page.goto('/');
  for (const [delivery, label] of [
    ['not_ready', 'Not ready'],
    ['not_running', 'Not running'],
    [undefined, 'Delivery status unavailable'],
  ] as const) {
    await mount(page, { delivery, unavailable: true });
    await expect(page.getByText(label, { exact: false })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
    expect((await proof(page)).draft).toBeUndefined();
  }
  await mount(page, { hold: true, mode: 'held' });
  await expect(page.getByText('This send waits for approval on your machine.')).toBeVisible();
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Waiting for approval on your machine.');
  expect((await proof(page)).sends).toHaveLength(1);
  await expect(page.getByRole('button', { name: 'Approve', exact: true })).toHaveCount(0);
  await mount(page, { mode: 'throw' });
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Do not resend');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
});

test('two tabs adopting one frozen operation have only one send winner', async ({
  page,
  context,
}) => {
  const other = await context.newPage();
  try {
    await Promise.all([page.goto('/'), other.goto('/')]);
    const options = { operationId: '00000000-0000-4000-8000-000000000777', issuedAt: Date.now() };
    await Promise.all([mount(page, options), mount(other, options)]);
    await Promise.all([
      page.getByRole('button', { name: 'Send', exact: true }).click(),
      other.getByRole('button', { name: 'Send', exact: true }).click(),
    ]);
    await Promise.all([
      expect(page.getByRole('status')).toContainText(/Accepted|uncertain/),
      expect(other.getByRole('status')).toContainText(/Accepted|uncertain/),
    ]);
    const [a, b] = await Promise.all([proof(page), proof(other)]);
    expect(a.sends.length + b.sends.length).toBe(1);
    expect([a.state, b.state].sort()).toEqual(['accepted', 'uncertain']);
    expect(a.draft).toEqual(b.draft);
  } finally {
    await other.close();
  }
});
