import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// #1627: a page created at 1.5 MiB (and up to the 2 MiB source limit) must create, read, open and
// edit in the browser and the CLI. Past the limit, creation refuses by name.
const MIB = 1024 * 1024;
const text = (n: number) => {
  // Varied, not one repeated byte, so the page is a realistic size on the wire.
  const line = (i: number) =>
    `<p id="p${i}">Line ${i}: ${'lorem ipsum dolor sit amet '.repeat(3)}</p>\n`;
  let out = '';
  for (let i = 0; out.length < n; i++) out += line(i);
  return out.slice(0, n);
};

test.afterEach(disposeActiveWorlds);
test('a 1.5 MiB page creates, opens in the browser and takes edits from both sides', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const a = await pairBrowser(world, 'size-author');
    const colab = world.binaries.colab;
    const read = (id: string) =>
      JSON.parse(run(world, colab, ['page', 'read', id, '--json'])) as { source: string };

    const source = text(1.5 * MIB);
    const created = createPage(world, 'Large page', source);
    expect(read(created.pageId).source === source).toBe(true);

    // The browser opens it with the exact source and saves a small edit.
    const page = await openPage(door, a, created);
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    const box = page.getByRole('textbox', { name: 'Source', exact: true });
    expect((await box.inputValue()) === source).toBe(true);
    const browserEdit = source + '<p>browser edit</p>';
    // A paste: Playwright's own fill() takes minutes to insert 1.5 MiB into a textarea.
    await box.evaluate((el, value) => {
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
      set.call(el, value);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }, browserEdit);
    await page.getByRole('button', { name: 'Save source', exact: true }).click();
    await expect(page.getByRole('alert')).toHaveCount(0);
    await expect.poll(() => read(created.pageId).source === browserEdit).toBe(true);

    // A small CLI edit on top, then the browser reloads to the latest.
    const cliEdit = browserEdit + '<p>cli edit</p>';
    run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], cliEdit);
    expect(read(created.pageId).source === cliEdit).toBe(true);
    await page.reload();
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    expect(
      (await page.getByRole('textbox', { name: 'Source', exact: true }).inputValue()) === cliEdit,
    ).toBe(true);
  });
});

test('creating past the 2 MiB source limit refuses by name and creates nothing', async () => {
  test.setTimeout(120_000);
  await withWorld(async (world) => {
    const atLimitSource = text(2 * MIB);
    const atLimit = createPage(world, 'At the limit', atLimitSource);
    expect(atLimit.pageId).toBeTruthy();
    // Native publication batches a large replacement into bounded updates.
    const small = createPage(world, 'Small', '<p>small</p>');
    const replacement = text(1.5 * MIB);
    run(
      world,
      world.binaries.colab,
      ['page', 'write', small.pageId, '--file', '-', '--json'],
      replacement,
    );
    const read = (id: string) =>
      JSON.parse(run(world, world.binaries.colab, ['page', 'read', id, '--json'])) as {
        source: string;
        revision: string;
      };
    expect(read(small.pageId).source).toBe(replacement);

    // A fresh 2 MiB tail plus a fully different 2 MiB replacement exceeds the
    // retained 4 MiB budget. Refusal preserves both exact source and revision.
    const before = read(atLimit.pageId);
    let replace = '';
    try {
      run(
        world,
        world.binaries.colab,
        ['page', 'write', atLimit.pageId, '--file', '-', '--json'],
        'z'.repeat(2 * MIB),
      );
    } catch (error) {
      replace = (error as Error).message;
    }
    expect(replace).toContain('COLAB_CAPACITY');
    expect(replace).toContain(atLimit.pageId);
    expect(replace).toContain('4 MiB');
    expect(replace).toContain('tmt colab export');
    expect(read(atLimit.pageId)).toEqual(before);
    expect(before.source).toBe(atLimitSource);
    let refusal = '';
    try {
      createPage(world, 'Over the limit', text(2 * MIB + 1));
    } catch (error) {
      refusal = (error as Error).message;
    }
    expect(refusal).toContain('COLAB_CAPACITY');
    expect(refusal).toContain('2097153 bytes (2 MiB)');
    expect(refusal).toContain('at most 2097152 bytes (2 MiB)');
    expect(refusal).toContain('Nothing was written.');
    const listed = JSON.parse(run(world, world.binaries.colab, ['ls', '--json'])) as {
      pages: { title: string | null }[];
    };
    expect(listed.pages.map((p) => p.title).sort()).toEqual(['At the limit', 'Small']);
  });
});
