import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { distinct, openSource, save } from './harness/page-source.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// This case moves megabytes through traced actions. Its trace keeps every action, log, network
// event and screenshot, but not a DOM snapshot per action: each snapshot would hold the whole
// source twice (field and preview), about 280 MB that the browser close must compress inside the
// world's closer bound. The assertions compare bytes, which a DOM snapshot does not show.
test.use({ trace: { mode: 'retain-on-failure', snapshots: false, screenshots: true } });
test.afterEach(disposeActiveWorlds);

test('a 1.5 MiB page takes a browser save, a CLI write and 80 small browser edits, byte for byte', async () => {
  test.setTimeout(900_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'save-author');
    const colab = world.binaries.colab;
    const read = (id: string) =>
      JSON.parse(run(world, colab, ['page', 'read', id, '--json'])) as { source: string };
    const big = 1.5 * 1024 * 1024;
    const a = distinct(big, 'a');
    const created = createPage(world, 'Save big', a);
    const page = await openPage(door, browser, created);
    const box = await openSource(page);
    await expect(box).toHaveValue(a, { timeout: 60_000 });

    // The browser saves a different 1.5 MiB; the CLI reads exactly those bytes.
    const b = distinct(big, 'b');
    await box.fill(b);
    const started = Date.now();
    await save(page);
    await expect.poll(() => read(created.pageId).source === b, { timeout: 60_000 }).toBe(true);
    console.log(`SAVE_UPLOAD browser 1.5 MiB save reached the CLI in ${Date.now() - started} ms`);
    await expect(page.getByRole('alert')).toHaveCount(0);

    // A CLI write of another 1.5 MiB reaches the open browser, which then saves on top of it.
    const c = distinct(big, 'c');
    run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], c);
    await expect(box).toHaveValue(c, { timeout: 60_000 });
    const d = c + '<p>browser after CLI</p>';
    await box.fill(d);
    await save(page);
    await expect.poll(() => read(created.pageId).source === d, { timeout: 60_000 }).toBe(true);
    await page.reload();
    await expect(await openSource(page)).toHaveValue(d, { timeout: 60_000 });

    // Many small edits whose changes total far more than one 256 KiB update.
    const small = createPage(world, 'Save small', '<p>start</p>');
    const sp = await openPage(door, browser, small);
    const sbox = await openSource(sp);
    let current = '<p>start</p>';
    for (let i = 0; i < 80; i++) {
      const next = current + `<i>${distinct(8 * 1024, `s${i}`)}</i>`;
      await sbox.fill(next);
      await save(sp);
      await expect
        .poll(() => read(small.pageId).source === next, { timeout: 30_000, message: `edit ${i}` })
        .toBe(true);
      await expect(sbox).toHaveValue(next);
      current = next;
    }
    expect(current.length).toBeGreaterThan(640 * 1024);
    await expect(sp.getByRole('alert')).toHaveCount(0);
  });
});
