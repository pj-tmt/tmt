import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';

/**
 * A page shows its document before its threads and asks are verified. Until then nothing reads
 * as verified: no comment count, no "Nothing waits", no bands or composers, only a quiet place
 * where the discussion will be. Verification then fills everything in without moving the document.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');

async function open(page: Page, width = 1440) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, discussionPending: true });
  // A thread the viewer will see once it is verified: the fixture holds it back until then.
  await run(page, 'conversation', { surface: 'thread', state: 'replied' });
  await expect(frame(page).locator('#selected')).toBeVisible();
}
/** Where the header actions, the status chip and the floating buttons sit, in order. */
const placed = (page: Page) =>
  page.evaluate(() => {
    const nodes = [
      ...document.querySelectorAll(
        '#ask-page-fixture .page-header-actions button, #ask-page-fixture [aria-label="Search pages"], #ask-page-fixture .page-header-meta .status, #ask-page-fixture .page-floating-actions button',
      ),
    ];
    return nodes.map((node) => {
      const { x, y, width, height } = node.getBoundingClientRect();
      return {
        name: node.getAttribute('aria-label') ?? node.textContent?.trim().slice(0, 24) ?? '',
        x: Math.round(x * 100) / 100,
        y: Math.round(y * 100) / 100,
        width: Math.round(width * 100) / 100,
        height: Math.round(height * 100) / 100,
      };
    });
  });
const capture = async (page: Page, name: string, width: number) => {
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`progressive-${name}-${width}.png`) });
};

for (const width of [1440, 390])
  test(`the document is shown at ${width}px with no count, no Needs you and no bands until verified`, async ({
    page,
  }) => {
    await open(page, width);
    // Nothing unverified reads as verified: no "(0)", no "✓ Nothing waits".
    await expect(page.getByRole('button', { name: 'Discussion', exact: true })).toBeVisible();
    const needsYou = page.locator(
      '#ask-page-fixture [aria-label*="Needs you"], #ask-page-fixture [aria-label*="Nothing waits"]',
    );
    await expect(needsYou).toHaveCount(0);
    await expect(page.locator('#ask-page-fixture').getByText('Nothing waits')).toHaveCount(0);
    await expect(
      page.locator('[data-inline-reading], [data-inline-review], .proposal-inline'),
    ).toHaveCount(0);
    await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(0);
    await capture(page, 'pending', width);
    // The select button holds its place, fenced rather than greyed.
    const select = page.getByRole('button', { name: 'Selection mode' });
    await expect(select).toHaveAttribute('aria-busy', 'true');
    await expect(select).toHaveAttribute('aria-disabled', 'true');
    expect(await select.evaluate((node) => (node as HTMLButtonElement).disabled)).toBe(false);
    const pending = await placed(page);
    await run(page, 'verifyDiscussion');
    await expect(page.getByRole('button', { name: /^Discussion \(1\)/ })).toBeVisible();
    await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(1);
    await expect(needsYou).toHaveCount(1);
    await expect(select).not.toHaveAttribute('aria-busy', 'true');
    // Nothing the person aims at moves when verification arrives.
    const verified = await placed(page);
    expect(verified.length).toBeGreaterThanOrEqual(5);
    // The Discussion button gains its number in its name; every box stays where it was, to the
    // layout engine's 1/64 px.
    verified.forEach((box, index) => {
      const before = pending[index]!;
      for (const key of ['x', 'y', 'width', 'height'] as const)
        expect(Math.abs(box[key] - before[key]), `${before.name} ${key}`).toBeLessThan(0.1);
    });
    await capture(page, 'verified', width);
  });

for (const drawer of ['Comments', 'Chat'] as const)
  test(`the ${drawer} drawer says comments are loading, offers no composer, then fills in`, async ({
    page,
  }) => {
    await open(page);
    await (await pageAction(page, drawer)).click();
    const panel = page.locator('.tmt-ui-drawer[open]');
    await expect(panel.getByRole('status')).toHaveText('Loading comments…');
    await expect(panel.getByRole('textbox').or(panel.getByRole('combobox'))).toHaveCount(0);
    await expect(panel.getByTestId('comment-thread')).toHaveCount(0);
    await run(page, 'verifyDiscussion');
    await expect(panel.getByText('Loading comments…')).toHaveCount(0);
    await expect(
      drawer === 'Chat'
        ? panel.getByRole('combobox', { name: 'Message' })
        : panel.getByRole('button', { name: '+ Comment on page' }),
    ).toBeVisible();
  });

test('the document does not move when verification arrives', async ({ page }) => {
  await open(page);
  const before = await page.locator('#ask-page-fixture iframe').boundingBox();
  await run(page, 'verifyDiscussion');
  await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(1);
  expect(await page.locator('#ask-page-fixture iframe').boundingBox()).toEqual(before);
});

/**
 * The compact lines of answer blocks sit in slots the frame places once the parent asks for them
 * (#4253). The parent mounts them only after the discussion is verified, which can be after the
 * page is ready; a lone question there stayed detached and invisible, so it could not be answered.
 */
test('a lone question placed while the discussion was pending shows its line once verified', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, discussionPending: true, layout: 'column' });
  await run(
    page,
    'authored',
    '<h1>Project notes</h1><p>Before the question.</p><tmt-question data-id="10000000-0000-4000-8000-000000000090" data-kind="single" data-agent="answer-agent"><p data-question>Which heading should we use?</p><span data-choice>Keep it</span><span data-choice>Change it</span></tmt-question><p id="after-question">The page continues here.</p>',
  );
  await expect(frame(page).locator('#after-question')).toBeVisible();
  // The document is ready and nothing of the discussion is mounted yet.
  await expect(page.locator('.question-group-card')).toHaveCount(0);
  await run(page, 'verifyDiscussion');
  const card = page.locator('.question-group-card');
  await expect(card).toHaveCount(1);
  await expect(card).not.toHaveAttribute('data-detached');
  const line = card.locator('.question-line');
  await expect(line).toBeVisible();
  await expect(line).toContainText('Which heading should we use?');
  // The line sits between the paragraphs it was authored between, and opens like any other.
  const before = await frame(page).getByText('Before the question.').boundingBox();
  const after = await frame(page).locator('#after-question').boundingBox();
  const box = (await line.boundingBox())!;
  expect(box.y).toBeGreaterThan(before!.y);
  expect(box.y + box.height).toBeLessThanOrEqual(after!.y + 1);
  await line.click();
  await expect(line).toHaveAttribute('aria-expanded', 'true');
  await expect(card.getByLabel('Keep it', { exact: true })).toBeVisible();
});
