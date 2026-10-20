import { mkdirSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { PNG, actions, fixture } from '../test/attachment-page.js';

/** Answers carry images through the shared attachment tray (#4024). */
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const picture = (name: string) => ({ name, mimeType: 'image/png', buffer: PNG });
/** The uploads and the answer comments the fixture recorded, as `message:<question>:<files>`. */
const published = async (page: Page) =>
  (await actions(page))
    .filter((action) => /^(attach:upload|message:)/.test(action))
    .map((action) => action.replace(/^message:Answer to “([^”]+)”[^]*:(\d+)$/, 'message:$1:$2'));

async function mount(page: Page, options: object = {}) {
  await page.goto('/');
  await run(page, 'mount', { attachments: true, ...options });
  await run(page, 'questionGroup', { count: 3 });
  const group = page.locator('[data-question-group]').first();
  await expect(group).not.toHaveAttribute('data-detached');
  return group;
}
/** The open panel of the question with this title: its answer field and tray. */
async function open(group: Locator, title: RegExp | string) {
  await group.locator('.question-line', { hasText: title }).click();
  return group.locator('.question-compact-controls:not([hidden])');
}
const answerField = (panel: Locator) => panel.getByLabel('Your answer', { exact: true });
const SHORT = 'What should the introduction say?';
const LONG = 'What should section 3 explain?';

/** The card never covers the page's text below it, also after its tray grows. */
const clear = (page: Page, group: Locator) =>
  expect
    .poll(async () => {
      const card = await group.locator('.question-compact-block').boundingBox();
      const below = await page
        .frameLocator('#ask-page-fixture iframe')
        .locator('#after-question')
        .boundingBox();
      return !!card && !!below && below.y >= card.y + card.height - 1;
    })
    .toBe(true);

/** A paste or a drop of files, the way the browser delivers them to the field. */
async function deliver(field: Locator, kind: 'paste' | 'drop', names: string[]) {
  await field.evaluate(
    (node, { kind, names, base64 }) => {
      const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
      const data = new DataTransfer();
      for (const name of names) data.items.add(new File([bytes], name, { type: 'image/png' }));
      const event =
        kind === 'paste'
          ? new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true })
          : new DragEvent('drop', { dataTransfer: data, bubbles: true, cancelable: true });
      node.dispatchEvent(event);
    },
    { kind, names, base64: PNG.toString('base64') },
  );
}

test('only the text answers have a tray, and it takes a pick, a paste and a drop', async ({
  page,
}) => {
  const group = await mount(page);
  // A choice has no tray: there is no text to carry files.
  const choice = await open(group, 'Which heading should we use?');
  await expect(choice.getByTestId('attachment-input')).toHaveCount(0);
  await expect(choice.getByRole('button', { name: text.messageAttach })).toHaveCount(0);

  const short = await open(group, SHORT);
  await expect(short.getByTestId('attachment-input')).toHaveCount(1);
  const chooser = page.waitForEvent('filechooser');
  await short.getByRole('button', { name: text.messageAttach }).click();
  await (await chooser).setFiles([picture('picked.png')]);
  await expect(short.getByTestId('attachment-chip')).toHaveCount(1);
  await deliver(answerField(short), 'paste', ['pasted.png']);
  await expect(short.getByTestId('attachment-chip')).toHaveCount(2);

  const long = await open(group, LONG);
  await expect(long.getByTestId('attachment-input')).toHaveCount(1);
  await deliver(answerField(long), 'drop', ['dropped.png']);
  await expect(long.getByTestId('attachment-chip')).toHaveCount(1);
  // Adding only makes inert local chips: nothing is stored or sent before Send.
  expect((await actions(page)).filter((action) => action.startsWith('attach:'))).toEqual([]);
});

test('a batch Send publishes each answer with its own files and the card shows them', async ({
  page,
}) => {
  const group = await mount(page);
  const short = await open(group, SHORT);
  await answerField(short).fill('Make the purpose clear.');
  const long = await open(group, LONG);
  await answerField(long).fill('Explain the tradeoffs.');
  await deliver(answerField(long), 'paste', ['tradeoffs.png']);
  await expect(long.getByTestId('attachment-chip')).toHaveCount(1);
  await group.getByRole('button', { name: /^Send 2 answers/ }).click();

  await expect
    .poll(() => published(page))
    .toEqual([`message:${SHORT}:0`, 'attach:upload:tradeoffs.png', `message:${LONG}:1`]);
  // The answered card shows the image the way a comment does.
  await group.locator('.question-line', { hasText: LONG }).click();
  const answered = group.locator('.question-compact-controls:not([hidden]) .question-answer');
  await expect(answered).toContainText('Explain the tradeoffs.');
  await expect(answered.getByTestId('message-attachment')).toHaveCount(1);
  await expect(answered.locator('img')).toBeVisible();
  // Once: the answer's own comment in the Conversation below keeps its text, not its files.
  await expect(
    group.locator('.question-compact-controls:not([hidden]) [data-testid="message-attachment"]'),
  ).toHaveCount(1);
  // The answer without files carries none.
  await group.locator('.question-line', { hasText: SHORT }).click();
  await expect(
    group.locator('.question-compact-controls:not([hidden]) [data-testid="message-attachment"]'),
  ).toHaveCount(0);
});

test('a draft keeps its files while the disclosure moves, and a Send that sends nothing uploads nothing', async ({
  page,
}) => {
  const group = await mount(page);
  const long = await open(group, LONG);
  await answerField(long).fill('A draft with a file.');
  await deliver(answerField(long), 'paste', ['kept.png']);
  await expect(long.getByTestId('attachment-chip')).toHaveCount(1);
  // Another question, then the first again, then closed and open.
  await open(group, SHORT);
  const again = await open(group, LONG);
  await expect(answerField(again)).toHaveValue('A draft with a file.');
  await expect(again.getByTestId('attachment-chip')).toHaveCount(1);
  await group.locator('.question-line', { hasText: LONG }).click();
  await group.locator('.question-line', { hasText: LONG }).click();
  await expect(group.getByTestId('attachment-chip')).toHaveCount(1);
  expect((await actions(page)).filter((action) => action.startsWith('attach:'))).toEqual([]);
});

test('files over the shared limits show the shared notices, and a refused upload keeps the answer for the next Send', async ({
  page,
}) => {
  const group = await mount(page);
  const long = await open(group, LONG);
  await answerField(long).fill('One answer.');
  const chooser = page.waitForEvent('filechooser');
  await long.getByRole('button', { name: text.messageAttach }).click();
  await (
    await chooser
  ).setFiles([{ name: 'empty.png', mimeType: 'image/png', buffer: Buffer.alloc(0) }]);
  await expect(long.getByTestId('attachment-notice')).toContainText(
    text.attachNotice.empty('empty.png'),
  );
  await long.getByTestId('attachment-notice').getByRole('button').first().click();
  await deliver(answerField(long), 'paste', ['refuse.png']);
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  // The upload is refused before any comment: the draft stays, with the chip's own reason.
  await expect(long.getByText('Could not confirm this answer.')).toBeVisible();
  await expect(long.getByTestId('attachment-chip')).toContainText(text.attachRefused.capacity);
  expect((await actions(page)).filter((action) => action.startsWith('message:'))).toEqual([]);
});

test('retrying a failed recipient sends no bytes and publishes no comment again', async ({
  page,
}) => {
  const group = await mount(page, { prepareFailures: 1 });
  const long = await open(group, LONG);
  await answerField(long).fill('With a file and a failing recipient.');
  await deliver(answerField(long), 'paste', ['once.png']);
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  await expect(group.getByTestId('recipient-failure')).toHaveCount(1);
  const before = await published(page);
  expect(before).toEqual(['attach:upload:once.png', `message:${LONG}:1`]);
  await group.locator('.question-line', { hasText: LONG }).click();
  await group
    .getByTestId('recipient-failure')
    .getByRole('button', { name: 'Ask again', exact: true })
    .click();
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await expect(group.getByTestId('recipient-failure')).toHaveCount(0);
  expect(await published(page)).toEqual(before);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`the tray in an answer at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 1000 });
      await page.emulateMedia({ colorScheme: theme as 'light' | 'dark', reducedMotion: 'reduce' });
      const group = await mount(page);
      const long = await open(group, LONG);
      await answerField(long).fill('Explain the tradeoffs without repeating the introduction.');
      await deliver(answerField(long), 'paste', ['tradeoffs.png', 'second-screenshot.png']);
      await expect(long.getByTestId('attachment-chip')).toHaveCount(2);
      expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(
        false,
      );
      await clear(page, group);
      const captures = process.env.COLAB_ANSWER_FILES_CAPTURE_DIR;
      if (captures) {
        mkdirSync(captures, { recursive: true });
        await page.screenshot({ fullPage: true, path: `${captures}/draft-${width}-${theme}.png` });
      }
      await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
      await group.locator('.question-line', { hasText: LONG }).click();
      await expect(
        group.locator('.question-answer [data-testid="message-attachment"]'),
      ).toHaveCount(2);
      await expect(group.getByTestId('message-attachment')).toHaveCount(2);
      // Equal thumbnails level at the top of their row, and one line a name.
      const tiles = await group
        .locator('.question-answer .attachment-thumbnail')
        .evaluateAll((nodes) =>
          nodes.map((node) => {
            const box = node.getBoundingClientRect();
            return [Math.round(box.top), Math.round(box.width), Math.round(box.height)];
          }),
        );
      expect(new Set(tiles.map(([top]) => top)).size).toBe(1);
      expect(new Set(tiles.map(([, width, height]) => `${width}x${height}`)).size).toBe(1);
      const lines = await group
        .locator('.question-answer .attachment-name, .question-answer .attachment-size')
        .evaluateAll((nodes) =>
          nodes.map(
            (node) =>
              node.getBoundingClientRect().height / parseFloat(getComputedStyle(node).lineHeight),
          ),
        );
      expect(Math.max(...lines)).toBeLessThan(1.1);
      if (captures)
        await page.screenshot({
          fullPage: true,
          path: `${captures}/answered-${width}-${theme}.png`,
        });
    });
