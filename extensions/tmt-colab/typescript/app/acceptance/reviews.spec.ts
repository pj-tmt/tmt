import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

const pending = (page: import('@playwright/test').Page, count: number) =>
  page.getByRole('button', { name: `Needs you: ${count}`, exact: true });

test('a section confirmed in the browser is a resolved thread to the CLI, and an agent resolve never closes one', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('review-agent');
    const author = await pairBrowser(world, 'review-author', { talk: true });
    const viewer = await pairBrowser(world, 'review-viewer', { talk: true });
    const section = (id: number, title: string, content: string) =>
      `<tmt-review data-id="10000000-0000-4000-8000-00000000030${id}" data-agent="${agent.name}"><h2 data-title>${title}</h2><p>${content}</p></tmt-review>`;
    const created = createPage(
      world,
      'Review acceptance',
      `<h1>Review page</h1>${section(1, 'Section one', 'First content.')}${section(2, 'Section two', 'Second content.')}`,
      agent.pane,
    );
    const colab = (args: string[], pane?: string) =>
      run(world, world.binaries.colab, args, undefined, pane);
    const listing = () =>
      (
        JSON.parse(colab(['threads', created.pageId, '--json'])) as {
          threads: {
            id: string;
            resolved: boolean;
            anchor: { exact: string } | null;
            comments: { body: string }[];
          }[];
        }
      ).threads.filter((thread) => thread.anchor?.exact.startsWith('Section'));
    const thread = (title: string) => listing().find((value) => value.anchor?.exact === title);

    const first = await openPage(door, author, created);
    const ownerCount = async (count: number) => {
      // A list read schedules the existing background refresh; only a fresh summary is known.
      await expect
        .poll(async () => {
          const response = await first
            .context()
            .request.get(new URL('api/pages', `${door.mounts}colab/`).href);
          expect(response.status()).toBe(200);
          const body = (await response.json()) as {
            pages: { pageId: string; summary?: { needsYou: number; waiting: boolean } }[];
          };
          const summary = body.pages.find((page) => page.pageId === created.pageId)?.summary;
          if (summary) expect(summary.waiting).toBe(summary.needsYou > 0);
          return summary?.needsYou;
        })
        .toBe(count);
    };
    const bands = first.locator('.review-card');
    await expect(bands).toHaveCount(2);
    await expect(pending(first, 2)).toBeVisible();
    await ownerCount(2);
    expect(listing()).toHaveLength(0);

    // Confirm and close: one comment and a person Resolve, which tells the agent once.
    await bands.nth(0).getByRole('button', { name: 'Confirm and close', exact: true }).click();
    await expect(pending(first, 1)).toBeVisible();
    await expect.poll(() => thread('Section one')?.resolved).toBe(true);
    await ownerCount(1);
    expect(thread('Section one')!.comments).toHaveLength(1);
    expect(thread('Section one')!.comments[0].body).toContain('Confirmed “Section one”');
    await expect.poll(() => agent.received().length).toBe(1);

    // Another device reads the same state and sees the section faded but expanded.
    const second = await openPage(door, viewer, created);
    await expect(pending(second, 1)).toBeVisible();
    const faded = second.frameLocator('iframe').locator('tmt-review').first();
    await expect(faded).toHaveAttribute('data-colab-review-closed', 'true');
    await expect(faded.getByText('First content.')).toBeVisible();

    // The agent asks again with the existing reopen: the section waits and Confirm returns.
    const threadId = thread('Section one')!.id;
    const reopened = JSON.parse(
      colab(['threads', 'reopen', created.pageId, threadId, '--json'], agent.pane),
    ) as { changed: boolean };
    expect(reopened.changed).toBe(true);
    await expect(pending(first, 2)).toBeVisible();
    await expect(pending(second, 2)).toBeVisible();
    await ownerCount(2);
    await expect(
      bands.nth(0).getByRole('button', { name: 'Confirm and close', exact: true }),
    ).toBeEnabled();

    // Needs change: a comment and an Ask, the thread stays open.
    await bands.nth(1).getByRole('button', { name: 'Needs change…', exact: true }).click();
    await bands.nth(1).getByLabel('What needs to change', { exact: true }).fill('Say more here.');
    await bands.nth(1).getByRole('button', { name: 'Send', exact: true }).click();
    await expect(bands.nth(1)).toHaveAttribute('data-review-state', 'changes');
    await expect(pending(first, 1)).toBeVisible();
    await expect.poll(() => thread('Section two')?.comments.length).toBe(1);
    expect(thread('Section two')!.resolved).toBe(false);
    expect(thread('Section two')!.comments[0].body).toContain(
      'Needs change to “Section two”: Say more here.',
    );
    await expect.poll(() => agent.received().length).toBe(2);

    // An agent's own resolve is labelled as the agent's and does not close the section.
    const resolved = JSON.parse(
      colab(
        ['threads', 'resolve', created.pageId, thread('Section two')!.id, '--json'],
        agent.pane,
      ),
    ) as { changed: boolean; action: { actor: string } };
    expect(resolved.changed).toBe(true);
    expect(resolved.action.actor).toBe('agent');
    await expect.poll(() => thread('Section two')?.resolved).toBe(true);
    // The fixture replies to the change Ask, returning it to the reader's pending list.
    await expect(bands.nth(1)).toHaveAttribute('data-review-state', 'waiting');
    await expect(pending(first, 2)).toBeVisible();
    await expect(bands.nth(1)).not.toHaveAttribute('data-review-state', 'closed');

    // Only the reader's confirmation closes it: reopen first, then a person Resolve.
    await bands.nth(1).getByRole('button', { name: 'Confirm and close', exact: true }).click();
    await expect(pending(first, 1)).toBeVisible();
    await expect(pending(second, 1)).toBeVisible();
    await expect(bands.nth(1)).toHaveAttribute('data-review-state', 'closed');
    await expect(second.locator('.review-card').nth(1)).toHaveAttribute(
      'data-review-state',
      'closed',
    );
    await expect.poll(() => thread('Section two')?.resolved).toBe(true);
    await ownerCount(1);
  });
});

test('recommended reading records one person read and notification, survives reload, and links only scroll', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('reading-agent');
    const author = await pairBrowser(world, 'reading-author', { talk: true });
    const viewer = await pairBrowser(world, 'reading-viewer', { talk: true });
    const id = '10000000-0000-4000-8000-000000000401';
    const next = '10000000-0000-4000-8000-000000000402';
    const block = (id: string, title: string) =>
      `<tmt-read data-id="${id}" data-agent="${agent.name}" data-why="Read before deciding"><h2 data-title>${title}</h2><p>Content of ${title}.</p></tmt-read>`;
    const created = createPage(
      world,
      'Reading acceptance',
      `<h1>Read these decisions</h1>${block(id, 'First decision')}<div style="height:1000px"></div>${block(next, 'Next decision')}`,
      agent.pane,
    );
    const first = await openPage(door, author, created);
    const unread = (page: import('@playwright/test').Page, count: number) =>
      expect(
        page.locator('[data-inline-reading][data-reading-state="unread"]:not([data-detached])'),
      ).toHaveCount(count);
    // The mounted shell carries a reading hint in route search; it only scrolls.
    const openReading = async (page: import('@playwright/test').Page, read: string) => {
      const url = new URL(page.url());
      url.searchParams.set('read', read);
      await page.goto(url.href);
    };
    await unread(first, 2);
    await openReading(first, id);
    const band = first.locator(`[data-inline-reading="${id}"]`);
    await expect(band.getByRole('button', { name: 'Mark read', exact: true })).toBeEnabled();
    const highlighted = (page: import('@playwright/test').Page) =>
      page
        .frameLocator('iframe')
        .locator('body')
        .evaluate(() => {
          const highlights = (CSS as unknown as { highlights: Map<string, { size: number }> })
            .highlights;
          return highlights.get('colab-reading')?.size;
        });
    await expect.poll(() => highlighted(first)).toBe(1);
    expect(
      JSON.parse(run(world, world.binaries.colab, ['threads', created.pageId, '--json'])).threads,
    ).toHaveLength(0);
    expect(agent.received()).toHaveLength(0);
    await band.getByRole('button', { name: 'Mark read', exact: true }).click();
    await expect(band).toHaveAttribute('data-reading-state', 'read');
    await expect.poll(() => agent.received().length).toBe(1);
    await expect(band).toContainText(`${agent.name} notified`);
    const listing = () =>
      (
        JSON.parse(run(world, world.binaries.colab, ['threads', created.pageId, '--json'])) as {
          threads: {
            resolved: boolean;
            comments: { body: string }[];
            anchor: { exact: string } | null;
          }[];
        }
      ).threads;
    await expect
      .poll(() => listing().filter((thread) => thread.anchor?.exact === 'First decision').length)
      .toBe(1);
    const read = listing().find((thread) => thread.anchor?.exact === 'First decision')!;
    expect(read.resolved).toBe(true);
    expect(read.comments).toHaveLength(1);
    expect(read.comments[0].body).toContain(`Reading: ${id}`);
    await unread(first, 1);
    await openReading(first, next);
    await expect.poll(() => highlighted(first)).toBe(1);
    expect(listing().filter((thread) => thread.anchor?.exact === 'Next decision')).toHaveLength(0);
    expect(agent.received()).toHaveLength(1);
    await first.reload();
    await expect(first.locator(`[data-inline-reading="${id}"]`)).toHaveAttribute(
      'data-reading-state',
      'read',
    );
    expect(agent.received()).toHaveLength(1);
    const second = await openPage(door, viewer, created);
    await openReading(second, next);
    await expect(second.locator(`[data-inline-reading="${next}"]`)).toHaveAttribute(
      'data-reading-state',
      'unread',
    );
    await expect.poll(() => highlighted(second)).toBe(1);
    expect(listing().filter((thread) => thread.anchor?.exact === 'Next decision')).toHaveLength(0);
    expect(agent.received()).toHaveLength(1);
  });
});
