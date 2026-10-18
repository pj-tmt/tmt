import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';

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
  await expect(
    page.locator('.question-card:visible').getByLabel('Your answer', { exact: true }),
  ).toBeFocused();
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

test('the header opens the next unanswered question without changing its retained draft', async ({
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
  await expect(panel.locator('.question-compact-block')).toBeVisible();
  await expect(panel.locator('.question-card:visible').getByRole('radio').first()).toBeFocused();
  await panel.getByLabel('One column', { exact: true }).check();
  await panel.getByRole('button', { name: /^Send/ }).click();
  await expect(panel.locator('.question-line').first()).toContainText('Answered: One column');
  await expect(trigger).toBeVisible();
  // Group Send includes every prepared answer: start the retained draft only after Q1 is stored.
  await panel.locator('.question-line').nth(1).click();
  await panel.getByLabel('Your answer', { exact: true }).fill('Keep this unsent draft.');
  await panel.locator('.question-line').first().click();
  const before = await run(page, 'proof');
  const records = await run(page, 'discussionProof');
  await trigger.click();
  await page.getByRole('menuitem', { name: 'Two list decisions · Agent 1' }).click();
  await expect(panel.locator('.question-line').nth(1)).toHaveAttribute('aria-expanded', 'true');
  await expect(panel.getByLabel('Your answer', { exact: true })).toHaveValue(
    'Keep this unsent draft.',
  );
  await expect(
    panel.locator('.question-card:visible').getByLabel('Your answer', { exact: true }),
  ).toBeFocused();
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

const mixedSource = `<style>
body { margin:24px; font:16px/1.5 sans-serif; background:#fff; color:#222 }
html[data-theme="dark"] body { background:#181818; color:#eee }
tmt-read,tmt-review { display:block; margin:20px 0 }
</style><h1>Release decisions</h1>
<tmt-read data-id="a0000000-0000-4000-8000-000000500101" data-agent="Agent 1" data-why="Read the summary before choosing the layout."><h2 data-title>Read the summary</h2><p>Keep the page compact and make the next action clear. The new layout keeps the important material together.</p></tmt-read>
<div>The layout has one remaining decision: <tmt-questions data-id="a0000000-0000-4000-8000-000000500102" data-form="chip"><p data-title>Choose the layout</p><tmt-question data-id="a0000000-0000-4000-8000-000000500103" data-kind="short" data-agent="Agent 1"><p data-question>Which layout should ship?</p></tmt-question></tmt-questions> Details stay on the page.</div>
<tmt-review data-id="a0000000-0000-4000-8000-000000500104" data-agent="Agent 1"><h2 data-title>Confirm the scope</h2><p>The release includes these presentation changes. Confirm this scope after reading the summary.</p></tmt-review><p>The rest of the page remains available.</p>`;
async function capture(page: Page, name: string) {
  const directory = process.env.COLAB_NEEDS_YOU_CAPTURES;
  if (!directory) return;
  mkdirSync(directory, { recursive: true });
  await page.screenshot({ path: `${directory}/${name}.png` });
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`mixed pending actions, grouped menu and settled header ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 1100 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      await run(page, 'mount');
      await run(page, 'change', mixedSource);
      const root = page.locator('#ask-page-fixture');
      await (await pageAction(root, theme === 'dark' ? 'Theme: Dark' : 'Theme: Light')).click();
      const trigger = root.getByRole('button', { name: 'Needs you: 3', exact: true });
      await expect(trigger).toBeVisible();
      await expect(root.locator('[data-inline-reading]')).not.toHaveAttribute('data-detached');
      await expect(root.locator('[data-inline-review]')).not.toHaveAttribute('data-detached');
      await expect(root.locator('.question-line')).toBeVisible();
      const initialWidth = (await root.locator('.needs-you-header').boundingBox())!.width;
      await expect
        .poll(async () => (await trigger.innerText()).replace(/\s+/g, ''))
        .toBe(width === 390 ? '◆3▾' : '◆Needsyou3▾');
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const decoration = () =>
        trigger.evaluate((node) => getComputedStyle(node).textDecorationLine);
      await page.mouse.move(0, 0);
      expect(await decoration()).toBe('none');
      await trigger.hover();
      expect(await decoration()).toBe('underline');
      await page.mouse.move(0, 0);
      await trigger.press('Shift+Tab');
      await page.keyboard.press('Tab');
      await expect(trigger).toBeFocused();
      expect(await decoration()).toBe('underline');
      await trigger.evaluate((node) => (node as HTMLButtonElement).blur());
      expect(await decoration()).toBe('none');
      const colours = await trigger.evaluate((node) => ({
        mark: getComputedStyle(node.querySelector('.needs-you-header-mark')!).color,
        text: getComputedStyle(node).color,
      }));
      expect(colours.mark).not.toBe(colours.text);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await capture(page, `needs-you-3-${width}-${theme}`);
      const before = await run(page, 'proof');
      await trigger.click();
      const menu = page.getByRole('menu');
      await expect(menu.getByRole('group')).toHaveCount(3);
      expect(await menu.locator('.needs-you-header-group').allTextContents()).toEqual([
        'Please read · 1',
        'Please answer · 1',
        'Please confirm · 1',
      ]);
      await expect(menu.getByRole('menuitem')).toHaveCount(3);
      await capture(page, `needs-you-menu-${width}-${theme}`);
      await menu.getByRole('menuitem', { name: 'Read the summary · Agent 1' }).click();
      const markRead = root
        .locator('[data-inline-reading]')
        .getByRole('button', { name: 'Mark read', exact: true });
      await expect(markRead).toBeFocused();
      expect(await run(page, 'proof')).toEqual(before);
      await markRead.press('Enter');
      await expect(root.getByRole('button', { name: 'Needs you: 2', exact: true })).toBeVisible();
      await root.getByRole('button', { name: 'Needs you: 2', exact: true }).click();
      await page.getByRole('menuitem', { name: 'Choose the layout · Agent 1' }).click();
      const panel = root.locator('[data-question-group]');
      await expect(
        panel.locator('.question-card:visible').getByLabel('Your answer', { exact: true }),
      ).toBeFocused();
      await panel.getByLabel('Your answer', { exact: true }).fill('Use the compact layout.');
      await panel.getByRole('button', { name: /^Send/ }).click();
      await expect(root.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
      await expect(panel.locator('.question-line')).toHaveAttribute('aria-expanded', 'false');
      await root.getByRole('button', { name: 'Needs you: 1', exact: true }).click();
      await page.getByRole('menuitem', { name: 'Confirm the scope · Agent 1' }).click();
      const confirm = root
        .locator('[data-inline-review]')
        .getByRole('button', { name: 'Confirm and close', exact: true });
      await expect(confirm).toBeFocused();
      await confirm.press('Enter');
      const zero = root.getByLabel('✓ Nothing waits', { exact: true });
      await expect(zero).toBeVisible();
      await expect(zero.getByRole('button')).toHaveCount(0);
      expect((await zero.boundingBox())!.width).toBe(initialWidth);
      expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'status:resolve']);
      expect(await run(page, 'discussionProof')).toHaveLength(3);
      await capture(page, `needs-you-0-${width}-${theme}`);
    });
  }

test('the menu does not shift the authored page under the fixed header', async ({ page }) => {
  await mount(page);
  const padding = await page.locator('body').evaluate((node) => getComputedStyle(node).paddingTop);
  const top = (await page.locator('#ask-page-fixture iframe').boundingBox())!.y;
  await page.getByRole('button', { name: 'Needs you: 1', exact: true }).click();
  await expect(page.getByRole('menu')).toBeVisible();
  expect(await page.locator('body').evaluate((node) => getComputedStyle(node).paddingTop)).toBe(
    padding,
  );
  expect((await page.locator('#ask-page-fixture iframe').boundingBox())!.y).toBe(top);
});

for (const width of [390, 320])
  test(`long target and agent labels remain accessible in a contained one-line row ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await run(page, 'mount');
    const title = 'A long section heading that explains the pending decision in detail. '
      .repeat(2)
      .trim();
    const agent = 'Agent with a deliberately long visible name';
    await run(
      page,
      'change',
      `<h1>Long labels</h1><tmt-review data-id="a0000000-0000-4000-8000-000000500105" data-agent="${agent}"><h2 data-title>${title}</h2><p>The section remains on the page.</p></tmt-review>`,
    );
    const trigger = page.getByRole('button', { name: 'Needs you: 1', exact: true });
    await trigger.click();
    const row = page.getByRole('menuitem', { name: `${title} · ${agent}`, exact: true });
    await expect(row).toBeVisible();
    const shape = await row.locator('.needs-you-header-row').evaluate((node) => {
      const style = getComputedStyle(node),
        rect = node.getBoundingClientRect();
      return {
        ellipsis: style.textOverflow,
        lines: style.whiteSpace,
        clipped: node.scrollWidth > node.clientWidth,
        left: rect.left,
        right: rect.right,
      };
    });
    expect(shape.ellipsis).toBe('ellipsis');
    expect(shape.lines).toBe('nowrap');
    expect(shape.clipped).toBe(true);
    expect(shape.left).toBeGreaterThanOrEqual(0);
    expect(shape.right).toBeLessThanOrEqual(width);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.keyboard.press('Escape');
    await expect(trigger).toBeFocused();
    expect((await run(page, 'proof')).actions).toEqual([]);
  });

test('archived readings keep cosmetic focus without Mark read or a notification', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'change', mixedSource);
  const root = page.locator('#ask-page-fixture');
  await expect(root.getByRole('button', { name: 'Needs you: 3', exact: true })).toBeVisible();
  await run(page, 'archive');
  const before = await run(page, 'proof');
  await root.getByRole('button', { name: 'Needs you: 3', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Read the summary · Agent 1' }).click();
  const reading = root.locator('[data-inline-reading]');
  await expect(reading).toBeFocused();
  await expect(reading.getByRole('button')).toHaveCount(0);
  expect(await run(page, 'proof')).toEqual(before);
  expect(await run(page, 'discussionProof')).toHaveLength(0);
});

test('Needs-you entries are links and modified activation leaves this page unchanged', async ({
  page,
  context,
}) => {
  await mount(page, { sections: 2, group: true });
  const before = await run(page, 'proof');
  const threads = await run(page, 'discussionProof');
  await page.getByRole('button', { name: 'Needs you: 3', exact: true }).click();
  const menu = page.getByRole('menu');
  const link = menu.getByRole('menuitem', { name: 'Two questions · Agent 1' });
  await expect(link).toHaveJSProperty('tagName', 'A');
  const href = await link.getAttribute('href');
  expect(href).toContain('/pages/');
  const target = new URL(href!, page.url()).href;
  for (const options of [
    { modifiers: ['ControlOrMeta'] },
    { modifiers: ['Shift'] },
    { button: 'middle' as const },
  ] satisfies NonNullable<Parameters<typeof link.click>[0]>[]) {
    const opened = context.waitForEvent('page');
    await link.click(options);
    const destination = await opened;
    await expect(destination).toHaveURL(target);
    await expect(menu).toBeVisible();
    await destination.close();
    await page.bringToFront();
  }
  for (const key of ['Control+Enter', 'Meta+Enter', 'Shift+Enter']) {
    const opened = context.waitForEvent('page');
    await link.press(key);
    const destination = await opened;
    await expect(destination).toHaveURL(target);
    await expect(menu).toBeVisible();
    await destination.close();
    await page.bringToFront();
  }
  expect(await run(page, 'proof')).toEqual(before);
  expect(await run(page, 'discussionProof')).toEqual(threads);
  await link.press('Enter');
  await expect(menu).toHaveCount(0);
  await expect(
    page.locator('.question-card:visible').getByLabel('Your answer', { exact: true }),
  ).toBeFocused();
  expect(await run(page, 'proof')).toEqual(before);
});

test('separate placements count as one pending group and the menu names its remaining questions', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'questionGroup', { split: true });
  const trigger = page.getByRole('button', { name: 'Needs you: 1', exact: true });
  await expect(trigger).toBeVisible();
  await trigger.click();
  const item = page.getByRole('menuitem', { name: /2 of 2 questions waiting/ });
  await expect(item).toHaveCount(1);
  await item.click();
  const rows = page.locator('.question-line');
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'true');
  await page.getByLabel('Keep it', { exact: true }).check();
  await page.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }).click();
  await expect(rows.first()).toContainText('Answered: Keep it');
  await expect(trigger).toBeVisible();
  const records = await run(page, 'discussionProof');
  await trigger.click();
  await expect(page.getByRole('menuitem', { name: /1 of 2 questions waiting/ })).toHaveCount(1);
  await page.getByRole('menuitem', { name: /1 of 2 questions waiting/ }).click();
  await expect(rows.nth(1)).toHaveAttribute('aria-expanded', 'true');
  await expect(
    page.locator('.question-card:visible').getByLabel('Your answer', { exact: true }),
  ).toBeFocused();
  expect(await run(page, 'discussionProof')).toEqual(records);
});
