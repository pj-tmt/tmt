import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// #1627 slice B: the CLI combines its own tail into a checkpoint, so a page takes far more than
// 200 changes, the browser still opens and edits it, and the exact source survives everywhere.
const MIB = 1024 * 1024;
const text = (n: number) => {
  let out = '';
  for (let i = 0; out.length < n; i++)
    out += `<p id="p${i}">Line ${i}: ${'lorem ipsum dolor sit amet '.repeat(3)}</p>\n`;
  return out.slice(0, n);
};
test.afterEach(disposeActiveWorlds);
test('260 CLI changes with a browser edit in the middle read back exact in the CLI and the browser', async () => {
  test.setTimeout(600_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const a = await pairBrowser(world, 'compact-author');
    const colab = world.binaries.colab;
    const read = (id: string) =>
      JSON.parse(run(world, colab, ['page', 'read', id, '--json'])) as { source: string };
    const created = createPage(world, 'Many changes', '<p>start</p>');
    const page = await openPage(door, a, created);

    let source = '<p>start</p>';
    const append = (i: number) => {
      source += `<i>${String(i).padStart(4, '0')}</i>`;
      run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], source);
    };
    for (let i = 0; i < 100; i++) append(i);
    // The browser edits in the middle of the run, on top of the CLI's changes.
    await page.reload();
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    const box = page.getByRole('textbox', { name: 'Source', exact: true });
    expect((await box.inputValue()) === source).toBe(true);
    source += '<p>from the browser</p>';
    await box.evaluate((el, value) => {
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
      set.call(el, value);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }, source);
    await page.getByRole('button', { name: 'Save source', exact: true }).click();
    await expect(page.getByRole('alert')).toHaveCount(0);
    await expect.poll(() => read(created.pageId).source === source).toBe(true);
    // More than the old 200-change limit, with the CLI's own changes after the browser's.
    for (let i = 100; i < 260; i++) append(i);
    expect(read(created.pageId).source === source).toBe(true);

    await page.reload();
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    expect(
      (await page.getByRole('textbox', { name: 'Source', exact: true }).inputValue()) === source,
    ).toBe(true);
  });
});

test('a 2 MiB page keeps taking changes and reads back exact', async () => {
  test.setTimeout(600_000);
  await withWorld(async (world) => {
    const colab = world.binaries.colab;
    let source = text(2 * MIB - 16 * 1024);
    const created = createPage(world, 'Big and busy', source);
    for (let i = 0; i < 300; i++) {
      source = source.slice(0, -4) + `<i>${i}</i>` + source.slice(-4);
      run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], source);
    }
    const read = JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
      source: string;
    };
    expect(read.source === source).toBe(true);
  });
});
