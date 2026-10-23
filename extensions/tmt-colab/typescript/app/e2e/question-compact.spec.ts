import { expect, test, type Page } from '@playwright/test';
import { rgb, tokens } from './tokens.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, options = {}, session = {}) {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
  await page.goto('/');
  await run(page, 'mount', session);
  await run(page, 'questionGroup', options);
  const rows = page.locator('.question-line');
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toBeVisible();
  await expect(rows.nth(1)).toBeVisible();
  return rows;
}

test('compact wrapper rows start closed, preserve drafts and keep one explicit batched Send', async ({
  page,
}) => {
  const rows = await mount(page);
  await expect(page.getByRole('tab')).toHaveCount(0);
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  await expect(rows.nth(1)).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 2 of 2 waiting',
  );
  await rows.first().click();
  await page.getByLabel('Change it', { exact: true }).check();
  await rows.nth(1).click();
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  await page.getByLabel('Your answer', { exact: true }).fill('Keep this introduction draft.');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  const send = page.getByRole('button', { name: 'Send 2 answers · to @Agent 1', exact: true });
  await send.click();
  await expect(rows.first()).toContainText('Answered: Change it');
  await expect(rows.nth(1)).toContainText('Answered: Keep this introduction draft.');
  await expect(rows.nth(1)).toHaveAttribute('aria-expanded', 'false');
  const threads = await run(page, 'discussionProof');
  expect(threads).toHaveLength(2);
  expect(threads.every((thread: { comments: unknown[] }) => thread.comments.length === 1)).toBe(
    true,
  );
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  const attempts = await run(page, 'answerBatchProof');
  expect(attempts).toHaveLength(1);
  expect(attempts[0].selection.messageIds).toHaveLength(2);
});

test('split Send publishes only its own question and keeps another passage draft', async ({
  page,
}) => {
  const rows = await mount(page, { split: true });
  await expect(page.locator('.question-compact-heading')).toHaveCount(0);
  await rows.first().click();
  await page.getByLabel('Change it', { exact: true }).check();
  await rows.nth(1).click();
  await page.getByLabel('Your answer', { exact: true }).fill('Only this passage is sent.');
  await page.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }).click();
  await expect(rows.nth(1)).toContainText('Answered: Only this passage is sent.');
  await expect(rows.first()).toContainText('Needs you');
  const threads = await run(page, 'discussionProof');
  expect(threads).toHaveLength(1);
  expect(threads[0].anchor.exact).toBe('What should the introduction say?');
  expect(threads[0].comments[0].body).not.toContain('Change it');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await rows.first().click();
  await expect(page.getByLabel('Change it', { exact: true })).toBeChecked();
  expect(await run(page, 'discussionProof')).toHaveLength(1);
});

test('row keyboard navigation, opening and Escape are cosmetic and preserve the draft', async ({
  page,
}) => {
  const rows = await mount(page);
  await rows.first().focus();
  await rows.first().press('ArrowDown');
  await expect(rows.nth(1)).toBeFocused();
  await rows.nth(1).press('Space');
  const input = page.getByLabel('Your answer', { exact: true });
  await expect(input).toBeFocused();
  await input.fill('Still a local draft.');
  await input.press('Escape');
  await expect(rows.nth(1)).toBeFocused();
  await expect(rows.nth(1)).toHaveAttribute('aria-expanded', 'false');
  await rows.nth(1).press('Enter');
  await expect(input).toHaveValue('Still a local draft.');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('split question recipients stay local to their own block', async ({ page }) => {
  const rows = await mount(page, { split: true, agents: ['Agent 1', 'Agent 2'] });
  await rows.first().click();
  await expect(page.getByRole('button', { name: 'Remove @Agent 1', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Remove @Agent 2', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Remove @Agent 1', exact: true }).click();
  await page.getByLabel('Keep it', { exact: true }).check();
  await rows.nth(1).click();
  await expect(page.getByRole('button', { name: 'Remove @Agent 2', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Remove @Agent 1', exact: true })).toHaveCount(0);
  await page.getByLabel('Your answer', { exact: true }).fill('Ask only Agent 2.');
  await page.getByRole('button', { name: 'Send answer · to @Agent 2', exact: true }).click();
  await expect(rows.nth(1)).toContainText('Answered: Ask only Agent 2.');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  const attempts = await run(page, 'answerBatchProof');
  expect(attempts[0].selection.comment).toContain('@Agent 2');
  expect(attempts[0].selection.comment).not.toContain('@Agent 1');
  await rows.first().click();
  await expect(page.getByLabel('Keep it', { exact: true })).toBeChecked();
  await expect(page.getByRole('button', { name: 'Remove @Agent 1', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Send answer · to thread only', exact: true }).click();
  await expect(rows.first()).toContainText('Answered: Keep it');
  expect(await run(page, 'discussionProof')).toHaveLength(2);
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

test('a split question cannot bypass the recipient read by pressing Enter', async ({ page }) => {
  const rows = await mount(page, { split: true }, { holdQuestionDirectory: true });
  await rows.nth(1).click();
  const answer = page.getByLabel('Your answer', { exact: true });
  await answer.fill('Keep the directory admission fence.');
  await answer.press('Enter');
  await expect(page.getByRole('button', { name: /^Send answer/ })).toBeDisabled();
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await run(page, 'finishQuestionDirectory');
  await expect(
    page.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeEnabled();
  await answer.press('Enter');
  await expect(rows.nth(1)).toContainText('Answered: Keep the directory admission fence.');
  await expect(rows.first()).toContainText('Needs you');
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});

for (const width of [1440, 390]) {
  test(`compact rows stay flat and legible at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    const rows = await mount(page, { darkPage: true });
    await page.evaluate(() => {
      document.documentElement.dataset.theme = 'dark';
    });
    const row = rows.first();
    // The block is the page sheet (dark here) with a light rule, whatever the page paints.
    await expect(page.locator('.question-compact-block')).toHaveCSS(
      'background-color',
      rgb(tokens.browser.surface.sheet.dark),
    );
    await expect(page.locator('.question-compact-block')).toHaveCSS(
      'outline-color',
      rgb(tokens.browser.surface.rule.dark),
    );
    await expect(row.locator('.question-line-chevron svg')).toHaveCSS('width', '16px');
    await expect(row.locator('.question-line-chevron svg')).toHaveCSS('height', '16px');
    expect(
      await row
        .locator('.question-line-state')
        .evaluate((node) => getComputedStyle(node, '::before').content),
    ).toBe(width === 390 ? 'none' : '"· "');
    await row.focus();
    await row.press('Enter');
    await page.mouse.move(0, 0);
    await row.focus();
    await expect(row).toHaveAttribute('aria-expanded', 'true');
    await expect(row).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await expect(row).not.toHaveCSS('outline-style', 'none');
    for (const surface of [
      page.locator('.question-card:visible .question-controls'),
      page.locator('.question-group-footer'),
    ])
      await expect(surface).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await row.hover();
    await expect(row).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await page.getByRole('button', { name: 'Needs you: 1', exact: true }).click();
    const menuRow = page.getByRole('menu').locator('.needs-you-header-row');
    const agent = menuRow.locator('.agent-chip-name');
    await expect(agent).toHaveText('Agent 1');
    await expect(agent).toHaveCSS('white-space', 'nowrap');
    // Long attribution ellipsizes on one row instead of wrapping and clipping its first letter.
    await agent.evaluate((node) => {
      node.textContent = 'Agent with a deliberately long display name';
    });
    expect(
      await menuRow.evaluate((node) => {
        const range = document.createRange();
        range.selectNodeContents(node.querySelector('.agent-chip-name')!);
        const rects = [...range.getClientRects()];
        return rects.length > 0 && rects.every((rect) => Math.abs(rect.y - rects[0].y) < 1);
      }),
    ).toBe(true);
    await expect(menuRow.locator('.needs-you-header-agent')).toHaveCSS('text-overflow', 'ellipsis');
    expect(await run(page, 'discussionProof')).toHaveLength(0);
    expect((await run(page, 'proof')).sends).toHaveLength(0);
  });

  test(`compact stacks reserve their full trusted height at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.route('**/src/main.tsx', (route) =>
      route.fulfill({ contentType: 'text/javascript', body: '' }),
    );
    await page.goto('/');
    await run(page, 'mount');
    await run(page, 'questionGroup', { count: 6 });
    const block = page.locator('.question-compact-block');
    const rows = block.locator('.question-line');
    await expect(rows).toHaveCount(6);
    const after = page.frameLocator('#ask-page-fixture iframe').locator('#after-question');
    const assertSpace = async () => {
      await expect
        .poll(
          async () => {
            const trusted = await block.boundingBox();
            const prose = await after.boundingBox();
            return !!trusted && !!prose && prose.y >= trusted.y + trusted.height - 1;
          },
          { message: 'authored prose must follow the complete trusted stack, not a capped chip' },
        )
        .toBe(true);
    };
    expect((await block.boundingBox())!.height).toBeGreaterThan(200);
    await assertSpace();
    await rows.nth(2).click();
    await page
      .locator('.question-card:visible')
      .getByLabel('Your answer', { exact: true })
      .fill('Retain this expanded passage.');
    await assertSpace();
    await rows.nth(2).click();
    await assertSpace();
    expect(await run(page, 'discussionProof')).toHaveLength(0);
    expect((await run(page, 'proof')).sends).toHaveLength(0);
  });
}
