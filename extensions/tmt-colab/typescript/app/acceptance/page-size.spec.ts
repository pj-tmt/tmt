import { expect, test } from '@playwright/test';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// Ben's #1627 state: browser saves, then many CLI appends, push the page's update tail past the
// old 256 KiB / 200-object read caps. Every root-local reader must still return it byte-exact.
const text = (n: number, tag: string) => `<p>${tag.repeat(Math.max(0, n - 7))}</p>`;

test.afterEach(disposeActiveWorlds);
test('a page past the old tail caps still reads back byte-exact and does not hide the others', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const a = await pairBrowser(world, 'size-author');
    const other = createPage(world, 'Neighbour', '<p>unaffected</p>');
    const created = createPage(world, 'Big page', '<p>start</p>');
    const page = await openPage(door, a, created);

    // Two browser whole-source saves, 100 KiB then 200 KiB.
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    for (const [kb, tag] of [
      [100, 'a'],
      [200, 'b'],
    ] as const) {
      await page.getByLabel('Source', { exact: true }).fill(text(kb * 1024, tag));
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
    expect(read().source).toBe(text(200 * 1024, 'b'));

    // Past 200 objects with small CLI appends (create, two saves, then 205 more).
    let source = text(200 * 1024, 'b');
    for (let i = 0; i < 205; i++) {
      source += `<i>${String(i).padStart(4, '0')}</i>`;
      run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], source);
    }

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
    // The browser's fold worker still stops at 200 updates / 256 KiB; opening such a page there
    // belongs to the browser-side limit agreement (#1627 part 2).
  });
});
