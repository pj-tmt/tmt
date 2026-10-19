import { expect, test, type Page } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

test('a shell reading item is Unread until the page itself marks it read; opening it marks nothing', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('shell-reading-agent');
    const author = await pairBrowser(world, 'shell-reading-author', { talk: true });
    const reading = '10000000-0000-4000-8000-000000000501';
    const colab = (args: string[], input?: string) =>
      run(world, world.binaries.colab, args, input, agent.pane);
    const source = createPage(
      world,
      'Reading source',
      `<h1>Source</h1><tmt-read data-id="${reading}" data-agent="${agent.name}" data-why="Read before deciding"><h2 data-title>First decision</h2><p>Content.</p></tmt-read>`,
      agent.pane,
    );
    const shell = JSON.parse(colab(['shell', 'create', '--title', 'Reading list', '--json'])) as {
      pageId: string;
      path: string;
    };
    const revision = (
      JSON.parse(colab(['shell', 'read', shell.pageId, '--json'])) as { revision: string }
    ).revision;
    colab(
      [
        'shell',
        'reading-suggest',
        shell.pageId,
        '--expected-revision',
        revision,
        '--file',
        '-',
        '--json',
      ],
      JSON.stringify({
        itemId: '20000000-0000-4000-8000-000000000501',
        pageId: source.pageId,
        readingId: reading,
        note: 'Before Friday',
      }),
    );
    const threads = () =>
      (JSON.parse(colab(['threads', source.pageId, '--json'])) as { threads: unknown[] }).threads;

    const page: Page = await author.context.newPage();
    await page.goto(new URL(shell.path, `${door.address}/`).href);
    const cell = page.locator('.shell-cell');
    await expect(cell).toHaveCount(1);
    // Arriving on the shell, and a suggestion, mark nothing and notify no one.
    await expect(cell.locator('.shell-cell-state')).toHaveText('• Unread');
    await expect(cell.locator('a')).toHaveText('First decision');
    expect(agent.received()).toHaveLength(0);
    expect(threads()).toHaveLength(0);

    // Opening the item scrolls to the reading; it stays unread until the page's own Mark read.
    await cell.locator('a').click();
    const band = page.locator(`[data-inline-reading="${reading}"]`);
    await expect(band).toHaveAttribute('data-reading-state', 'unread');
    expect(agent.received()).toHaveLength(0);
    expect(threads()).toHaveLength(0);

    // The real user path: Mark read is disabled until the page can act (never a silent no-op),
    // so the click itself waits for it.
    await band.getByRole('button', { name: 'Mark read', exact: true }).click();
    await expect(band).toHaveAttribute('data-reading-state', 'read', { timeout: 15_000 });
    await expect.poll(() => agent.received().length).toBe(1);

    // Coming back, and a reload, show what the page recorded.
    await page.goBack();
    await expect(cell.locator('.shell-cell-state')).toHaveText('✓ Read');
    await page.reload();
    await expect(cell.locator('.shell-cell-state')).toHaveText('✓ Read');
    expect(agent.received()).toHaveLength(1);
  });
});
