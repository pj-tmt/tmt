import { expect, test } from '@playwright/test';
import { text } from '../src/strings.js';
import {
  fixture,
  PNG,
  mount,
  actions,
  threads,
  openChat,
  attach,
  send,
} from '../test/attachment-page.js';

const note = (name: string) => ({
  name,
  mimeType: 'text/plain',
  buffer: Buffer.from(`bytes of ${name}`),
});

test('a narrow thread thumbnail aligns with the message and keeps a long filename in its accessible label', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await mount(page);
  const quote = page.frameLocator('#ask-page-fixture iframe').locator('#selected');
  await quote.evaluate((node) => {
    const selection = getSelection()!;
    const range = document.createRange();
    range.selectNodeContents(node);
    selection.removeAllRanges();
    selection.addRange(range);
  });
  await page.getByTestId('selection-ask').click();
  await page.getByRole('combobox', { name: 'Message', exact: true }).fill('Review the image.');
  await attach(page, [
    {
      name: 'a-very-long-image-filename-that-must-wrap-on-a-narrow-thread.png',
      mimeType: 'image/png',
      buffer: PNG,
    },
  ]);
  await send(page).click();
  const row = page.getByTestId('message-attachment');
  await expect(row).toBeVisible();
  const geometry = await row.evaluate((row) => {
    const image = row.querySelector('button')!.getBoundingClientRect();
    const body = row
      .closest('.conversation-body')!
      .querySelector('.comment-body')!
      .getBoundingClientRect();
    return {
      left: Math.abs(image.left - body.left),
      width: row.clientWidth,
      scroll: row.scrollWidth,
    };
  });
  expect(geometry.left).toBeLessThanOrEqual(1);
  expect(geometry.scroll).toBeLessThanOrEqual(geometry.width);
  await expect(row.locator('img')).toBeVisible();
  await row.getByRole('button', { name: /Preview a-very-long-image-filename/ }).click();
  await expect(
    page.getByRole('dialog', { name: /a-very-long-image-filename/ }).locator('img'),
  ).toBeVisible();
});

test('choosing files makes local chips only; Send uploads them and the message shows them', async ({
  page,
}) => {
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Two files for you');
  await attach(page, [note('notes.txt'), { name: 'shot.png', mimeType: 'image/png', buffer: PNG }]);
  const chips = panel.getByTestId('attachment-chip');
  await expect(chips).toHaveCount(2);
  await expect(chips.first()).toContainText(text.attachReady);
  // Choosing stored nothing, sent nothing and left the draft text alone.
  expect(await actions(page)).toEqual([]);
  await expect(input).toHaveText('Two files for you', { useInnerText: true });
  await send(page).click();
  await expect(panel.getByTestId('message-attachment')).toHaveCount(2);
  expect((await actions(page)).filter((action) => !action.startsWith('attach:open'))).toEqual([
    'attach:upload:notes.txt',
    'attach:upload:shot.png',
    'message:Two files for you:2',
  ]);
  await expect(chips).toHaveCount(0);
  await expect(input).toHaveText('', { useInnerText: true });
});

test('a visible raster previews from a data URL; any file downloads its exact bytes and URLs are revoked', async ({
  page,
}) => {
  await page.addInitScript(() => {
    const created: string[] = [];
    const revoked: string[] = [];
    const create = URL.createObjectURL.bind(URL);
    URL.createObjectURL = (value) => {
      const url = create(value);
      created.push(url);
      return url;
    };
    const revoke = URL.revokeObjectURL.bind(URL);
    URL.revokeObjectURL = (url) => {
      revoked.push(url);
      revoke(url);
    };
    (window as unknown as { urls: unknown }).urls = { created, revoked };
  });
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Look');
  await attach(page, [note('notes.txt'), { name: 'shot.png', mimeType: 'image/png', buffer: PNG }]);
  await send(page).click();
  const entries = panel.getByTestId('message-attachment');
  await expect(entries).toHaveCount(2);
  const png = entries.filter({ has: page.getByRole('button', { name: 'Preview shot.png' }) });
  const notes = entries.filter({ hasText: 'notes.txt' });
  // Only the visible raster reads automatically; the generic file still needs a click.
  await expect(png.locator('img')).toHaveAttribute('src', /^data:image\/png;base64,/);
  expect((await actions(page)).filter((a) => a.startsWith('attach:open'))).toEqual([
    'attach:open:shot.png:1',
  ]);
  await expect(png.locator('img')).toHaveAttribute('alt', 'shot.png');
  await expect(png.locator('img')).toHaveJSProperty('complete', true);
  await png.getByRole('button', { name: 'Preview shot.png' }).click();
  await expect(page.locator('.attachment-viewer').locator('img')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(png.getByRole('button')).toBeFocused();

  const download = page.waitForEvent('download');
  await notes.getByRole('button', { name: text.attachmentDownload }).click();
  const saved = await download;
  expect(saved.suggestedFilename()).toBe('notes.txt');
  const path = await saved.path();
  expect((await import('node:fs')).readFileSync(path).toString()).toBe('bytes of notes.txt');
  expect(await actions(page)).toContain('attach:open:notes.txt:1');
  await expect
    .poll(() =>
      page.evaluate(() => {
        const u = (window as unknown as { urls: { created: string[]; revoked: string[] } }).urls;
        return u.created.length > 0 && u.created.every((url) => u.revoked.includes(url));
      }),
    )
    .toBe(true);
});

test('dropping and pasting files add chips; a paste that carries text stays a text paste', async ({
  page,
}) => {
  await mount(page);
  const { panel } = await openChat(page);
  const compose = panel.getByTestId('annotation-compose');
  await compose.evaluate((node) => {
    const data = new DataTransfer();
    data.items.add(new File(['dropped'], 'dropped.txt', { type: 'text/plain' }));
    node.dispatchEvent(
      new DragEvent('drop', { dataTransfer: data, bubbles: true, cancelable: true }),
    );
  });
  await expect(panel.getByTestId('attachment-chip')).toHaveCount(1);
  await compose.evaluate((node) => {
    const data = new DataTransfer();
    data.items.add(new File(['pasted'], 'pasted.png', { type: 'image/png' }));
    node.dispatchEvent(
      new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }),
    );
  });
  await expect(panel.getByTestId('attachment-chip')).toHaveCount(2);
  await compose.evaluate((node) => {
    const data = new DataTransfer();
    data.setData('text/plain', 'rich text');
    data.items.add(new File(['also'], 'also.png', { type: 'image/png' }));
    node.dispatchEvent(
      new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }),
    );
  });
  await expect(panel.getByTestId('attachment-chip')).toHaveCount(2);
  expect(await actions(page)).toEqual([]);
});

test('empty, oversized and surplus files are refused as notices; chips can be removed', async ({
  page,
}) => {
  await mount(page);
  const { panel } = await openChat(page);
  await attach(page, [
    { name: 'empty.txt', mimeType: 'text/plain', buffer: Buffer.alloc(0) },
    {
      name: 'huge.bin',
      mimeType: 'application/octet-stream',
      buffer: Buffer.alloc(8 * 1024 * 1024 + 1),
    },
    note('ok.txt'),
  ]);
  const notices = panel.getByTestId('attachment-notice');
  await expect(notices).toHaveCount(2);
  await expect(notices.nth(0)).toContainText(text.attachNotice.empty('empty.txt'));
  await expect(notices.nth(1)).toContainText(text.attachNotice['too-large']('huge.bin'));
  await expect(panel.getByTestId('attachment-chip')).toHaveCount(1);
  await notices.nth(0).getByRole('button', { name: text.attachDismiss }).click();
  await expect(notices).toHaveCount(1);
  await panel.getByRole('button', { name: text.attachRemove }).click();
  await expect(panel.getByTestId('attachment-chip')).toHaveCount(0);
  expect(await actions(page)).toEqual([]);
});

test('a refused upload sends nothing, keeps the draft and never retries by itself', async ({
  page,
}) => {
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Please keep this text');
  await attach(page, [note('refuse.bin')]);
  await send(page).click();
  await expect(panel.getByRole('alert')).toHaveText(text.attachBlocked.failed);
  const chip = panel.getByTestId('attachment-chip');
  await expect(chip).toHaveAttribute('data-state', 'refused');
  await expect(chip).toContainText(text.attachRefused.capacity);
  await expect(input).toHaveText('Please keep this text', { useInnerText: true });
  await send(page).click();
  await expect(panel.getByRole('alert')).toHaveText(text.attachBlocked.refused);
  // Two Sends, one upload attempt: the refused file was not retried by either.
  expect(await actions(page)).toEqual(['attach:upload:refuse.bin']);
  await chip.getByRole('button', { name: text.attachRemove }).click();
  await send(page).click();
  await expect(panel.getByTestId('chat-thread')).toHaveCount(1);
  expect((await actions(page)).at(-1)).toBe('message:Please keep this text:0');
});

test('an unconfirmed upload blocks Send until its own status is checked', async ({ page }) => {
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Maybe stored');
  await attach(page, [note('unknown.bin')]);
  await send(page).click();
  const chip = panel.getByTestId('attachment-chip');
  await expect(chip).toHaveAttribute('data-state', 'unknown');
  await expect(chip).toContainText(text.attachUnknown);
  await send(page).click();
  await expect(panel.getByRole('alert')).toHaveText(text.attachBlocked.unknown);
  expect((await actions(page)).filter((a) => a.startsWith('attach:upload'))).toHaveLength(1);
  await chip.getByRole('button', { name: text.attachCheck }).click();
  await expect(chip).toHaveAttribute('data-state', 'stored');
  expect((await actions(page)).filter((a) => a.startsWith('attach:resume'))).toHaveLength(1);
  await send(page).click();
  await expect(panel.getByTestId('message-attachment')).toHaveCount(1);
  expect((await actions(page)).filter((a) => a.startsWith('attach:upload'))).toHaveLength(1);
});

test('a page that changed mid-attach asks the user to send again and reuses the local bytes', async ({
  page,
}) => {
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Racing the page');
  await attach(page, [note('stale.txt')]);
  await send(page).click();
  await expect(panel.getByRole('alert')).toHaveText(text.attachBlocked.stale);
  const chip = panel.getByTestId('attachment-chip');
  await expect(chip).toHaveAttribute('data-state', 'again');
  await expect(chip).toContainText(text.attachAgainPageChanged);
  await expect(input).toHaveText('Racing the page', { useInnerText: true });
  await send(page).click();
  await expect(panel.getByTestId('message-attachment')).toHaveCount(1);
  const done = await actions(page);
  // The file was not picked again, and the first original was released.
  expect(done.filter((a) => a === 'attach:upload:stale.txt')).toHaveLength(2);
  expect(done).toContain('attach:discard:stale.txt');
  expect(done.at(-1)).toBe('message:Racing the page:1');
  const [thread] = await threads(page);
  expect(thread.comments).toHaveLength(1);
});

test('a file that cannot be opened is reported on its own row without touching others', async ({
  page,
}) => {
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('One is missing');
  await attach(page, [note('missing.txt'), note('fine.txt')]);
  await send(page).click();
  const entries = panel.getByTestId('message-attachment');
  await entries.nth(0).getByRole('button', { name: text.attachmentDownload }).click();
  await expect(entries.nth(0).getByRole('alert')).toHaveText(text.attachmentUnavailable);
  await expect(entries.nth(1).getByRole('alert')).toHaveCount(0);
});

const captures = process.env.COLAB_1854_CAPTURE_DIR;
for (const width of [1440, 390])
  for (const scheme of ['light', 'dark'] as const)
    test(`captures the attachment states at ${width} ${scheme}`, async ({ page }) => {
      test.skip(!captures, 'Set COLAB_1854_CAPTURE_DIR to write UX review captures.');
      await page.emulateMedia({ colorScheme: scheme });
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      const { panel, input } = await openChat(page);
      await input.fill('Files for review');
      await attach(page, [
        note('notes.txt'),
        { name: 'shot.png', mimeType: 'image/png', buffer: PNG },
        note('refuse.bin'),
      ]);
      const shot = (name: string) =>
        page.screenshot({ path: `${captures}/1854-${name}-${width}-${scheme}.png` });
      await shot('chips-ready');
      await send(page).click();
      await expect(panel.getByTestId('attachment-chip').nth(2)).toHaveAttribute(
        'data-state',
        'refused',
      );
      await shot('chips-refused');
      await panel.getByRole('button', { name: text.attachRemove }).last().click();
      await send(page).click();
      const png = panel
        .getByTestId('message-attachment')
        .filter({ has: page.getByRole('button', { name: 'Preview shot.png' }) });
      await expect(png.locator('img')).toBeVisible();
      await shot('message-preview');
    });

test('an epoch or admission change disposes and rereads the visible thumbnail', async ({
  page,
}) => {
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Look');
  await attach(page, [{ name: 'shot.png', mimeType: 'image/png', buffer: PNG }]);
  await send(page).click();
  const png = panel.getByTestId('message-attachment');
  await expect(png.locator('img')).toBeVisible();
  await page.evaluate(async (path) => (await import(path)).advanceDisclosure(), fixture);
  await expect
    .poll(
      async () => (await actions(page)).filter((a) => a.startsWith('attach:open:shot.png')).length,
    )
    .toBe(2);
  await expect(png.locator('img')).toBeVisible();
});

test('stacked downloads stay clickable through a tooltip and dismiss it on activation', async ({
  page,
}) => {
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Two downloads');
  await attach(page, [note('first.txt'), note('second.txt')]);
  await send(page).click();
  const rows = panel.getByTestId('message-attachment');
  const first = rows
    .filter({ hasText: 'first.txt' })
    .getByRole('button', { name: text.attachmentDownload });
  const second = rows
    .filter({ hasText: 'second.txt' })
    .getByRole('button', { name: text.attachmentDownload });
  await first.hover();
  const tooltip = page.locator('.tmt-ui-icon-action-tooltip:popover-open');
  await expect(tooltip).toBeVisible();
  await expect(tooltip).toHaveCSS('pointer-events', 'none');
  expect(
    await second.evaluate((button) => {
      const r = button.getBoundingClientRect();
      return button.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
    }),
  ).toBe(true);
  for (const [button, name] of [
    [first, 'first.txt'],
    [second, 'second.txt'],
  ] as const) {
    const download = page.waitForEvent('download');
    await button.click();
    expect((await download).suggestedFilename()).toBe(name);
    await expect(tooltip).toHaveCount(0);
  }
  await first.hover();
  await expect(tooltip).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(tooltip).toHaveCount(0);
  await expect(panel).toBeHidden();
});
