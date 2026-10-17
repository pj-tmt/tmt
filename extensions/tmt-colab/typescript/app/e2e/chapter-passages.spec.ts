import { capturePath } from './captures.js';
import { expect, test, type Page } from '@playwright/test';

const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
async function mount(page: Page, kind: 'reading' | 'review') {
  await page.goto('/');
  await run(page, 'mount', { todayPassages: true, statusAgents: true });
  await run(page, kind, kind === 'reading' ? { count: 2 } : { sections: 2 });
  const passage = page.locator('[data-chapter-passage]');
  await expect(passage).toBeVisible();
  return passage;
}

test('Today uses the ordinary Mark read and notice owner and keeps the completed passage until Next', async ({
  page,
}) => {
  const passage = await mount(page, 'reading');
  const first = passage.locator('[data-inline-reading]').first();
  await expect(first).not.toHaveAttribute('data-detached');
  await first.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect(first).toHaveAttribute('data-reading-state', 'read');
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
  await expect(first).toBeVisible();
  await expect(page.locator('#ask-page-fixture .today-count')).toContainText(
    '1 thing waits for you',
  );
  await expect(
    page
      .locator('#ask-page-fixture .today-chapter-state')
      .getByText('Needs you 1', { exact: true }),
  ).toBeVisible();
  await expect(
    first.locator('.needs-you-actions').getByRole('button', { name: 'Next ›', exact: true }),
  ).toHaveCSS('text-decoration-line', 'underline');
  await passage.getByRole('button', { name: 'Next ›', exact: true }).click();
  await expect(first).toBeHidden();
  await expect(
    passage
      .locator('[data-inline-reading]')
      .nth(1)
      .getByRole('button', { name: 'Mark read', exact: true }),
  ).toBeVisible();
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
});

test('Today Confirm records and closes through the existing review owner, outside the author viewport', async ({
  page,
}) => {
  const passage = await mount(page, 'review');
  const confirm = passage.getByRole('button', { name: /Confirm and close/ }).first();
  await expect(confirm).toBeVisible();
  const [viewport, button] = await Promise.all([
    passage.locator('.chapter-passage-clip').boundingBox(),
    confirm.boundingBox(),
  ]);
  expect(button!.y).toBeGreaterThanOrEqual(viewport!.y + viewport!.height);
  await confirm.click();
  await expect(passage.locator('[data-inline-review]').first()).toContainText('Closed');
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
  await expect(passage.getByRole('button', { name: 'Next ›', exact: true })).toBeVisible();
});

test('chip-form answers expand in the chapter and use the existing explicit Send', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { todayPassages: true, statusAgents: true });
  await run(page, 'chipPage');
  const passage = page.locator('[data-chapter-passage]');
  const group = passage.locator('[data-question-group]').first();
  await expect(group).not.toHaveAttribute('data-detached');
  await group.getByRole('radio', { name: 'One column', exact: true }).check();
  await group.getByRole('button', { name: /^Send 1 answer/ }).click();
  await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(1);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  const records = await run(page, 'discussionProof');
  expect(records[0].comments).toHaveLength(1);
  expect(records[0].comments[0].body).toContain('One column');
});

test('a selected review exposes its contained question through the ordinary answer owner', async ({
  page,
}) => {
  const passage = await mount(page, 'review');
  await run(page, 'review', { question: true });
  await expect(passage.locator('.chapter-passage-kicker')).toHaveText('Please confirm');
  const answer = passage.getByRole('textbox', { name: 'Your answer', exact: true });
  await expect(answer).toBeVisible();
  await answer.fill('Use the clearer heading.');
  await passage.getByRole('button', { name: /^Send answer/ }).click();
  await expect(passage.locator('[data-inline-review]').first()).toContainText('Closed');
  const records = await run(page, 'discussionProof');
  expect(records).toHaveLength(2);
  expect(records.flatMap((thread: { comments: unknown[] }) => thread.comments)).toHaveLength(2);
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
});

test('a chapter review draft follows its shared page owner into the foreground and back', async ({
  page,
}) => {
  const passage = await mount(page, 'review');
  const card = passage.locator('[data-inline-review]').first();
  await card.getByRole('button', { name: 'Add a note', exact: true }).click();
  await card.getByRole('textbox').fill('Retain this unsent chapter note.');
  await page.locator('#ask-page-fixture .today-title a').click();
  const full = page.locator('#ask-page-fixture [data-inline-review]').first();
  await expect(full.getByRole('textbox')).toHaveValue('Retain this unsent chapter note.');
  await page.evaluate(() => {
    location.hash = '/';
  });
  await expect(passage.locator('[data-inline-review]').first().getByRole('textbox')).toHaveValue(
    'Retain this unsent chapter note.',
  );
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

for (const returnToToday of [false, true])
  test(`a retired chapter sends its answer once and keeps the review open${returnToToday ? ' after opening and closing the full page' : ''}`, async ({
    page,
  }) => {
    await page.goto('/');
    await run(page, 'mount', {
      todayPassages: true,
      statusAgents: true,
      holdQuestionView: true,
    });
    await run(page, 'review', { question: true });
    const passage = page.locator('[data-chapter-passage]');
    await passage
      .getByRole('textbox', { name: 'Your answer', exact: true })
      .fill('A clearer heading.');
    await run(page, 'pausePrepare');
    await passage.getByRole('button', { name: /^Send answer/ }).click();
    await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(1);
    await page.locator('#ask-page-fixture .today-title a').click();
    await expect(passage.locator('iframe')).toHaveCount(0);
    if (returnToToday) {
      await page.evaluate(() => {
        location.hash = '/';
      });
      await expect(passage).toBeVisible();
    }
    await run(page, 'releaseQuestionView');
    await run(page, 'resumePrepare');
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
    const review = page.locator('#ask-page-fixture .review-card').first();
    await expect(review).toHaveAttribute('data-review-state', 'waiting');
    await expect(review.getByRole('button', { name: /Confirm and close/ })).toBeEnabled();
    await expect(review.getByRole('alert')).toHaveCount(0);
    expect((await run(page, 'proof')).actions).toEqual([]);
    expect(await run(page, 'discussionProof')).toHaveLength(1);
    if (!returnToToday) await expect(passage).toHaveCount(0);
  });

test('a page opening retires the chapter frame while its existing notification finishes once', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { todayPassages: true, statusAgents: true, holdStatusNotice: true });
  await run(page, 'reading', { count: 2 });
  const passage = page.locator('[data-chapter-passage]');
  await passage.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect
    .poll(async () => (await run(page, 'proof')).actions)
    .toEqual(['status:resolve', 'notify:Agent 1']);
  await page.locator('#ask-page-fixture .today-title a').click();
  await expect(passage).toBeHidden();
  await expect(passage.locator('iframe')).toHaveCount(0);
  await expect(page.locator('#ask-page-fixture iframe')).toHaveCount(1);
  await run(page, 'finishStatusNotice');
  await expect(passage).toHaveCount(0);
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
});

for (const hide of ['scroll', 'visibility'] as const)
  test(`a busy chapter keeps its inert frame while its slot remains after ${hide}`, async ({
    page,
  }) => {
    await page.goto('/');
    await run(page, 'mount', { todayPassages: true, statusAgents: true, holdStatusNotice: true });
    await run(page, 'reading', { count: 2 });
    const passage = page.locator('[data-chapter-passage]');
    await passage.getByRole('button', { name: 'Mark read', exact: true }).click();
    await expect
      .poll(async () => (await run(page, 'proof')).actions)
      .toEqual(['status:resolve', 'notify:Agent 1']);
    await page.evaluate((hide) => {
      if (hide === 'visibility') {
        Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
        document.dispatchEvent(new Event('visibilitychange'));
      } else {
        const spacer = document.createElement('div');
        spacer.style.height = '2000px';
        document.body.append(spacer);
        spacer.scrollIntoView({ block: 'start' });
      }
    }, hide);
    await expect(passage).toBeHidden();
    await expect(passage).toHaveAttribute('inert', '');
    await expect(passage.locator('iframe')).toHaveCount(1);
    await run(page, 'finishStatusNotice');
    await expect(passage).toHaveCount(0);
    expect(await run(page, 'discussionProof')).toHaveLength(1);
    expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
  });

test('an answer already recorded in a chapter finishes its one Send after opening the page', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { todayPassages: true, statusAgents: true });
  await run(page, 'chipPage');
  const passage = page.locator('[data-chapter-passage]');
  const group = passage.locator('[data-question-group]').first();
  await group.getByRole('radio', { name: 'One column', exact: true }).check();
  await run(page, 'pausePrepare');
  await group.getByRole('button', { name: /^Send 1 answer/ }).click();
  await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(1);
  await expect(group.getByRole('button', { name: /^Send 1 answer/ })).toBeDisabled();
  await page.locator('#ask-page-fixture .today-title a').click();
  await expect(passage).toBeHidden();
  await expect(passage.locator('iframe')).toHaveCount(0);
  await run(page, 'resumePrepare');
  await expect(passage).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toHaveLength(1);
  const records = await run(page, 'discussionProof');
  expect(records).toHaveLength(1);
  expect(records[0].comments).toHaveLength(1);
});

test('an archived chapter retains its passage without writable controls', async ({ page }) => {
  const passage = await mount(page, 'reading');
  await expect(passage.getByRole('button', { name: 'Mark read', exact: true })).toBeVisible();
  await run(page, 'archive');
  await expect(passage.getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  await expect(passage).toBeVisible();
  await expect(passage.locator('iframe')).toHaveCount(1);
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

test('a failed live chapter retires its frame and actions while retaining its page entry', async ({
  page,
}) => {
  const passage = await mount(page, 'reading');
  await expect(passage.getByRole('button', { name: 'Mark read', exact: true })).toBeVisible();
  await run(page, 'block');
  await expect(passage.getByText('Preview unavailable', { exact: false })).toBeVisible();
  await expect(passage.getByRole('button', { name: 'Open the page', exact: true })).toBeVisible();
  await expect(passage.locator('iframe')).toHaveCount(0);
  await expect(passage.getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

for (const width of [1440, 390]) {
  test(`a clipped reading at ${width} keeps its real controls outside the viewport`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const passage = await mount(page, 'reading');
    await run(page, 'reading', { count: 2, tall: true });
    const clip = passage.locator('.chapter-passage-clip');
    const mark = passage.getByRole('button', { name: 'Mark read', exact: true });
    await expect(mark).toBeVisible();
    await expect(
      passage.getByRole('button', { name: 'Continue on the page →', exact: true }),
    ).toBeVisible();
    const [box, button] = await Promise.all([clip.boundingBox(), mark.boundingBox()]);
    expect(box!.height).toBe(width === 390 ? 320 : 420);
    expect(button!.y).toBeGreaterThanOrEqual(box!.y + box!.height);
    expect(await run(page, 'discussionProof')).toHaveLength(0);
  });
}

test('a refused chapter keeps its page link and exposes no source or action', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', { todayPassages: true, refuseChapter: true });
  const fixtureApp = page.locator('#ask-page-fixture');
  await expect(fixtureApp.getByText('Preview unavailable', { exact: false })).toBeVisible();
  await expect(fixtureApp.getByRole('button', { name: 'Open the page', exact: true })).toHaveCSS(
    'text-decoration-line',
    'underline',
  );
  await expect(fixtureApp.locator('[data-chapter-slot] iframe')).toHaveCount(0);
  await expect(fixtureApp.getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`chapter passage states at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 1100 });
      await page.emulateMedia({ colorScheme: theme });
      const app = page.locator('#ask-page-fixture');
      const capture = async (state: string) => {
        if (process.env.COLAB_CAPTURE_DIR)
          await page.screenshot({
            path: capturePath(`chapter-${state}-${width}-${theme}.png`),
            fullPage: true,
          });
      };
      for (const kind of ['review', 'answer', 'reading'] as const) {
        await page.goto('/');
        await run(page, 'mount', { todayPassages: true, statusAgents: true });
        await run(page, kind === 'answer' ? 'questionGroup' : kind, {
          darkPage: theme === 'dark',
          ...(kind === 'review' ? { sections: 2 } : { count: 2 }),
        });
        const passage = app.locator('[data-chapter-passage]');
        await expect(passage.locator('.chapter-passage-clip')).toBeVisible();
        await expect(passage.locator('.chapter-passage-kicker')).toHaveText(
          { review: 'Please confirm', answer: 'Please answer', reading: 'Please read' }[kind],
        );
        await expect(passage.locator('.chapter-passage-controls')).toBeVisible();
        await capture(kind);
        if (kind === 'review') {
          await run(page, 'review', { question: true, darkPage: theme === 'dark' });
          await expect(
            passage.getByRole('textbox', { name: 'Your answer', exact: true }),
          ).toBeVisible();
          await capture('review-answer');
        }
        if (kind === 'reading') {
          await run(page, 'reading', { count: 2, tall: true, darkPage: theme === 'dark' });
          await expect(
            passage.getByRole('button', { name: 'Continue on the page →', exact: true }),
          ).toBeVisible();
          await capture('clipped');
          await passage.getByRole('button', { name: 'Mark read', exact: true }).click();
          await expect(passage.locator('[data-inline-reading]').first()).toHaveAttribute(
            'data-reading-state',
            'read',
          );
          await expect(passage.getByRole('button', { name: 'Next ›', exact: true })).toBeVisible();
          await capture('done-next');
          await expect(passage.locator('[data-inline-reading]').first()).toBeVisible();
          await expect(passage.locator('[data-inline-reading]').first()).toHaveAttribute(
            'data-reading-state',
            'read',
          );
        }
      }
      await page.goto('/');
      await run(page, 'mount', { todayPassages: true, pastReplies: true });
      await expect(app.locator('.chapter-passage-kicker')).toContainText('Latest reply');
      await expect(app.locator('.chapter-passage-clip')).toBeVisible();
      await expect(app.locator('.chapter-passage-reply .conversation-avatar')).toHaveCount(0);
      const [meta, body] = await Promise.all([
        app.locator('.chapter-passage-reply .comment-byline').boundingBox(),
        app.locator('.chapter-passage-reply .conversation-body').boundingBox(),
      ]);
      expect(meta!.x).toBe(body!.x);
      await capture('reply');
      await page.goto('/');
      await run(page, 'mount', { todayPassages: true });
      await expect(app.locator('.chapter-passage-kicker')).toHaveText('Start of page');
      await expect(app.locator('.chapter-passage-clip')).toBeVisible();
      await expect(app.locator('.chapter-passage-controls')).toBeHidden();
      await capture('opening');
      await page.goto('/');
      await run(page, 'mount', { todayPassages: true, refuseChapter: true });
      await expect(app.getByRole('button', { name: 'Open the page', exact: true })).toBeVisible();
      await expect(app.locator('[data-chapter-slot] iframe')).toHaveCount(0);
      await capture('unavailable');
    });
  }

test('the excerpt contains its full heading and ends before the next authored passage', async ({
  page,
}) => {
  const passage = await mount(page, 'reading');
  const clip = passage.locator('.chapter-passage-clip');
  const frame = passage.frameLocator('iframe');
  const [viewport, heading, second] = await Promise.all([
    clip.boundingBox(),
    frame.locator('tmt-read').first().locator('[data-title]').boundingBox(),
    frame.locator('tmt-read').nth(1).locator('[data-title]').boundingBox(),
  ]);
  expect(heading!.y).toBeGreaterThanOrEqual(viewport!.y);
  expect(heading!.y + heading!.height).toBeLessThanOrEqual(viewport!.y + viewport!.height);
  expect(second!.y).toBeGreaterThanOrEqual(viewport!.y + viewport!.height);
});

test('a short opening includes the complete paragraph and does not claim overflow', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { todayPassages: true });
  const passage = page.locator('[data-chapter-passage]');
  const clip = passage.locator('.chapter-passage-clip');
  await expect(clip).toBeVisible();
  const [box, paragraph] = await Promise.all([
    clip.boundingBox(),
    passage.frameLocator('iframe').locator('#selected').boundingBox(),
  ]);
  expect(paragraph!.y).toBeGreaterThanOrEqual(box!.y);
  expect(paragraph!.y + paragraph!.height).toBeLessThanOrEqual(box!.y + box!.height);
  await expect(page.locator('#ask-page-fixture .today-kicker')).not.toContainText('Waiting');
  await expect(
    passage.getByRole('button', { name: 'Continue on the page →', exact: true }),
  ).toHaveCount(0);
});

test('a short reply does not inherit overflow from the hidden source frame', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', { todayPassages: true, pastReplies: true });
  const passage = page.locator('[data-chapter-passage]');
  await expect(passage.locator('.chapter-passage-kicker')).toContainText('Latest reply');
  await expect(passage.locator('.conversation-turn')).toBeVisible();
  await expect(
    passage.getByRole('button', { name: 'Continue on the page →', exact: true }),
  ).toHaveCount(0);
});
