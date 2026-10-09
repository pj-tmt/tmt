import { pageAction } from '../test/page-actions.js';
import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';

const fixture = '/test/ask-page-browser.tsx';
// A 1x1 PNG: a real raster the parent may preview.
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);
const note = (name: string) => ({
  name,
  mimeType: 'text/plain',
  buffer: Buffer.from(`bytes of ${name}`),
});

async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async ({ fixture }) => (await import(fixture)).mount({ attachments: true }), {
    fixture,
  });
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
}
const actions = async (page: Page): Promise<string[]> =>
  page.evaluate(async (path) => (await import(path)).proof().actions, fixture);
async function openFiles(page: Page) {
  const host = page.locator('#ask-page-fixture');
  const toggle = await pageAction(host, 'Files');
  await toggle.click();
  return page.getByTestId('files-panel');
}
async function attach(panel: ReturnType<Page['getByTestId']>, page: Page, files: unknown[]) {
  const chooser = page.waitForEvent('filechooser');
  await panel.getByRole('button', { name: text.attachFiles }).click();
  await (await chooser).setFiles(files as never);
}
const add = (panel: ReturnType<Page['getByTestId']>) =>
  panel.getByRole('button', { name: text.filesAdd, exact: true });

test('Files is a peer panel: empty for a writer, then rows with a count after Add to page', async ({
  page,
}) => {
  await mount(page);
  const toggle = await pageAction(page.locator('#ask-page-fixture'), 'Files');
  await expect(toggle).toHaveAccessibleName('Files (0)');
  const panel = await openFiles(page);
  await expect(panel.getByText(text.filesEmpty)).toBeVisible();
  await attach(panel, page, [
    note('notes.txt'),
    { name: 'shot.png', mimeType: 'image/png', buffer: PNG },
  ]);
  const chips = panel.getByTestId('attachment-chip');
  await expect(chips).toHaveCount(2);
  await expect(chips.first()).toContainText(text.filesReady);
  // Choosing stored nothing and wrote nothing into the page.
  expect(await actions(page)).toEqual([]);
  await add(panel).click();
  const rows = panel.getByTestId('file-row');
  await expect(rows).toHaveCount(2);
  await expect(chips).toHaveCount(0);
  await expect(toggle).toHaveAccessibleName('Files (2)');
  await expect(toggle.locator('.page-header-count')).toHaveText('2');
  expect(await actions(page)).toEqual([
    'attach:upload:notes.txt',
    'attach:upload:shot.png',
    'files:add:notes.txt,shot.png',
  ]);
  // Bytes open only on click.
  expect((await actions(page)).filter((a) => a.startsWith('attach:open'))).toEqual([]);
  expect(await rows.nth(0).locator('button').allTextContents()).toEqual([
    text.attachmentDownload,
    text.attachRemove,
  ]);
  const png = rows.nth(1);
  await png.getByRole('button', { name: text.attachmentPreview }).click();
  const image = page.getByTestId('file-preview').locator('img');
  await expect(image).toHaveAttribute('src', /^data:image\/png;base64,/);
  await png.getByRole('button', { name: text.attachmentHidePreview }).click();
  await expect(image).toHaveCount(0);
  const download = page.waitForEvent('download');
  await rows.nth(0).getByRole('button', { name: text.attachmentDownload }).click();
  const saved = await download;
  expect(saved.suggestedFilename()).toBe('notes.txt');
  expect((await import('node:fs')).readFileSync((await saved.path())!).toString()).toBe(
    'bytes of notes.txt',
  );
  expect(await actions(page)).toContain('attach:open:notes.txt:document');
});

test('a file sealed under an earlier epoch says it is being secured and stays available', async ({
  page,
}) => {
  await mount(page);
  const panel = await openFiles(page);
  await attach(panel, page, [note('notes.txt')]);
  await add(panel).click();
  const row = panel.getByTestId('file-row');
  await expect(row).toHaveCount(1);
  await expect(row).not.toContainText(text.filesResealing);
  // The page's epoch advances: the file stays listed and downloadable, and says so.
  await page.evaluate(async (path) => (await import(path)).rotateEpoch(), fixture);
  await expect(row).toHaveCount(1);
  await expect(row).toContainText(text.filesResealing);
  await expect(row.getByRole('button', { name: text.attachmentDownload })).toBeEnabled();
});
test('removing a file drops its row and the count; the last removal restores the empty state', async ({
  page,
}) => {
  await mount(page);
  const panel = await openFiles(page);
  await attach(panel, page, [note('one.txt')]);
  await add(panel).click();
  await expect(panel.getByTestId('file-row')).toHaveCount(1);
  await panel.getByRole('button', { name: text.attachRemove }).click();
  await expect(panel.getByTestId('file-row')).toHaveCount(0);
  await expect(panel.getByText(text.filesEmpty)).toBeVisible();
  expect(await actions(page)).toContain('files:remove:1');
});

test('a page that changed while uploading asks to add again; a failed save keeps the chip', async ({
  page,
}) => {
  await mount(page);
  const panel = await openFiles(page);
  await attach(panel, page, [note('stale.txt')]);
  await add(panel).click();
  await expect(
    panel.getByRole('status').filter({ hasText: text.filesBlocked.stale }),
  ).toBeVisible();
  const chip = panel.getByTestId('attachment-chip');
  await expect(chip).toContainText(text.filesAgainPageChanged);
  await add(panel).click();
  await expect(panel.getByTestId('file-row')).toHaveCount(1);
  await expect(chip).toHaveCount(0);

  await attach(panel, page, [note('unsaved.txt')]);
  await add(panel).click();
  await expect(panel.getByRole('status').filter({ hasText: text.filesBlocked.save })).toBeVisible();
  await expect(chip).toHaveCount(1);
  await add(panel).click();
  await expect(panel.getByTestId('file-row')).toHaveCount(2);
});

test('an epoch or admission change disposes shown previews and failures, and a later read starts clean', async ({
  page,
}) => {
  await mount(page);
  const panel = await openFiles(page);
  await attach(panel, page, [
    { name: 'shot.png', mimeType: 'image/png', buffer: PNG },
    note('missing.txt'),
  ]);
  await add(panel).click();
  const rows = panel.getByTestId('file-row');
  await expect(rows).toHaveCount(2);
  await rows.nth(0).getByRole('button', { name: text.attachmentPreview }).click();
  await expect(page.getByTestId('file-preview').locator('img')).toBeVisible();
  await rows.nth(1).getByRole('button', { name: text.attachmentDownload }).click();
  await expect(rows.nth(1).getByRole('alert')).toHaveText(text.attachmentUnavailable);

  await page.evaluate(async (path) => (await import(path)).advanceDisclosure(), fixture);
  await expect(page.getByTestId('file-preview')).toHaveCount(0);
  await expect(rows.nth(1).getByRole('alert')).toHaveCount(0);
  // The preview is only a click away again, read afresh under the new disclosure.
  const opens = (await actions(page)).filter((a) => a.startsWith('attach:open')).length;
  await rows.nth(0).getByRole('button', { name: text.attachmentPreview }).click();
  await expect(page.getByTestId('file-preview').locator('img')).toBeVisible();
  expect((await actions(page)).filter((a) => a.startsWith('attach:open')).length).toBe(opens + 1);
});

test('a read that finishes after the disclosure changed shows nothing', async ({ page }) => {
  await mount(page);
  const panel = await openFiles(page);
  await attach(panel, page, [{ name: 'slow.png', mimeType: 'image/png', buffer: PNG }]);
  await add(panel).click();
  const row = panel.getByTestId('file-row');
  await row.getByRole('button', { name: text.attachmentPreview }).click();
  await expect
    .poll(async () => (await actions(page)).includes('attach:open:slow.png:document'))
    .toBe(true);
  await page.evaluate(async (path) => (await import(path)).advanceDisclosure(), fixture);
  await page.evaluate(async (path) => (await import(path)).releaseOpen(), fixture);
  await expect(row.getByRole('button', { name: text.attachmentPreview })).toBeEnabled();
  await expect(page.getByTestId('file-preview')).toHaveCount(0);
  await expect(row.getByRole('alert')).toHaveCount(0);
});

test('a file that cannot open says why: access ended, no longer on the page, changed, or unavailable', async ({
  page,
}) => {
  await mount(page);
  const panel = await openFiles(page);
  await attach(panel, page, [
    note('denied.txt'),
    note('not-found.txt'),
    note('changed.txt'),
    note('missing.txt'),
  ]);
  await add(panel).click();
  const rows = panel.getByTestId('file-row');
  await expect(rows).toHaveCount(4);
  const reasons = [
    text.attachmentReason.denied,
    text.attachmentReason['not-found'],
    text.attachmentReason.changed,
    text.attachmentReason.unavailable,
  ];
  for (const [index, reason] of reasons.entries()) {
    await rows.nth(index).getByRole('button', { name: text.attachmentDownload }).click();
    await expect(rows.nth(index).getByRole('alert')).toHaveText(reason);
  }
});

const reader = '/test/reader-files-browser.tsx';
async function mountReader(page: Page, files: boolean) {
  await page.goto('/');
  await page.evaluate(async ({ reader, files }) => (await import(reader)).mount(files), {
    reader,
    files,
  });
}

test('a reader link lists files read-only and only when the page has files', async ({ page }) => {
  await mountReader(page, false);
  await expect(await pageAction(page, 'Files')).toHaveCount(0);
  await mountReader(page, true);
  const toggle = await pageAction(page, 'Files');
  await expect(toggle).toHaveText(`${text.files} 3`);
  await toggle.click();
  const rows = page.getByTestId('file-row');
  await expect(rows).toHaveCount(3);
  // Read-only: no Remove and no attach control anywhere in the panel.
  await expect(page.getByRole('button', { name: text.attachRemove })).toHaveCount(0);
  await expect(page.getByRole('button', { name: text.attachFiles })).toHaveCount(0);
  expect(await rows.nth(0).locator('button').allTextContents()).toEqual([text.attachmentDownload]);
  await rows.nth(1).getByRole('button', { name: text.attachmentPreview }).click();
  await expect(page.getByTestId('file-preview').locator('img')).toHaveAttribute(
    'src',
    /^data:image\/png;base64,/,
  );
  const download = page.waitForEvent('download');
  await rows.nth(0).getByRole('button', { name: text.attachmentDownload }).click();
  expect((await download).suggestedFilename()).toBe('report.txt');
  await rows.nth(2).getByRole('button', { name: text.attachmentDownload }).click();
  await expect(rows.nth(2).getByRole('alert')).toHaveText(text.attachmentUnavailable);
  expect(await page.evaluate(async (reader) => (await import(reader)).opens(), reader)).toEqual([
    'document:shot.png',
    'document:report.txt',
    'document:gone.bin',
  ]);
});

const captures = process.env.COLAB_1855_CAPTURE_DIR;
/** A 160x90 two-tone PNG, large enough to show in a review capture. */
async function swatch() {
  const { deflateSync } = await import('node:zlib');
  const width = 160,
    height = 90;
  const raw = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) {
      const at = y * (width * 3 + 1) + 1 + x * 3;
      raw[at] = x < 80 ? 70 : 190;
      raw[at + 1] = 120;
      raw[at + 2] = y < 45 ? 200 : 90;
    }
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (buffer: Buffer) => {
    let c = 0xffffffff;
    for (const byte of buffer) c = crcTable[(c ^ byte) & 0xff]! ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc(body), body.length + 4);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}
for (const width of [1440, 390])
  for (const scheme of ['light', 'dark'] as const)
    test(`captures the Files panel states at ${width} ${scheme}`, async ({ page }) => {
      test.skip(!captures, 'Set COLAB_1855_CAPTURE_DIR to write UX review captures.');
      await page.emulateMedia({ colorScheme: scheme });
      await page.setViewportSize({ width, height: 900 });
      const shot = (name: string) =>
        page.screenshot({ path: `${captures}/1855-${name}-${width}-${scheme}.png` });
      await mount(page);
      const panel = await openFiles(page);
      await shot('empty');
      await attach(panel, page, [
        note('notes.txt'),
        { name: 'shot.png', mimeType: 'image/png', buffer: await swatch() },
        note('refuse.bin'),
      ]);
      await shot('chips-ready');
      await add(panel).click();
      await expect(panel.getByTestId('attachment-chip').nth(2)).toHaveAttribute(
        'data-state',
        'refused',
      );
      await shot('chip-refused');
      await panel.getByRole('button', { name: text.attachRemove }).last().click();
      await add(panel).click();
      const rows = panel.getByTestId('file-row');
      await expect(rows).toHaveCount(2);
      await shot('several');
      await rows.nth(1).getByRole('button', { name: text.attachmentPreview }).click();
      await expect(page.getByTestId('file-preview').locator('img')).toBeVisible();
      await shot('preview');
      await mountReader(page, true);
      await (await pageAction(page, 'Files')).click();
      await expect(page.getByTestId('file-row')).toHaveCount(3);
      await shot('reader');
    });

for (const width of [1440, 390])
  for (const scheme of ['light', 'dark'] as const)
    test(`captures the re-sealing file state at ${width} ${scheme}`, async ({ page }) => {
      test.skip(!captures, 'Set COLAB_1855_CAPTURE_DIR to write UX review captures.');
      await page.emulateMedia({ colorScheme: scheme });
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      const panel = await openFiles(page);
      await attach(panel, page, [note('notes.txt'), note('plan.txt')]);
      await add(panel).click();
      await expect(panel.getByTestId('file-row')).toHaveCount(2);
      await page.evaluate(async (path) => (await import(path)).rotateEpoch(), fixture);
      await expect(panel.getByText(text.filesResealing)).toHaveCount(2);
      await page.screenshot({ path: `${captures}/2293-resealing-${width}-${scheme}.png` });
    });

test('the Export panel lists every attachment with its outcome and saves an included one under its own name', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(
    async ({ fixture }) => (await import(fixture)).mount({ exportAttachments: true }),
    { fixture },
  );
  const host = page.locator('#ask-page-fixture');
  await expect(host.locator('.status')).toContainText('Live');
  await (await pageAction(host, 'Export page')).click();
  // The drawer is portaled out of the host.
  const panel = page.getByRole('region', { name: 'Export page' });
  const list = panel.getByRole('region', { name: text.exportAttachments });
  await expect(list.getByRole('listitem')).toHaveCount(4);
  // An included file offers a download; every other outcome says why and offers none.
  const row = (name: string) => list.getByRole('listitem').filter({ hasText: name });
  await expect(
    row('note.txt').getByRole('button', { name: text.attachmentDownload }),
  ).toBeEnabled();
  await expect(list.getByRole('button')).toHaveCount(1);
  await expect(list).toContainText(text.exportAttachmentState.missing);
  await expect(list).toContainText(text.exportAttachmentState.denied);
  await expect(list).toContainText(text.exportAttachmentState['too-large']);
  const pending = page.waitForEvent('download');
  await row('note.txt').getByRole('button', { name: text.attachmentDownload }).click();
  const received = await pending;
  expect(received.suggestedFilename()).toBe('note.txt');
  expect(readFileSync((await received.path())!).toString()).toBe('exported note');
  // Requesting the other files completes the copy: the included attachment counts too.
  await expect(panel.getByRole('status')).toContainText(text.exportPartial);
  for (const name of ['page.html', 'conversations.json', 'conversations.md', 'manifest.json']) {
    const next = page.waitForEvent('download');
    await panel.getByRole('button', { name: `Download ${name}`, exact: true }).click();
    await next;
  }
  await expect(panel.getByRole('status')).toContainText(text.exportRequested);
});
