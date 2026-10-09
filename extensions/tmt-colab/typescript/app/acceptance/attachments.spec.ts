import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
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
    const added = colab(['share', 'link', 'add', created.pageId, '--yes']);
    const readerPath = added.readerPath as string;
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

    // Ending the link while a preview is shown disposes it: nothing of the read stays on screen.
    // The earlier check left the preview shown.
    await expect(reader.page.getByTestId('file-preview').locator('img')).toBeVisible();
    colab(['share', 'link', 'reset', created.pageId, added.linkId as string, '--yes']);
    await expect(reader.page.getByRole('heading', { name: 'Access ended', level: 2 })).toBeVisible({
      timeout: 60_000,
    });
    await expect(reader.page.locator('img[src^="data:"]')).toHaveCount(0);
    await expect(reader.page.getByTestId('file-row')).toHaveCount(0);
  });
});

test('a foreign write during an upload leaves a message attachment valid; a membership change does not', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'fence-author');
    const viewer = await pairBrowser(world, 'fence-viewer');
    const created = createPage(world, 'Fence', '<h1>Fence</h1>');
    const first = await openPage(door, author, created);
    const second = await openPage(door, viewer, created);
    await openChat(first);
    await openChat(second);
    const mine = first.getByTestId('chat-panel');
    const theirs = second.getByTestId('chat-panel');
    const big = {
      name: 'big.bin',
      mimeType: 'application/octet-stream',
      buffer: bytes(3 * 1024 * 1024, 21),
    };
    const chip = mine.getByTestId('attachment-chip');
    const sendBig = async (message: string) => {
      await mine.getByRole('combobox', { name: 'Message', exact: true }).fill(message);
      const chooser = first.waitForEvent('filechooser');
      await mine.getByRole('button', { name: text.attachFiles }).click();
      await (await chooser).setFiles([big]);
      await mine.getByRole('button', { name: text.askSend, exact: true }).click();
      await expect(chip).toHaveAttribute('data-state', 'uploading');
    };
    const colab = (args: string[], input?: string) =>
      run(world, world.binaries.colab, [...args, '--json'], input);

    // A foreign write (the page's source moves) while the upload runs: the message still
    // sends with its attachment and no "page changed" notice appears.
    await sendBig('Big one');
    colab(
      ['page', 'write', created.pageId, '--file', '-'],
      '<h1>Fence, moved by another writer</h1>',
    );
    await expect(chip).toHaveAttribute('data-state', 'uploading', { timeout: 1000 });
    await expect(mine.getByTestId('message-attachment')).toHaveCount(1, { timeout: 120_000 });
    await expect(first.getByText(text.attachAgainPageChanged)).toHaveCount(0);
    await expect(theirs.getByTestId('message-attachment')).toHaveCount(1, { timeout: 30_000 });

    // A membership change (the share mode) while it runs: that upload is no longer valid.
    await sendBig('Second one');
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    await expect(chip).toHaveAttribute('data-state', 'refused', { timeout: 120_000 });
    await expect(mine.getByTestId('message-attachment')).toHaveCount(1);
    await expect(theirs.getByTestId('message-attachment')).toHaveCount(1);
  });
});

test('export and attachment read return the same bytes as the browser, and say why they cannot', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'export-files-author');
    const created = createPage(world, 'Export files', '<h1>Export files</h1>');
    const first = await openPage(door, author, created);
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'])) as Record<string, unknown>;
    const out = fs.mkdtempSync(path.join(os.tmpdir(), 'colab-export-files-'));
    try {
      const notes = { name: 'notes.txt', mimeType: 'text/plain', buffer: bytes(40 * 1024, 5) };
      const picture = { name: 'dot.png', mimeType: 'image/png', buffer: PNG };
      const chat = {
        name: 'chat.bin',
        mimeType: 'application/octet-stream',
        buffer: bytes(3000, 7),
      };

      // Two document files through the Files panel and one message attachment through Chat.
      const files = await openFiles(first);
      const chooser = first.waitForEvent('filechooser');
      await files.getByRole('button', { name: text.attachFiles }).click();
      await (await chooser).setFiles([notes, picture]);
      await files.getByRole('button', { name: text.filesAdd, exact: true }).click();
      await expect(files.getByTestId('file-row')).toHaveCount(2, { timeout: 30_000 });
      await openChat(first);
      const panel = first.getByTestId('chat-panel');
      await panel.getByRole('combobox', { name: 'Message', exact: true }).fill('One file');
      const pick = first.waitForEvent('filechooser');
      await panel.getByRole('button', { name: text.attachFiles }).click();
      await (await pick).setFiles([chat]);
      await panel.getByRole('button', { name: text.askSend, exact: true }).click();
      await expect(panel.getByTestId('message-attachment')).toHaveCount(1, { timeout: 30_000 });
      const originals = new Map([notes, picture, chat].map((file) => [file.name, file.buffer]));

      // The CLI export lists and includes all three, byte for byte, in private files.
      const exported = colab(['export', created.pageId, '--dir', out]);
      const directory = exported.directory as string;
      const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
      type Row = {
        filename: string;
        source: string;
        state: string;
        file?: string;
        sha256?: string;
        reference: Record<string, unknown>;
      };
      const rows = manifest.attachments as Row[];
      expect(rows.map((row) => [row.filename, row.source, row.state]).sort()).toEqual(
        [
          ['chat.bin', 'message', 'included'],
          ['dot.png', 'document', 'included'],
          ['notes.txt', 'document', 'included'],
        ].sort(),
      );
      for (const row of rows) {
        const written = fs.readFileSync(path.join(directory, row.file!));
        expect(sha(written)).toBe(sha(originals.get(row.filename)!));
        expect(row.sha256).toBe(sha(written));
        expect(fs.statSync(path.join(directory, row.file!)).mode & 0o777).toBe(0o600);
      }
      expect(fs.statSync(path.join(directory, 'attachments')).mode & 0o777).toBe(0o700);

      // The browser's Export panel offers the same three files under their own names.
      await first.getByRole('button', { name: 'Export page', exact: true }).click();
      const exportPanel = first.getByRole('region', { name: 'Export page' });
      for (const file of [notes, picture, chat]) {
        const pending = first.waitForEvent('download');
        await exportPanel.getByRole('button', { name: `Download ${file.name}` }).click();
        const saved = await pending;
        expect(saved.suggestedFilename()).toBe(file.name);
        expect(sha(fs.readFileSync((await saved.path())!))).toBe(sha(file.buffer));
      }
      await exportPanel.getByRole('button', { name: 'Close export' }).click();

      // attachment read returns one file from its manifest reference, and only that file.
      const referenceOf = (name: string) => {
        const file = path.join(out, `${name}.reference.json`);
        fs.writeFileSync(
          file,
          JSON.stringify(rows.find((row) => row.filename === name)!.reference),
        );
        return file;
      };
      const read = (name: string) =>
        colab([
          'attachment',
          'read',
          created.pageId,
          '--reference',
          referenceOf(name),
          '--output',
          out,
        ]);
      const result = read('notes.txt');
      expect(sha(fs.readFileSync(path.join(result.directory as string, 'attachment.bin')))).toBe(
        sha(notes.buffer),
      );
      expect(JSON.stringify(result)).not.toContain(notes.buffer.toString('latin1').slice(0, 64));
      expect(fs.readdirSync(result.directory as string).sort()).toEqual([
        'attachment.bin',
        'manifest.json',
      ]);

      // The page's source moves: the old document reference is stale, a message's is not.
      run(
        world,
        world.binaries.colab,
        ['page', 'write', created.pageId, '--file', '-'],
        '<h1>Moved</h1>',
      );
      expect(() => read('notes.txt')).toThrow(/COLAB_STALE_BASE/);
      const chatAgain = read('chat.bin');
      expect(sha(fs.readFileSync(path.join(chatAgain.directory as string, 'attachment.bin')))).toBe(
        sha(chat.buffer),
      );
      expect(() =>
        run(world, world.binaries.colab, [
          'attachment',
          'read',
          created.pageId,
          '--reference',
          path.join(out, 'missing.json'),
        ]),
      ).toThrow();

      // With no serve, nothing can be read: every row is listed and none is disclosed.
      run(world, world.binaries.colab, ['stop']);
      const offline = colab(['export', created.pageId, '--dir', out]);
      const offlineManifest = JSON.parse(
        fs.readFileSync(path.join(offline.directory as string, 'manifest.json'), 'utf8'),
      );
      expect(
        (offlineManifest.attachments as Row[]).map((row) => [row.state, row.file, row.sha256]),
      ).toEqual(Array(3).fill(['unavailable', undefined, undefined]));
      expect(fs.existsSync(path.join(offline.directory as string, 'attachments'))).toBe(false);
      expect(() => read('chat.bin')).toThrow(/COLAB_UNAVAILABLE/);
    } finally {
      fs.rmSync(out, { recursive: true, force: true });
    }
  });
});
