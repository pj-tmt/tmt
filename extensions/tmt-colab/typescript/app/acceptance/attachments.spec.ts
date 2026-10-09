import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
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
