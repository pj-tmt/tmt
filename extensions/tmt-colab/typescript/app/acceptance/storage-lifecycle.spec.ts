// Product acceptance of the local attachment lifecycle (#1857) on the three shipped binaries.
// Each case states what it adds to the matrix in references/acceptance.md; the backend contract
// and the native lifecycle policy are cited there, not repeated.
import { pageAction } from '../test/page-actions.js';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openReaderLink, pairBrowser, startDoor } from './harness/browser.js';
import {
  annotationInput,
  clientId,
  createPage,
  freePort,
  openChat,
  openPage,
  run,
  selectInRenderer,
} from './harness/ask.js';
import { captureResponsive } from './harness/captures.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import { text } from '../src/strings.js';

test.afterEach(disposeActiveWorlds);

const sha = (value: Buffer) => createHash('sha256').update(value).digest('hex');
const bytes = (size: number, seed: number) =>
  Buffer.from(Uint8Array.from({ length: size }, (_, i) => (i * 131 + seed) & 0xff));
/** A real picture of the given type, drawn and encoded by the browser under test. */
async function raster(page: Page, type: 'image/png' | 'image/jpeg' | 'image/webp') {
  const encoded = await page.evaluate((mime) => {
    const canvas = document.createElement('canvas');
    canvas.width = 96;
    canvas.height = 64;
    const context = canvas.getContext('2d')!;
    const gradient = context.createLinearGradient(0, 0, 96, 64);
    gradient.addColorStop(0, '#2b6cb0');
    gradient.addColorStop(1, '#d69e2e');
    context.fillStyle = gradient;
    context.fillRect(0, 0, 96, 64);
    return canvas.toDataURL(mime, 0.9).split(',')[1]!;
  }, type);
  return Buffer.from(encoded, 'base64');
}
const CLIP = Buffer.concat([
  Buffer.from([0, 0, 0, 0x18]),
  Buffer.from('ftypmp42'),
  bytes(30 * 1024, 9),
]);

test('annotation attachments: PNG, JPEG and WebP preview, video is download-only, and the CSP never widens', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'life-annotation-author');
    const viewer = await pairBrowser(world, 'life-annotation-viewer');
    const created = createPage(
      world,
      'Annotation files',
      '<h1>Annotation files</h1><p id="quote">Discuss this exact quote.</p>',
    );
    // Every download URL the parent creates must be revoked again: none outlives its hand-off.
    await viewer.context.addInitScript(() => {
      if (window !== window.top) return;
      const urls = { created: [] as string[], revoked: [] as string[] };
      Object.defineProperty(window, 'downloadUrls', { value: urls });
      const create = URL.createObjectURL.bind(URL);
      const revoke = URL.revokeObjectURL.bind(URL);
      URL.createObjectURL = (blob) => {
        const url = create(blob);
        urls.created.push(url);
        return url;
      };
      URL.revokeObjectURL = (url) => {
        urls.revoked.push(url);
        revoke(url);
      };
    });
    const first = await openPage(door, author, created);
    const second = await openPage(door, viewer, created);
    const policy = async (page: Page) =>
      (await page.context().request.get(page.url())).headers()['content-security-policy'];
    const before = await policy(first);
    expect(before).toContain("script-src 'self'; style-src 'self'");

    const files = [
      { name: 'dot.png', mimeType: 'image/png', buffer: await raster(first, 'image/png') },
      { name: 'dot.jpg', mimeType: 'image/jpeg', buffer: await raster(first, 'image/jpeg') },
      { name: 'dot.webp', mimeType: 'image/webp', buffer: await raster(first, 'image/webp') },
      { name: 'clip.mp4', mimeType: 'video/mp4', buffer: CLIP },
    ];
    await selectInRenderer(first, '#quote');
    await first.getByTestId('selection-ask').click();
    const composer = first.locator('.annotation-new');
    const input = await annotationInput(composer, '');
    await input.fill('Four files on this quote');
    const chooser = first.waitForEvent('filechooser');
    await composer.getByRole('button', { name: text.attachFiles }).click();
    await (await chooser).setFiles(files);
    await expect(composer.getByTestId('attachment-chip')).toHaveCount(4);
    await input.press('Enter');

    await (await pageAction(second, 'Comments')).click();
    await second.getByTestId('annotation-row').click();
    const thread = second.getByTestId('comment-thread').first();
    const rows = thread.getByTestId('message-attachment');
    await expect(rows).toHaveCount(4, { timeout: 60_000 });
    for (const file of files) {
      const row = rows.filter({ hasText: file.name });
      const previewable = file.mimeType.startsWith('image/');
      await expect(row.getByRole('button', { name: text.attachmentPreview })).toHaveCount(
        previewable ? 1 : 0,
      );
      if (previewable) {
        await row.getByRole('button', { name: text.attachmentPreview }).click();
        await expect(row.locator('img')).toHaveAttribute(
          'src',
          new RegExp(`^data:${file.mimeType};base64,`),
        );
        // The browser decodes the preview: it is a real picture, not just a well-formed URL.
        await expect
          .poll(() => row.locator('img').evaluate((img: HTMLImageElement) => img.naturalWidth))
          .toBeGreaterThan(0);
      }
      const download = second.waitForEvent('download');
      await row.getByRole('button', { name: text.attachmentDownload }).click();
      const saved = await download;
      expect(saved.suggestedFilename()).toBe(file.name);
      expect(sha(fs.readFileSync((await saved.path())!))).toBe(sha(file.buffer));
    }
    await captureResponsive(second, 'thread-attachments');
    // Nothing embeds active content or video, and attachments left the policy as it was.
    await expect(second.locator('video, audio, object, embed, iframe[src^="blob:"]')).toHaveCount(
      0,
    );
    await expect
      .poll(() =>
        second.evaluate(() => {
          const u = (
            window as unknown as { downloadUrls: { created: string[]; revoked: string[] } }
          ).downloadUrls;
          return u.created.length >= 4 && u.created.every((url) => u.revoked.includes(url));
        }),
      )
      .toBe(true);
    expect(await policy(second)).toBe(before);
    expect(before).not.toContain('blob:');
    expect(before).not.toContain('unsafe-inline');
  });
});

async function openFiles(page: Page) {
  const toggle = await pageAction(page, 'Files');
  await expect(toggle).toBeVisible({ timeout: 30_000 });
  await toggle.click();
  return page.getByTestId('files-panel').or(page.locator('dialog[data-panel=files]'));
}
async function downloaded(page: Page, row: ReturnType<Page['locator']>, name: string) {
  const pending = page.waitForEvent('download');
  await row.getByRole('button', { name: text.attachmentDownload }).click();
  const saved = await pending;
  expect(saved.suggestedFilename()).toBe(name);
  return sha(fs.readFileSync((await saved.path())!));
}

test('principals: owner, a second paired device and a read-only link read; a revoked device and a removed link cannot', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const owner = await pairBrowser(world, 'life-principal-owner');
    const peer = await pairBrowser(world, 'life-principal-peer');
    const created = createPage(world, 'Principals', '<h1>Principals</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'])) as Record<string, unknown>;
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    const link = colab(['share', 'link', 'add', created.pageId, '--yes']);
    const first = await openPage(door, owner, created);
    const second = await openPage(door, peer, created);
    const note = { name: 'notes.txt', mimeType: 'text/plain', buffer: bytes(20 * 1024, 5) };

    const panel = await openFiles(first);
    const chooser = first.waitForEvent('filechooser');
    await panel.getByRole('button', { name: text.attachFiles }).click();
    await (await chooser).setFiles([note]);
    await panel.getByRole('button', { name: text.filesAdd, exact: true }).click();
    await expect(panel.getByTestId('file-row')).toHaveCount(1, { timeout: 30_000 });

    await expect(panel.getByText(text.filesAdding)).toHaveCount(0, { timeout: 30_000 });
    await captureResponsive(first, 'files');
    await (await pageAction(first, 'Export page')).click();
    await expect(
      first
        .getByRole('region', { name: 'Export page' })
        .getByRole('listitem')
        .filter({ hasText: note.name }),
    ).toHaveCount(1, { timeout: 30_000 });
    await captureResponsive(first, 'export');
    // The drawer's own close control: it sits in the dialog header, outside the export region.
    await first.getByRole('button', { name: 'Close Export page', exact: true }).click();
    await openFiles(first);

    // Owner, second paired device and the read-only link all read the same bytes.
    const rowsOf = (page: Page) =>
      page
        .getByTestId('files-panel')
        .or(page.locator('dialog[data-panel=files]'))
        .getByTestId('file-row');
    expect(await downloaded(first, rowsOf(first).first(), note.name)).toBe(sha(note.buffer));
    await openFiles(second);
    await expect(rowsOf(second)).toHaveCount(1, { timeout: 30_000 });
    expect(await downloaded(second, rowsOf(second).first(), note.name)).toBe(sha(note.buffer));
    const reader = await openReaderLink(world, door, link.readerPath as string, 'life-reader');
    await (await pageAction(reader.page, 'Files')).click();
    await expect(rowsOf(reader.page)).toHaveCount(1, { timeout: 30_000 });
    await expect(reader.page.getByRole('button', { name: text.attachRemove })).toHaveCount(0);
    expect(await downloaded(reader.page, rowsOf(reader.page).first(), note.name)).toBe(
      sha(note.buffer),
    );

    // A revoked paired device reads nothing after it reloads.
    run(world, world.binaries.remote, [
      'devices',
      'revoke',
      clientId(world, 'life-principal-peer'),
    ]);
    await second.reload();
    await expect(second.getByRole('heading', { name: 'Pair this browser first' })).toBeVisible({
      timeout: 30_000,
    });
    await expect(second.getByTestId('file-row')).toHaveCount(0);
    await expect(await pageAction(second, 'Files')).toHaveCount(0);

    // Removing the link ends the reader.
    colab(['share', 'link', 'remove', created.pageId, link.linkId as string, '--yes']);
    await expect(reader.page.getByRole('heading', { name: 'Access ended', level: 2 })).toBeVisible({
      timeout: 60_000,
    });
    await expect(reader.page.getByTestId('file-row')).toHaveCount(0);
    // Removing the link advanced the epoch: the owner's live session stops (STALE_EPOCH) and,
    // reopened, still reads the file sealed under the earlier epoch.
    await expect(first.getByRole('heading', { name: 'Preview stopped' })).toBeVisible({
      timeout: 60_000,
    });
    await first.reload();
    await openFiles(first);
    await expect(rowsOf(first)).toHaveCount(1, { timeout: 30_000 });
    expect(await downloaded(first, rowsOf(first).first(), note.name)).toBe(sha(note.buffer));
  });
});

test('cross-resource refusal and deletion: another page, a forged hash and a deleted page disclose nothing and leave no temporary files', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'life-delete-author');
    const mine = createPage(world, 'Mine', '<h1>Mine</h1>');
    const other = createPage(world, 'Other', '<h1>Other</h1>');
    const first = await openPage(door, author, mine);
    const exe = world.binaries.colab;
    const colab = (args: string[]) => JSON.parse(run(world, exe, [...args, '--json']));
    const out = fs.mkdtempSync(path.join(os.tmpdir(), 'colab-lifecycle-'));
    try {
      const note = { name: 'notes.txt', mimeType: 'text/plain', buffer: bytes(8 * 1024, 5) };
      const chat = {
        name: 'chat.bin',
        mimeType: 'application/octet-stream',
        buffer: bytes(3000, 7),
      };
      const panel = await openFiles(first);
      const chooser = first.waitForEvent('filechooser');
      await panel.getByRole('button', { name: text.attachFiles }).click();
      await (await chooser).setFiles([note]);
      await panel.getByRole('button', { name: text.filesAdd, exact: true }).click();
      await expect(panel.getByTestId('file-row')).toHaveCount(1, { timeout: 30_000 });
      await openChat(first);
      const chatPanel = first.getByTestId('chat-panel');
      await chatPanel.getByRole('combobox', { name: 'Message', exact: true }).fill('One file');
      const pick = first.waitForEvent('filechooser');
      await chatPanel.getByRole('button', { name: text.attachFiles }).click();
      await (await pick).setFiles([chat]);
      await chatPanel.getByRole('button', { name: text.askSend, exact: true }).click();
      await expect(chatPanel.getByTestId('message-attachment')).toHaveCount(1, { timeout: 30_000 });

      const manifest = JSON.parse(
        fs.readFileSync(
          path.join(colab(['export', mine.pageId, '--dir', out]).directory, 'manifest.json'),
          'utf8',
        ),
      );
      const rows = manifest.attachments as {
        filename: string;
        reference: Record<string, unknown>;
      }[];
      expect(rows.map((row) => row.filename).sort()).toEqual(['chat.bin', 'notes.txt']);
      const reference = (
        name: string,
        edit: (value: Record<string, unknown>) => void = () => {},
      ) => {
        const value = structuredClone(rows.find((row) => row.filename === name)!.reference);
        edit(value);
        const file = path.join(out, `${name}-${Math.random().toString(16).slice(2)}.json`);
        fs.writeFileSync(file, JSON.stringify(value));
        return file;
      };
      const read = (page: string, file: string) => [
        'attachment',
        'read',
        page,
        '--reference',
        file,
        '--output',
        out,
      ];
      const refusedWith = (args: string[], code: RegExp) => {
        let failure = '';
        try {
          run(world, exe, [...args, '--json']);
        } catch (error) {
          failure = (error as Error).message;
        }
        expect(failure).toMatch(code);
      };
      const dirs = () =>
        fs
          .readdirSync(out)
          .filter((name) => !name.endsWith('.json'))
          .sort();

      // Positive control: each reference reads its own bytes from its own page.
      for (const file of [note, chat]) {
        const result = colab(read(mine.pageId, reference(file.name)));
        expect(sha(fs.readFileSync(path.join(result.directory, 'attachment.bin')))).toBe(
          sha(file.buffer),
        );
      }
      const kept = dirs();
      // Another page (a document reference is stale there, a message is not found), a forged
      // descriptor hash and a forged message ID read nothing; each refusal has its own code.
      refusedWith(read(other.pageId, reference('notes.txt')), /COLAB_STALE_BASE/);
      refusedWith(read(other.pageId, reference('chat.bin')), /COLAB_STATE_MISSING/);
      refusedWith(
        read(
          mine.pageId,
          reference('notes.txt', (v) => (v.descriptorHash = 'f'.repeat(64))),
        ),
        /COLAB_INPUT_INVALID/,
      );
      refusedWith(
        read(
          mine.pageId,
          reference('chat.bin', (v) => (v.messageId = '00000000-0000-4000-8000-0000000000aa')),
        ),
        /COLAB_STATE_MISSING/,
      );
      expect(dirs()).toEqual(kept);

      // Deleting the page: nothing is exported or read, and the refusals create nothing.
      const live = reference('notes.txt');
      colab(['delete', mine.pageId, '--yes']);
      refusedWith(['export', mine.pageId, '--dir', out], /COLAB_PAGE_DELETED/);
      refusedWith(read(mine.pageId, live), /COLAB_PAGE_DELETED/);
      expect(dirs()).toEqual(kept);
      expect(fs.readdirSync(out).filter((name) => name.startsWith('.tmt-colab-export-'))).toEqual(
        [],
      );
      // The other page is untouched by the deletion.
      expect(colab(['export', other.pageId, '--dir', out]).directory).toBeTruthy();
    } finally {
      fs.rmSync(out, { recursive: true, force: true });
    }
  });
});
