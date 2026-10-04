import { expect, test } from '@playwright/test';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// #1627: browser saves, then CLI appends up to the browser's 200-update limit. The next write
// refuses by name, and every reader (page read, show, export, ls, the browser) returns the page.
const text = (n: number, tag: string) => `<p>${tag.repeat(Math.max(0, n - 7))}</p>`;

test.afterEach(disposeActiveWorlds);
test('a page at the browser tail limit refuses another write, stays readable everywhere and opens in the browser', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const a = await pairBrowser(world, 'size-author');
    const other = createPage(world, 'Neighbour', '<p>unaffected</p>');
    const created = createPage(world, 'Big page', '<p>start</p>');
    const page = await openPage(door, a, created);

    // Two browser whole-source saves, 60 KiB then 100 KiB.
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    for (const [kb, tag] of [
      [60, 'a'],
      [100, 'b'],
    ] as const) {
      await page.getByRole('textbox', { name: 'Source', exact: true }).fill(text(kb * 1024, tag));
      await page.getByRole('button', { name: 'Save source', exact: true }).click();
      await expect(page.getByRole('alert')).toHaveCount(0);
      await page.waitForTimeout(500);
    }
    const colab = world.binaries.colab;
    const read = () =>
      JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
        source: string;
        revision: string;
      };
    expect(read().source).toBe(text(100 * 1024, 'b'));

    // Small CLI appends until the page reaches the browser's 200-update limit: the next write
    // refuses, naming the page and the way out, and the page stays readable.
    let source = text(100 * 1024, 'b');
    let refusal = '';
    for (let i = 0; i < 260 && !refusal; i++) {
      const next = source + `<i>${String(i).padStart(4, '0')}</i>`;
      try {
        run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], next);
        source = next;
      } catch (error) {
        refusal = (error as Error).message;
      }
    }
    expect(refusal).toContain('COLAB_CAPACITY');
    expect(refusal).toContain(created.pageId);
    expect(refusal).toContain('would pass the limit of 200');
    expect(refusal).toContain('tmt colab export');

    // page read, show, export and ls all return it (and the neighbour) byte-exact.
    expect(read().source === source).toBe(true);
    const shown = JSON.parse(run(world, colab, ['show', created.pageId, '--json'])) as {
      page: { title: string; error?: unknown };
    };
    expect(shown.page.title).toBe('Big page');
    expect(shown.page.error).toBeUndefined();
    const directory = mkdtempSync(path.join(tmpdir(), 'colab-size-'));
    try {
      const exported = JSON.parse(
        run(world, colab, ['export', created.pageId, '--dir', directory, '--json']),
      ) as { directory: string };
      expect(readFileSync(path.join(exported.directory, 'page.html'), 'utf8') === source).toBe(
        true,
      );
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
    const listed = JSON.parse(run(world, colab, ['ls', '--json'])) as {
      pages: { pageId: string; title: string | null; error?: unknown }[];
    };
    expect(listed.pages.map((p) => p.title).sort()).toEqual(['Big page', 'Neighbour']);
    expect(listed.pages.some((p) => p.error !== undefined)).toBe(false);
    expect(other.pageId).not.toBe(created.pageId);
    // The browser opens the page at the limit and shows the exact source.
    await page.reload();
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    expect(await page.getByRole('textbox', { name: 'Source', exact: true }).inputValue()).toBe(
      source,
    );
  });
});
