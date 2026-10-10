import { pageAction } from '../test/page-actions.js';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { expect, test, type Locator } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { composeChat, createPage, freePort, openChat, openPage, run } from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import { text } from '../src/strings.js';

test.afterEach(disposeActiveWorlds);

const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);
const sha = (value: Buffer) => createHash('sha256').update(value).digest('hex');
const notes = Buffer.from(Uint8Array.from({ length: 40 * 1024 }, (_, i) => (i * 131 + 5) & 0xff));

/** A real DragEvent / ClipboardEvent carrying one File, as the browser delivers a drop or a paste. */
async function give(
  target: Locator,
  kind: 'drop' | 'paste',
  name: string,
  type: string,
  b64: string,
) {
  await target.evaluate(
    (node, [eventKind, fileName, mediaType, data]) => {
      const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
      const transfer = new DataTransfer();
      transfer.items.add(new File([bytes], fileName, { type: mediaType }));
      const init = { bubbles: true, cancelable: true };
      node.dispatchEvent(
        eventKind === 'drop'
          ? new DragEvent('drop', { ...init, dataTransfer: transfer })
          : new ClipboardEvent('paste', { ...init, clipboardData: transfer }),
      );
    },
    [kind, name, type, b64] as const,
  );
}

// #2464: a person drops one file and pastes one image into a Chat message for an agent; the
// agent's received request lists both, and one documented command writes byte-identical files.
test('an agent sees the files of a Chat message in its request and reads them by the listed IDs', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('files-agent');
    const author = await pairBrowser(world, 'files-author', { talk: true });
    const created = createPage(world, 'Agent files', '<h1>Agent files</h1>');
    const page = await openPage(door, author, created);
    const out = fs.mkdtempSync(path.join(os.tmpdir(), 'colab-agent-files-'));
    try {
      const composed = await composeChat(page, agent, 'Summarize these files');
      const panel = page.getByTestId('chat-panel');
      const compose = panel.getByTestId('annotation-compose');
      await give(compose, 'drop', 'notes "v2".txt', 'text/plain', notes.toString('base64'));
      await give(compose, 'paste', 'dot.png', 'image/png', PNG.toString('base64'));
      await expect(panel.getByTestId('attachment-chip')).toHaveCount(2);
      await panel.getByRole('button', { name: text.askSend, exact: true }).click();
      await expect(panel.getByTestId('message-attachment')).toHaveCount(2, { timeout: 30_000 });
      await until(() => agent.received().length === 1, 'the turn reached the agent');
      const request = composed.delivered();

      // The request lists both files by metadata only, with the quoted name and the one command.
      const lines = request.split('\n');
      const start = lines.indexOf('Attachments:');
      expect(start).toBeGreaterThan(0);
      const listed = lines.slice(start + 1, start + 3);
      expect(listed[0]).toMatch(
        /^- [0-9a-f]{8} "notes \\"v2\\"\.txt" \(application\/octet-stream, 40960 bytes\)$/,
      );
      expect(listed[1]).toMatch(/^- [0-9a-f]{8} "dot\.png" \(image\/png, \d+ bytes\)$/);
      expect(lines[start + 3]).toBe(`Read one: tmt colab attachment read ${created.pageId} <id>`);
      expect(request).not.toContain(notes.toString('latin1').slice(0, 64));

      const colab = (args: string[]) =>
        JSON.parse(run(world, world.binaries.colab, [...args, '--json'])) as Record<
          string,
          unknown
        >;
      const shortIds = listed.map((line) => line.split(' ')[1]);
      const read = (id: string) => {
        const result = colab(['attachment', 'read', created.pageId, id, '--output', out]);
        return fs.readFileSync(result.path as string);
      };
      expect(sha(read(shortIds[0]))).toBe(sha(notes));
      expect(sha(read(shortIds[1]))).toBe(sha(PNG));

      // `threads` finds the same files, with their full IDs, for an older message.
      const view = colab(['threads', created.pageId]) as unknown as {
        threads: { comments: { attachments?: { id: string; name: string }[] }[] }[];
      };
      const files = view.threads.flatMap((t) => t.comments.flatMap((c) => c.attachments ?? []));
      expect(files.map((file) => file.name).sort()).toEqual(['dot.png', 'notes "v2".txt']);
      expect(
        files.every((file) => file.id.startsWith(shortIds[0]) || file.id.startsWith(shortIds[1])),
      ).toBe(true);
      const dot = files.find((file) => file.name === 'dot.png')!;
      expect(sha(read(dot.id))).toBe(sha(PNG));

      // The browser's export and the CLI's name the same files in the same bytes.
      const exported = colab(['export', created.pageId, '--dir', out]);
      await (await pageAction(page, 'Export page')).click();
      const exportPanel = page.getByRole('region', { name: 'Export page' });
      await expect(exportPanel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
      for (const name of ['conversations.json', 'conversations.md']) {
        const pending = page.waitForEvent('download');
        await exportPanel.getByRole('button', { name: new RegExp(`Download ${name}`) }).click();
        const saved = await pending;
        const native = fs.readFileSync(path.join(exported.directory as string, name));
        expect(fs.readFileSync((await saved.path())!).equals(native), name).toBe(true);
      }
      expect(
        fs.readFileSync(path.join(exported.directory as string, 'conversations.md'), 'utf8'),
      ).toContain('- Attachment: ');
      await page.locator('.page-drawer[data-panel="export"] .drawer-bar button').click();

      // Unknown, too short and malformed IDs are refused without writing anything.
      expect(() => read('ffffffff')).toThrow(/COLAB_STATE_MISSING/);
      expect(() => read(shortIds[0].slice(0, 7))).toThrow(/COLAB_INPUT_INVALID/);
      expect(() => read('zzzzzzzz')).toThrow(/COLAB_INPUT_INVALID/);

      // Deleting the message makes its IDs unavailable.
      await openChat(page);
      const entry = panel.getByTestId('comment-entry').first();
      await entry.hover();
      await entry.getByRole('button', { name: 'Message actions', exact: true }).click();
      await entry.getByRole('menuitem', { name: 'Delete', exact: true }).click();
      await expect(panel.getByTestId('message-attachment')).toHaveCount(0, { timeout: 30_000 });
      expect(() => read(shortIds[0])).toThrow(/COLAB_STATE_MISSING/);
    } finally {
      fs.rmSync(out, { recursive: true, force: true });
    }
  });
});
