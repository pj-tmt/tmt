import { pageAction } from '../test/page-actions.js';
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openReaderLink, pairBrowser, startDoor } from './harness/browser.js';
import { clientId, createPage, freePort, openChat, openPage, run } from './harness/ask.js';
import { captureResponsive } from './harness/captures.js';
import { until } from './harness/process.js';
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
      if (!reloaded) await captureResponsive(second, 'chat-attachments');

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

test('native attach lists a file for every device, reads back byte-equal, and refuses what it must', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'native-author');
    const viewer = await pairBrowser(world, 'native-viewer');
    const created = createPage(world, 'Native attach', '<h1>Native attach</h1>');
    const first = await openPage(door, author, created);
    const second = await openPage(door, viewer, created);
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'])) as Record<string, any>;
    const refused = (args: string[], code: RegExp) =>
      expect(() => run(world, world.binaries.colab, [...args, '--json'])).toThrow(code);
    // Every slot of the serve: its name and the files it still holds.
    const slots = () => {
      const found = fs
        .readdirSync(world.dataRoot, { recursive: true, encoding: 'utf8' })
        .filter((entry) => entry.endsWith('attach-slots'));
      expect(found).toHaveLength(1);
      const directory = path.join(world.dataRoot, found[0]!);
      return Object.fromEntries(
        fs.readdirSync(directory).map((name) => [name, fs.readdirSync(path.join(directory, name))]),
      );
    };
    // A finished slot keeps only its small answer: no plaintext, no ciphertext.
    const onlyAnswers = () => Object.values(slots()).every((files) => files.join() === 'slot.json');
    const input = fs.mkdtempSync(path.join(os.tmpdir(), 'colab-native-attach-'));
    const out = fs.mkdtempSync(path.join(os.tmpdir(), 'colab-native-read-'));
    try {
      await expect(
        first.frameLocator('iframe').getByRole('heading', { name: 'Native attach', exact: true }),
      ).toBeVisible();
      const notes = bytes(40 * 1024, 21);
      fs.writeFileSync(path.join(input, 'notes.txt'), notes);
      fs.writeFileSync(path.join(input, 'dot.png'), PNG);

      // The serve seals, uploads and lists the file; the reply carries the exact reference.
      const added = colab(['attachment', 'attach', created.pageId, path.join(input, 'notes.txt')]);
      expect(added.attachment).toMatchObject({
        filename: 'notes.txt',
        mediaType: 'text/plain',
        plaintextBytes: notes.length,
      });
      expect(Object.keys(slots())).toHaveLength(1);
      expect(onlyAnswers()).toBe(true);

      // Every device lists it and downloads the same bytes, live and without a reload.
      await openFiles(first);
      await expectFilesCount(first, 1);
      await expectFilesCount(second, 1);
      const rowsOn = (page: Page) =>
        page
          .getByTestId('files-panel')
          .or(page.locator('dialog[data-panel=files]'))
          .getByTestId('file-row');
      await openFiles(second);
      await expect(rowsOn(second)).toHaveCount(1, { timeout: 30_000 });
      const pending = second.waitForEvent('download');
      await rowsOn(second).first().getByRole('button', { name: text.attachmentDownload }).click();
      const saved = await pending;
      expect(saved.suggestedFilename()).toBe('notes.txt');
      expect(sha(fs.readFileSync((await saved.path())!))).toBe(sha(notes));

      // The reference it printed reads the same bytes back through the serve.
      const reference = path.join(input, 'reference.json');
      fs.writeFileSync(reference, JSON.stringify(added.attachment.reference));
      const read = colab([
        'attachment',
        'read',
        created.pageId,
        '--reference',
        reference,
        '--output',
        out,
      ]);
      expect(sha(fs.readFileSync(read.path as string))).toBe(sha(notes));

      // A picture takes its type from the extension and previews like a browser upload.
      colab(['attachment', 'attach', created.pageId, path.join(input, 'dot.png')]);
      await expect(rowsOn(second)).toHaveCount(2, { timeout: 30_000 });
      await expect(
        rowsOn(second)
          .filter({ hasText: 'dot.png' })
          .getByRole('button', { name: text.attachmentPreview }),
      ).toHaveCount(1);

      // A reply lost after the serve started: the CLI dies once the file is sealed and uploading,
      // the serve finishes on its own, and the explicit resume of that slot returns the same
      // attachment without a second upload.
      fs.writeFileSync(path.join(input, 'large.bin'), bytes(8 * 1024 * 1024, 3));
      const before = new Set(Object.keys(slots()));
      const child = spawn(
        world.binaries.colab,
        ['attachment', 'attach', created.pageId, path.join(input, 'large.bin'), '--json'],
        { env: world.env(), stdio: ['ignore', 'pipe', 'pipe'] },
      );
      let announced = '';
      let printed = '';
      child.stderr.on('data', (chunk) => (announced += chunk));
      child.stdout.on('data', (chunk) => (printed += chunk));
      const slotOf = () => /slot ([0-9a-f]{32})/.exec(announced)?.[1];
      await expect
        .poll(() => slotOf() ?? `no slot yet; stderr=${announced} stdout=${printed}`, {
          timeout: 30_000,
        })
        .toMatch(/^[0-9a-f]{32}$/);
      const slot = slotOf()!;
      expect(before.has(slot)).toBe(false);
      await expect
        .poll(() => (slots()[slot] ?? []).includes('object'), { timeout: 30_000 })
        .toBe(true);
      child.kill('SIGKILL');
      await expectFilesCount(second, 3, 120_000);
      const resumed = colab(['attachment', 'attach', created.pageId, '--resume', slot]);
      expect(resumed.attachment).toMatchObject({
        filename: 'large.bin',
        plaintextBytes: 8 * 1024 * 1024,
      });
      expect(resumed.slot).toBe(slot);
      // The same attachment again, not a second one, and it reads back byte for byte.
      await expect(rowsOn(second)).toHaveCount(3);
      fs.writeFileSync(reference, JSON.stringify(resumed.attachment.reference));
      const large = colab([
        'attachment',
        'read',
        created.pageId,
        '--reference',
        reference,
        '--output',
        out,
      ]);
      expect(sha(fs.readFileSync(large.path as string))).toBe(sha(bytes(8 * 1024 * 1024, 3)));
      expect(onlyAnswers()).toBe(true);

      // Refusals stage nothing and add no attachment.
      const staged = Object.keys(slots()).length;
      fs.symlinkSync(path.join(input, 'notes.txt'), path.join(input, 'link.txt'));
      fs.writeFileSync(path.join(input, 'big.bin'), Buffer.alloc(8 * 1024 * 1024 + 1));
      refused(
        ['attachment', 'attach', created.pageId, path.join(input, 'absent.txt')],
        /COLAB_INPUT_INVALID/,
      );
      refused(
        ['attachment', 'attach', created.pageId, path.join(input, 'link.txt')],
        /COLAB_INPUT_INVALID/,
      );
      refused(['attachment', 'attach', created.pageId, input], /COLAB_INPUT_INVALID/);
      refused(
        ['attachment', 'attach', created.pageId, path.join(input, 'big.bin')],
        /COLAB_CAPACITY/,
      );
      refused(
        [
          'attachment',
          'attach',
          '20000000-0000-4000-8000-0000000000aa',
          path.join(input, 'notes.txt'),
        ],
        /COLAB_PAGE_NOT_FOUND/,
      );
      refused(
        ['attachment', 'attach', created.pageId, '--resume', 'f'.repeat(32)],
        /COLAB_STATE_MISSING/,
      );
      // A slot belongs to its page: another page cannot resume it.
      const other = createPage(world, 'Other page', '<h1>Other page</h1>');
      refused(['attachment', 'attach', other.pageId, '--resume', slot], /COLAB_STATE_MISSING/);
      expect(Object.keys(slots())).toHaveLength(staged);
      await expect(rowsOn(second)).toHaveCount(3);

      // An archived page refuses before anything is staged.
      run(world, world.binaries.colab, ['archive', created.pageId]);
      refused(
        ['attachment', 'attach', created.pageId, path.join(input, 'notes.txt')],
        /COLAB_PAGE_INACTIVE/,
      );
      expect(Object.keys(slots())).toHaveLength(staged);
    } finally {
      fs.rmSync(input, { recursive: true, force: true });
      fs.rmSync(out, { recursive: true, force: true });
    }
  });
});

/** The Files control of the page header says how many files the page lists. */
async function expectFilesCount(page: Page, count: number, timeout = 30_000) {
  await expect(await pageAction(page, 'Files')).toHaveAccessibleName(`${text.files} (${count})`, {
    timeout,
  });
}
async function openFiles(page: Page) {
  const toggle = await pageAction(page, 'Files');
  await expect(toggle).toBeVisible({ timeout: 30_000 });
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
    await expect(await pageAction(second, 'Files')).toHaveAccessibleName('Files (0)');

    await panel.getByRole('button', { name: text.filesAdd, exact: true }).click();
    await expect(panel.getByTestId('file-row')).toHaveCount(3, { timeout: 30_000 });
    await expect(await pageAction(first, 'Files')).toHaveAccessibleName('Files (3)');
    await expect(await pageAction(second, 'Files')).toHaveAccessibleName('Files (3)', {
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
    await expect(await pageAction(reader.page, 'Files')).toHaveText(`${text.files} 3`, {
      timeout: 30_000,
    });
    await (await pageAction(reader.page, 'Files')).click();
    await check(reader.page, false);
    await expect(reader.page.getByRole('button', { name: text.attachFiles })).toHaveCount(0);

    // Removing a file removes its row for every device, live.
    await panel
      .getByTestId('file-row')
      .filter({ hasText: 'notes.txt' })
      .getByRole('button', { name: text.attachRemove })
      .click();
    await expect(panel.getByTestId('file-row')).toHaveCount(2, { timeout: 30_000 });
    await expect(await pageAction(second, 'Files')).toHaveAccessibleName('Files (2)', {
      timeout: 30_000,
    });
    await expect(await pageAction(reader.page, 'Files')).toHaveText(`${text.files} 2`, {
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
      await (await pageAction(first, 'Export page')).click();
      const exportPanel = first.getByRole('region', { name: 'Export page' });
      for (const file of [notes, picture, chat]) {
        const pending = first.waitForEvent('download');
        await exportPanel
          .getByRole('listitem')
          .filter({ hasText: file.name })
          .getByRole('button', { name: text.attachmentDownload })
          .click();
        const saved = await pending;
        expect(saved.suggestedFilename()).toBe(file.name);
        expect(sha(fs.readFileSync((await saved.path())!))).toBe(sha(file.buffer));
      }
      await first.locator('.page-drawer[data-panel="export"] .drawer-bar button').click();

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
      // The extension comes from the verified media type, never from the author's file name.
      expect(path.extname(result.path as string)).toBe('.txt');
      expect(sha(fs.readFileSync(result.path as string))).toBe(sha(notes.buffer));
      expect(JSON.stringify(result)).not.toContain(notes.buffer.toString('latin1').slice(0, 64));
      expect(fs.readdirSync(result.directory as string).sort()).toEqual(
        [path.basename(result.path as string), 'manifest.json'].sort(),
      );

      // The page's source moves: the old document reference is stale, a message's is not.
      run(
        world,
        world.binaries.colab,
        ['page', 'write', created.pageId, '--file', '-'],
        '<h1>Moved</h1>',
      );
      expect(() => read('notes.txt')).toThrow(/COLAB_STALE_BASE/);
      const chatAgain = read('chat.bin');
      expect(path.extname(chatAgain.path as string)).toBe('.bin');
      expect(sha(fs.readFileSync(chatAgain.path as string))).toBe(sha(chat.buffer));
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

test('revoking a device rekeys a native document attachment with one complete swap and exact browser and CLI bytes', async () => {
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'rekey-author');
    const viewer = await pairBrowser(world, 'rekey-viewer');
    const created = createPage(world, 'Rekey', '<h1>Rekey</h1>');
    const first = await openPage(door, author, created);
    const second = await openPage(door, viewer, created);
    type Reference = {
      kind: 'document-current';
      attachmentId: string;
      descriptorHash: string;
      contentRevision: string;
    };
    type Row = {
      attachmentId: string;
      filename: string;
      source: string;
      state: string;
      reference: Reference;
      file?: string;
    };
    type Slot = {
      replaces?: string;
      doneMs?: number;
      sealed?: { descriptor: { attachmentId: string; epoch: string; payloadSha256: string } };
    };
    const input = fs.mkdtempSync(path.join(world.root, 'rekey-'));
    const exportPage = () => {
      const result = JSON.parse(
        run(world, world.binaries.colab, ['export', created.pageId, '--dir', input, '--json']),
      ) as { directory: string };
      const manifest = JSON.parse(
        fs.readFileSync(path.join(result.directory, 'manifest.json'), 'utf8'),
      ) as { epoch: string; attachments: Row[] };
      return { ...manifest, directory: result.directory };
    };
    try {
      for (const page of [first, second]) {
        await expect(
          page.frameLocator('iframe').getByRole('heading', { name: 'Rekey', exact: true }),
        ).toBeVisible();
      }
      const notes = bytes(40 * 1024, 33);
      fs.writeFileSync(path.join(input, 'notes.txt'), notes);
      const added = JSON.parse(
        run(world, world.binaries.colab, [
          'attachment',
          'attach',
          created.pageId,
          path.join(input, 'notes.txt'),
          '--json',
        ]),
      ) as { attachment: { attachmentId: string; reference: Reference } };
      await openFiles(second);
      const rows = second
        .getByTestId('files-panel')
        .or(second.locator('dialog[data-panel=files]'))
        .getByTestId('file-row');
      await expect(rows).toHaveCount(1, { timeout: 30_000 });
      const before = exportPage();
      expect(before.attachments).toHaveLength(1);
      expect(before.attachments[0]).toMatchObject({
        attachmentId: added.attachment.attachmentId,
        source: 'document',
        state: 'included',
        reference: added.attachment.reference,
      });
      expect(fs.readFileSync(path.join(before.directory, before.attachments[0]!.file!))).toEqual(
        notes,
      );

      const paths = fs.readdirSync(world.dataRoot, { recursive: true, encoding: 'utf8' });
      const slotDirectories = paths.filter((entry) => entry.endsWith('attach-slots'));
      const databases = paths.filter((entry) => entry.endsWith('/space.db'));
      expect(slotDirectories).toHaveLength(1);
      expect(databases).toHaveLength(1);
      const records = () => {
        const directory = path.join(world.dataRoot, slotDirectories[0]!);
        return fs
          .readdirSync(directory)
          .map(
            (name) =>
              JSON.parse(fs.readFileSync(path.join(directory, name, 'slot.json'), 'utf8')) as Slot,
          );
      };
      const original = records().find(
        (record) => record.sealed?.descriptor.attachmentId === added.attachment.attachmentId,
      )!.sealed!.descriptor;
      expect(original.epoch).toBe(before.epoch);

      // Revoke an actually registered device through the real Remote door, rather than a
      // sharing-mode toggle. Its callback narrows this page before the serve rekeys the file.
      run(world, world.binaries.remote, [
        'devices',
        'revoke',
        clientId(world, author.name),
        '--json',
      ]);

      // Wait on the published reference, not on a staging record or a fixed delay. Every
      // sampled snapshot must contain one complete old or new reference, never neither/both.
      const observations: { epoch: string; reference: Reference }[] = [];
      let after = before;
      await until(
        () => {
          after = exportPage();
          expect(after.attachments).toHaveLength(1);
          const row = after.attachments[0]!;
          expect(row).toMatchObject({ filename: 'notes.txt', source: 'document' });
          expect(row.reference).toMatchObject({
            kind: 'document-current',
            attachmentId: row.attachmentId,
          });
          expect(row.reference.descriptorHash).toMatch(/^[a-f0-9]{64}$/);
          expect(row.reference.contentRevision).toMatch(/^v1:/);
          observations.push({ epoch: after.epoch, reference: row.reference });
          return row.attachmentId !== added.attachment.attachmentId && row.state === 'included';
        },
        'the published document reference moved to the new epoch',
        90_000,
      );
      expect(BigInt(after.epoch)).toBe(BigInt(before.epoch) + 1n);
      const current = after.attachments[0]!;
      expect(current.reference.descriptorHash).not.toBe(added.attachment.reference.descriptorHash);
      expect(fs.readFileSync(path.join(after.directory, current.file!))).toEqual(notes);
      await until(
        () => records().some((record) => record.replaces && record.doneMs),
        'the rekey slot settled',
      );
      const replacements = records().filter((record) => record.replaces);
      expect(replacements).toHaveLength(1);
      const swapped = replacements[0]!;
      expect(swapped.replaces).toBe(added.attachment.attachmentId);
      expect(swapped.sealed!.descriptor).toMatchObject({
        attachmentId: current.attachmentId,
        epoch: after.epoch,
      });
      expect(swapped.sealed!.descriptor.payloadSha256).not.toBe(original.payloadSha256);
      for (const observation of observations) {
        const expected =
          observation.reference.attachmentId === added.attachment.attachmentId
            ? added.attachment.reference
            : current.reference;
        expect(observation.reference.attachmentId).toBe(expected.attachmentId);
        expect(observation.reference.descriptorHash).toBe(expected.descriptorHash);
      }

      // Independent, read-only durable evidence: exactly one content update in the new epoch
      // made the swap. Separate remove/add writes or repeated swaps cannot pass this check.
      const swaps = () =>
        Number(
          execFileSync(
            'sqlite3',
            [
              '-readonly',
              path.join(world.dataRoot, databases[0]!),
              `SELECT count(*) FROM receipts WHERE page='${created.pageId}' AND epoch='${after.epoch}' AND namespace='content'`,
            ],
            { encoding: 'utf8' },
          ).trim(),
        );
      expect(swaps()).toBe(1);

      // The surviving browser opens the new epoch and downloads the re-sealed object.
      await second.reload();
      await openFiles(second);
      await expect(rows).toHaveCount(1, { timeout: 30_000 });
      await expect(rows.first()).not.toContainText(text.filesResealing);
      const download = second.waitForEvent('download', { timeout: 30_000 });
      await rows.first().getByRole('button', { name: text.attachmentDownload }).click();
      const saved = await download;
      expect(saved.suggestedFilename()).toBe('notes.txt');
      expect(fs.readFileSync((await saved.path())!)).toEqual(notes);

      // The CLI reads the same new reference byte for byte; the superseded one is stale.
      const reference = path.join(input, 'reference.json');
      fs.writeFileSync(reference, JSON.stringify(current.reference));
      const readArgs = [
        'attachment',
        'read',
        created.pageId,
        '--reference',
        reference,
        '--output',
        input,
        '--json',
      ];
      const read = JSON.parse(run(world, world.binaries.colab, readArgs)) as {
        directory: string;
        path: string;
      };
      expect(fs.readFileSync(read.path)).toEqual(notes);
      fs.writeFileSync(reference, JSON.stringify(added.attachment.reference));
      expect(() => run(world, world.binaries.colab, readArgs)).toThrow(/COLAB_STALE_BASE/);
      const reopened = exportPage();
      expect(reopened.attachments).toHaveLength(1);
      expect(reopened.attachments[0]).toMatchObject({
        attachmentId: current.attachmentId,
        state: 'included',
        reference: current.reference,
      });
      expect(swaps()).toBe(1);
      expect(records().filter((record) => record.replaces)).toHaveLength(1);
    } finally {
      fs.rmSync(input, { recursive: true, force: true });
    }
  });
});
