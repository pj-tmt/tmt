import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
const fixture = '/test/ask-again-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, surface: string, mode: string, reverse = false) {
  await page.goto('/');
  await run(page, 'mount', { surface, mode, reverse });
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  await expect(page.locator('.annotation-status-row')).not.toContainText('Checking');
  await input.fill('@alpha @beta Explain this page.');
  await input.press('Enter');
  await expect(page.getByTestId('comment-entry')).toHaveCount(1);
  await expect.poll(async () => (await run(page, 'proof')).preparations).toBe(2);
  await expect
    .poll(async () => (await run(page, 'proof')).sends.length)
    .toBe(['prepare', 'unadopted', 'unknown'].includes(mode) ? 1 : 2);
  return input;
}
for (const surface of ['chat', 'thread'])
  for (const mode of ['prepare', 'unadopted', 'refused'])
    test(`${surface} ${mode}: Ask again sends only the unsent recipient on the original comment`, async ({
      page,
    }) => {
      const input = await mount(page, surface, mode);
      const again = page.getByRole('button', { name: 'Ask again', exact: true });
      await expect(again).toHaveCount(1);
      const before = await run(page, 'proof');
      await input.fill('My next draft.');
      const caret = await input.evaluate(() => document.getSelection()?.anchorOffset);
      await again.evaluate((node) => (node as HTMLButtonElement).click());
      expect((await run(page, 'proof')).sends).toEqual(before.sends);
      await again.focus();
      await expect(again).toBeFocused();
      await expect(again).toHaveAccessibleName('Ask again');
      await expect(again.locator('svg.lucide-send')).toBeVisible();
      await again.press('Enter');
      await expect
        .poll(async () => (await run(page, 'proof')).sends.length)
        .toBe(before.sends.length + 1);
      await expect(again).toHaveCount(0);
      const after = await run(page, 'proof');
      expect(after.writes).toBe(1);
      expect(after.thread.comments).toEqual(before.thread.comments);
      expect(after.records.slice(0, before.records.length)).toEqual(before.records);
      expect(after.sends.at(-1).agentId).toBe(before.inputs[0].destination.agent);
      expect(
        after.sends.filter(
          (value: { agentId: string }) => value.agentId === before.inputs[1].destination.agent,
        ),
      ).toHaveLength(1);
      expect(after.inputs.at(-1).context.message).toEqual(before.inputs[0].context.message);
      expect(after.inputs.at(-1).context.thread).toEqual(before.inputs[0].context.thread);
      expect(after.inputs.at(-1).retryOf).toBe(
        mode === 'refused' ? before.records[0].operationId : null,
      );
      expect(
        new Set(after.sends.map((value: { operationId: string }) => value.operationId)).size,
      ).toBe(after.sends.length);
      await expect(page.getByTestId('comment-entry')).toHaveCount(1);
      await expect(input).toHaveText('My next draft.', { useInnerText: true });
      await input.focus();
      expect(await input.evaluate(() => document.getSelection()?.anchorOffset)).toBe(caret);
      await expect(page.getByTestId('ask-state').last()).toContainText('Waiting for alpha');
    });

test('two trusted activations while preparation waits produce one fresh operation', async ({
  page,
}) => {
  await mount(page, 'chat', 'refused');
  await run(page, 'setPark', true);
  const again = page.getByRole('button', { name: 'Ask again', exact: true });
  const box = (await again.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, { clickCount: 2 });
  await expect(again).toBeDisabled();
  await expect(again).toHaveAttribute('aria-busy', 'true');
  await expect.poll(async () => (await run(page, 'proof')).preparations).toBe(3);
  await expect.poll(() => run(page, 'isParked')).toBe(true);
  await run(page, 'releasePreparation');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(3);
  expect((await run(page, 'proof')).writes).toBe(1);
});

test('another refusal keeps both honest outcomes and one recipient action; keyboard can ask again explicitly', async ({
  page,
}) => {
  await mount(page, 'thread', 'refused-again');
  const again = page.getByRole('button', { name: 'Ask again', exact: true });
  await again.focus();
  await again.press('Enter');
  await expect(page.getByTestId('ask-entry')).toHaveCount(3);
  await expect(page.locator('[data-ledger-state=refused]')).toHaveCount(2);
  await expect(again).toHaveCount(1);
  await expect(page.getByTestId('ask-state').last()).toContainText('Not delivered');
  await expect(page.locator('.ask-supporting').last()).toContainText(
    'Remote is busy. Try again later.',
  );
  expect((await run(page, 'proof')).writes).toBe(1);
});

for (const mode of ['uncertain', 'unknown', 'held'])
  test(`${mode}: Ask again is never offered`, async ({ page }) => {
    await mount(page, 'chat', mode);
    await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
    const proof = await run(page, 'proof');
    expect(proof.writes).toBe(1);
    if (mode === 'uncertain')
      await expect(page.getByRole('button', { name: 'Abandon tracking', exact: true })).toHaveCount(
        1,
      );
    if (mode === 'unknown')
      await expect(page.getByTestId('recipient-failure')).toContainText('Delivery unconfirmed');
  });

for (const mode of ['prepare', 'unadopted', 'refused'])
  test(`${mode}: reload restores only an admitted refusal action and never sends`, async ({
    page,
  }) => {
    await mount(page, 'chat', mode);
    await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(1);
    const before = await run(page, 'proof');
    await page.reload();
    await run(page, 'mount', { surface: 'chat', mode, restore: true });
    await expect(page.getByTestId('comment-entry')).toHaveCount(1);
    await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(
      mode === 'refused' ? 1 : 0,
    );
    expect((await run(page, 'proof')).sends).toEqual(before.sends);
  });

test('a directory failure or renamed-name UUID cannot select a replacement recipient', async ({
  page,
}) => {
  await mount(page, 'chat', 'refused');
  const again = page.getByRole('button', { name: 'Ask again', exact: true });
  for (const directory of ['fail', 'renamed']) {
    await run(page, 'setDirectory', directory);
    await again.click();
    await expect(page.getByRole('alert')).toBeVisible();
    expect((await run(page, 'proof')).sends).toHaveLength(2);
    expect((await run(page, 'proof')).preparations).toBe(2);
  }
  await run(page, 'setDirectory', 'ready');
  await again.click();
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(3);
});

test('a replaced binding cannot dispatch a late prepared retry', async ({ page }) => {
  await mount(page, 'chat', 'refused');
  await run(page, 'setPark', true);
  await page.getByRole('button', { name: 'Ask again', exact: true }).click();
  await expect.poll(async () => (await run(page, 'proof')).preparations).toBe(3);
  await run(page, 'replaceBinding');
  await expect.poll(() => run(page, 'isParked')).toBe(true);
  await run(page, 'releasePreparation');
  await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toBeEnabled();
  expect((await run(page, 'proof')).sends).toHaveLength(2);
});

test('another device sees the old refusal but gets no Ask again capability', async ({ page }) => {
  await mount(page, 'chat', 'refused');
  await run(page, 'mount', { surface: 'chat', mode: 'refused', restore: true, readonly: true });
  await expect(page.locator('[data-ledger-state=refused]')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
});

test('an accepted replacement blocks the older refusal after reload without another Send', async ({
  page,
}) => {
  await mount(page, 'chat', 'refused');
  await page.getByRole('button', { name: 'Ask again', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
  const before = await run(page, 'proof');
  await page.reload();
  await run(page, 'mount', { surface: 'chat', mode: 'refused', restore: true });
  await expect(page.locator('[data-ledger-state=refused]')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual(before.sends);
  expect((await run(page, 'proof')).writes).toBe(1);
});

test('two same-origin tabs with stale own projections share one durable pair fence', async ({
  page,
  context,
}) => {
  await mount(page, 'chat', 'refused');
  const other = await context.newPage();
  await other.goto('/');
  await run(other, 'mount', { surface: 'chat', mode: 'refused', restore: true });
  await expect(other.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(1);
  await run(page, 'setPark', true);
  await run(other, 'setPark', true);
  await page.getByRole('button', { name: 'Ask again', exact: true }).click();
  await other.getByRole('button', { name: 'Ask again', exact: true }).click();
  // Both controllers captured the same old own view before either adoption.
  await expect.poll(async () => (await run(page, 'proof')).preparations).toBe(3);
  await expect.poll(async () => (await run(other, 'proof')).preparations).toBe(3);
  await expect.poll(() => run(page, 'isParked')).toBe(true);
  await expect.poll(() => run(other, 'isParked')).toBe(true);
  await run(page, 'releasePreparation');
  await run(other, 'releasePreparation');
  await expect(page.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
  await expect(other.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
  const one = await run(page, 'proof');
  const two = await run(other, 'proof');
  expect(one.sends.length + two.sends.length - 4).toBe(1);
  expect(one.writes).toBe(1);
  expect(two.writes).toBe(1);
  expect(one.thread.comments).toEqual(two.thread.comments);
});

test('a newly refused replacement arriving before older rows leaves one enabled recipient action', async ({
  page,
}) => {
  await mount(page, 'thread', 'refused-again', true);
  const again = page.getByRole('button', { name: 'Ask again', exact: true });
  await again.click();
  await expect(page.locator('[data-ledger-state=refused]')).toHaveCount(2);
  await expect(again).toHaveCount(1);
  await expect(again).toBeEnabled();
  await again.click();
  await expect(page.locator('[data-ledger-state=refused]')).toHaveCount(3);
  await expect(again).toHaveCount(1);
  await expect(again).toBeEnabled();
  const proof = await run(page, 'proof');
  expect(proof.writes).toBe(1);
  expect(proof.sends).toHaveLength(4);
  expect(new Set(proof.sends.map((value: { operationId: string }) => value.operationId)).size).toBe(
    4,
  );
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`recipient actions ${width} ${theme}: distinct retry and observation controls fit`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page, 'chat', 'refused');
      const again = page.getByRole('button', { name: 'Ask again', exact: true });
      const check = page.getByRole('button', { name: 'Check again', exact: true });
      await expect(again).toBeVisible();
      await expect(check).toBeVisible();
      await expect(again.locator('svg.lucide-send')).toBeVisible();
      await expect(check.locator('svg.lucide-rotate-cw')).toBeVisible();
      const boxes = await Promise.all([again.boundingBox(), check.boundingBox()]);
      for (const box of boxes) {
        expect(box).not.toBeNull();
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
        if (width === 390) {
          expect(box!.width).toBeGreaterThanOrEqual(44);
          expect(box!.height).toBeGreaterThanOrEqual(44);
        }
      }
      const [a, b] = boxes;
      expect(
        a!.x + a!.width <= b!.x ||
          b!.x + b!.width <= a!.x ||
          a!.y + a!.height <= b!.y ||
          b!.y + b!.height <= a!.y,
      ).toBe(true);
      const before = await run(page, 'proof');
      for (const [button, label] of [
        [again, 'Ask again'],
        [check, 'Check again'],
      ] as const) {
        await button.hover();
        await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveText(label);
        await page.mouse.move(width - 1, 899);
        await button.focus();
        await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveText(label);
        await button.press('Escape');
        await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
      }
      await check.press('Enter');
      expect((await run(page, 'proof')).sends).toEqual(before.sends);
      expect((await run(page, 'proof')).writes).toBe(before.writes);
      await page.getByRole('combobox', { name: 'Message', exact: true }).focus();
      const captures = process.env.COLAB_AGAIN_CAPTURE_DIR;
      if (captures) {
        mkdirSync(captures, { recursive: true });
        await page.mouse.move(width - 1, 899);
        await page.screenshot({ path: `${captures}/actions-${width}-${theme}.png` });
      }
    });
