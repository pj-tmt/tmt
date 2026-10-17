import { expect, test, type Page } from '@playwright/test';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, options = {}) {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review', options);
  await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute('data-detached');
}

test('one unanswered group counts once and menu navigation only scrolls and focuses', async ({
  page,
}) => {
  await mount(page, { sections: 2, group: true });
  const trigger = page.getByRole('button', { name: 'Needs you: 3', exact: true });
  await expect(trigger).toBeVisible();
  const before = await run(page, 'proof');
  const threads = await run(page, 'discussionProof');
  await trigger.click();
  const menu = page.getByRole('menu');
  await expect(menu.getByText('Please answer · 1')).toBeVisible();
  await expect(menu.getByText('Please confirm · 2')).toBeVisible();
  await expect(menu.getByText('Please read · 0')).toHaveCount(0);
  await menu.getByRole('menuitem', { name: 'Two questions · Agent 1' }).click();
  await expect(menu).toHaveCount(0);
  await expect(page.locator('[data-question-group] button').first()).toBeFocused();
  expect(await run(page, 'proof')).toEqual(before);
  expect(await run(page, 'discussionProof')).toEqual(threads);
  await trigger.press('Enter');
  await page.getByRole('menuitem', { name: 'Section 2 · Agent 1' }).click();
  await expect(
    page
      .locator('.review-card')
      .nth(1)
      .getByRole('button', { name: 'Confirm and close', exact: true }),
  ).toBeFocused();
  expect(await run(page, 'proof')).toEqual(before);
});

test('confirmed and waiting-for-agent sections leave the count; zero is a stable noninteractive state', async ({
  page,
}) => {
  await mount(page, { sections: 2 });
  const trigger = page.getByRole('button', { name: 'Needs you: 2', exact: true });
  await expect(trigger).toBeVisible();
  const before = await page.locator('#ask-page-fixture .needs-you-header').boundingBox();
  const first = page.locator('.review-card').first();
  await first.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(first).toHaveAttribute('data-review-state', 'closed');
  await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
  const second = page.locator('.review-card').nth(1);
  await second.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await second
    .getByLabel('What needs to change', { exact: true })
    .fill('Please explain the trade-off.');
  await second.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(second).toHaveAttribute('data-review-state', 'changes');
  const zero = page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true });
  await expect(zero).toBeVisible();
  await expect(zero.getByRole('button')).toHaveCount(0);
  expect((await zero.boundingBox())!.width).toBe(before!.width);
});

test('a chip opens on the next unanswered question without changing its retained draft', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'chipPage');
  const trigger = page.getByRole('button', { name: 'Needs you: 1', exact: true });
  await expect(trigger).toBeVisible();
  // Synthetic activation cannot open the menu.
  await trigger.evaluate((node) => (node as HTMLButtonElement).click());
  await expect(page.getByRole('menu')).toHaveCount(0);
  await trigger.press('Enter');
  await page.getByRole('menuitem', { name: 'Two list decisions · Agent 1' }).click();
  const panel = page.locator('[data-question-group]');
  await expect(panel).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Collapse', exact: true })).toBeFocused();
  await panel.getByLabel('One column', { exact: true }).check();
  await panel.getByRole('button', { name: /^Send/ }).click();
  await expect(panel.locator('.question-card:visible')).toContainText('Answered');
  await expect(trigger).toBeVisible();
  // Group Send includes every prepared answer: start the retained draft only after Q1 is stored.
  await panel.getByRole('tab', { name: /Q2/ }).click();
  await panel.getByLabel('Your answer', { exact: true }).fill('Keep this unsent draft.');
  await panel.getByRole('tab', { name: /Q1/ }).click();
  const before = await run(page, 'proof');
  const records = await run(page, 'discussionProof');
  await trigger.click();
  await page.getByRole('menuitem', { name: 'Two list decisions · Agent 1' }).click();
  await expect(panel.getByRole('tab', { name: /Q2/ })).toHaveAttribute('aria-selected', 'true');
  await expect(panel.getByLabel('Your answer', { exact: true })).toHaveValue(
    'Keep this unsent draft.',
  );
  await expect(panel.getByRole('button', { name: 'Collapse', exact: true })).toBeFocused();
  expect(await run(page, 'proof')).toEqual(before);
  expect(await run(page, 'discussionProof')).toEqual(records);
});

test('archived navigation retains the pending target without exposing a write action', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'archive');
  const trigger = page.getByRole('button', { name: 'Needs you: 1', exact: true });
  await expect(trigger).toBeVisible();
  const before = await run(page, 'proof');
  await trigger.click();
  await page.getByRole('menuitem', { name: 'Section 1 · Agent 1' }).click();
  const target = page.locator('[data-inline-review]');
  await expect(target).toBeFocused();
  await expect(target.getByRole('button')).toHaveCount(0);
  expect(await run(page, 'proof')).toEqual(before);
});

test('replacement markup withdraws vanished targets and Escape returns focus without publication', async ({
  page,
}) => {
  await mount(page);
  const trigger = page.getByRole('button', { name: 'Needs you: 1', exact: true });
  await trigger.press('ArrowDown');
  await expect(page.getByRole('menuitem', { name: 'Section 1 · Agent 1' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(trigger).toBeFocused();
  await run(page, 'change', '<h1>The old section was removed.</h1>');
  await expect(page.locator('[data-inline-review]')).toHaveCount(0);
  await expect(trigger).toHaveCount(0);
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
  expect((await run(page, 'proof')).actions).toEqual([]);
  expect(await run(page, 'discussionProof')).toHaveLength(0);
});
