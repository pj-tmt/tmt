import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { openReaderLink, pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openChat, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import { text } from '../src/strings.js';

test.afterEach(disposeActiveWorlds);

const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);
const bytes = (size: number, seed: number) =>
  Buffer.from(Uint8Array.from({ length: size }, (_, i) => (i * 131 + seed) & 0xff));
const sha = (value: Buffer) => createHash('sha256').update(value).digest('hex');

test('files attached in Chat reach a second paired device, preview and download exactly, and survive reload', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'attach-author');
    const viewer = await pairBrowser(world, 'attach-viewer');
    const created = createPage(world, 'Attachments', '<h1>Attachments</h1>');
    const first = await openPage(door, author, created);
    const second = await openPage(door, viewer, created);
    await expect(
      first.frameLocator('iframe').getByRole('heading', { name: 'Attachments', exact: true }),
    ).toBeVisible();

    // Several 32 KiB parts, an ordinary file and bytes that merely look like video.
    const notes = { name: 'notes.txt', mimeType: 'text/plain', buffer: bytes(100 * 1024, 3) };
    const clip = {
      name: 'clip.mp4',
      mimeType: 'video/mp4',
      buffer: Buffer.concat([
        Buffer.from([0, 0, 0, 0x18]),
        Buffer.from('ftypmp42'),
        bytes(200 * 1024, 9),
      ]),
    };
    const picture = { name: 'dot.png', mimeType: 'image/png', buffer: PNG };

    await openChat(first);
    const panel = first.getByTestId('chat-panel');
    const input = panel.getByRole('combobox', { name: 'Message', exact: true });
    await input.fill('Three files');
    const chooser = first.waitForEvent('filechooser');
    await panel.getByRole('button', { name: text.attachFiles }).click();
    await (await chooser).setFiles([notes, clip, picture]);
    await expect(panel.getByTestId('attachment-chip')).toHaveCount(3);
    // Choosing stored nothing: the page's other device sees no message yet.
    await openChat(second);
    await expect(second.getByTestId('chat-panel').getByTestId('chat-thread')).toHaveCount(0);

    await panel.getByRole('button', { name: text.askSend, exact: true }).click();
    await expect(panel.getByTestId('message-attachment')).toHaveCount(3, { timeout: 30_000 });
    await expect(panel.getByTestId('attachment-chip')).toHaveCount(0);

    const there = second.getByTestId('chat-panel');
    await expect(there.getByTestId('message-attachment')).toHaveCount(3, { timeout: 30_000 });
    for (const reloaded of [false, true]) {
      if (reloaded) {
        await second.reload();
        await openChat(second);
        await expect(there.getByTestId('message-attachment')).toHaveCount(3, { timeout: 30_000 });
      }
      const rows = there.getByTestId('message-attachment');
      const row = (name: string) => rows.filter({ hasText: name });

      // Only the raster offers a preview, shown from a data URL after an explicit click.
      await expect(
        row('clip.mp4').getByRole('button', { name: text.attachmentPreview }),
      ).toHaveCount(0);
      await expect(
        row('notes.txt').getByRole('button', { name: text.attachmentPreview }),
      ).toHaveCount(0);
      await row('dot.png').getByRole('button', { name: text.attachmentPreview }).click();
      await expect(row('dot.png').locator('img')).toHaveAttribute(
        'src',
        /^data:image\/png;base64,/,
      );
      await expect(there.locator('video, audio, object, embed')).toHaveCount(0);

      for (const file of [notes, clip, picture]) {
        const download = second.waitForEvent('download');
        await row(file.name).getByRole('button', { name: text.attachmentDownload }).click();
        const saved = await download;
        expect(saved.suggestedFilename()).toBe(file.name);
        expect(sha(fs.readFileSync((await saved.path())!))).toBe(sha(file.buffer));
      }
    }
  });
});

async function openFiles(page: Page) {
  const toggle = page.getByTestId('files-toggle');
  const menu = page.getByRole('button', { name: 'More page actions' });
  // A reloaded page needs a moment before either control exists.
  await expect(toggle.or(menu)).toBeVisible({ timeout: 30_000 });
  if (!(await toggle.isVisible())) await menu.click();
  await toggle.click();
  return page.getByTestId('files-panel');
}

test('files added to the page reach another device and a read-only link byte-identically, and removal follows', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'files-author');
    const viewer = await pairBrowser(world, 'files-viewer');
    const created = createPage(world, 'Page files', '<h1>Page files</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'])) as Record<string, unknown>;
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    const readerPath = colab(['share', 'link', 'add', created.pageId, '--yes'])
      .readerPath as string;
    const first = await openPage(door, author, created);
    const second = await openPage(door, viewer, created);
    await expect(
      first.frameLocator('iframe').getByRole('heading', { name: 'Page files', exact: true }),
    ).toBeVisible();

    const notes = { name: 'notes.txt', mimeType: 'text/plain', buffer: bytes(100 * 1024, 5) };
    const clip = {
      name: 'clip.mp4',
      mimeType: 'video/mp4',
      buffer: Buffer.concat([
        Buffer.from([0, 0, 0, 0x18]),
        Buffer.from('ftypmp42'),
        bytes(200 * 1024, 11),
      ]),
    };
    const picture = { name: 'dot.png', mimeType: 'image/png', buffer: PNG };
    const all = [notes, clip, picture];

    const panel = await openFiles(first);
    await expect(panel.getByText(text.filesEmpty)).toBeVisible();
    const chooser = first.waitForEvent('filechooser');
    await panel.getByRole('button', { name: text.attachFiles }).click();
    await (await chooser).setFiles(all);
    await expect(panel.getByTestId('attachment-chip')).toHaveCount(3);
    // Choosing stored and wrote nothing: the other device still shows no files.
    await expect(second.getByTestId('files-toggle')).toHaveText(text.files);

    await panel.getByRole('button', { name: text.filesAdd, exact: true }).click();
    await expect(panel.getByTestId('file-row')).toHaveCount(3, { timeout: 30_000 });
    await expect(first.getByTestId('files-toggle')).toHaveText(`${text.files} 3`);
    await expect(second.getByTestId('files-toggle')).toHaveText(`${text.files} 3`, {
      timeout: 30_000,
    });

    const check = async (page: Page, writer: boolean) => {
      const rows = page
        .getByTestId('files-panel')
        .or(page.locator('dialog[data-panel=files]'))
        .getByTestId('file-row');
      const row = (name: string) => rows.filter({ hasText: name });
      await expect(rows).toHaveCount(3, { timeout: 30_000 });
      await expect(
        row('clip.mp4').getByRole('button', { name: text.attachmentPreview }),
      ).toHaveCount(0);
      await row('dot.png').getByRole('button', { name: text.attachmentPreview }).click();
      await expect(page.getByTestId('file-preview').locator('img')).toHaveAttribute(
        'src',
        /^data:image\/png;base64,/,
      );
      await expect(page.locator('video, audio, object, embed')).toHaveCount(0);
      for (const file of all) {
        const download = page.waitForEvent('download');
        await row(file.name).getByRole('button', { name: text.attachmentDownload }).click();
        const saved = await download;
        expect(saved.suggestedFilename()).toBe(file.name);
        expect(sha(fs.readFileSync((await saved.path())!))).toBe(sha(file.buffer));
      }
      await expect(page.getByRole('button', { name: text.attachRemove })).toHaveCount(
        writer ? 3 : 0,
      );
    };

    await openFiles(second);
    await check(second, true);
    await second.reload();
    await openFiles(second);
    await check(second, true);

    const reader = await openReaderLink(world, door, readerPath, 'files-reader');
    await expect(reader.page.getByTestId('files-toggle')).toHaveText(`${text.files} 3`, {
      timeout: 30_000,
    });
    await reader.page.getByTestId('files-toggle').click();
    await check(reader.page, false);
    await expect(reader.page.getByRole('button', { name: text.attachFiles })).toHaveCount(0);

    // Removing a file removes its row for every device, live.
    await panel
      .getByTestId('file-row')
      .filter({ hasText: 'notes.txt' })
      .getByRole('button', { name: text.attachRemove })
      .click();
    await expect(panel.getByTestId('file-row')).toHaveCount(2, { timeout: 30_000 });
    await expect(second.getByTestId('files-toggle')).toHaveText(`${text.files} 2`, {
      timeout: 30_000,
    });
    await expect(reader.page.getByTestId('files-toggle')).toHaveText(`${text.files} 2`, {
      timeout: 30_000,
    });
  });
});
