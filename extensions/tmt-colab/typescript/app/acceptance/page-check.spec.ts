import fs from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);

// #2753: a `colab-attachment:` reference is shown only when exactly one of the page's current
// document attachments matches it. `page check --page` and `page write` read the real page and
// its real attachment list, so a reference to a file that is not attached is reported before the
// page is published instead of rendering as a placeholder.
test('references resolve against the page’s current document attachments', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'page-check-author');
    const created = createPage(world, 'References', '<h1>References</h1>');
    const page = await openPage(door, author, created);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'References', exact: true }),
    ).toBeVisible();

    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'])) as Record<string, any>;
    // A refusal prints its report on stdout with the exit code 1.
    const refused = (args: string[]) => {
      try {
        run(world, world.binaries.colab, [...args, '--json']);
      } catch (error) {
        const failed = (error as Error).cause as { stdout?: string; status?: number };
        expect(failed.status).toBe(1);
        return JSON.parse(failed.stdout ?? '') as Record<string, any>;
      }
      throw new Error(`${args.join(' ')} should have been refused`);
    };

    const file = path.join(world.workspace, 'dot.png');
    fs.writeFileSync(file, PNG);
    const attached = colab(['attachment', 'attach', created.pageId, file]);
    const id = attached.attachment.attachmentId as string;
    expect(id).toMatch(/^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/);
    const foreignPrefix = id.startsWith('ffffffff') ? '00000000' : 'ffffffff';
    const foreignId = '10000000-0000-4000-8000-000000000999';

    const source = [
      `<img src="colab-attachment:${id}" alt="full id">`,
      `<img src="colab-attachment:${id.slice(0, 8)}" alt="prefix">`,
      `<img src="colab-attachment:${foreignPrefix}" alt="foreign prefix">`,
      `<img src="colab-attachment:${foreignId}" alt="foreign id">`,
      '',
    ].join('\n');
    const html = path.join(world.workspace, 'refs.html');
    fs.writeFileSync(html, source);

    // Without a page `check` validates the grammar only: all four references are well formed.
    expect(colab(['page', 'check', html]).ok).toBe(true);

    // With the page it resolves them: the two that name the attached file pass, the two that do
    // not are reported on their lines.
    const report = refused(['page', 'check', html, '--page', created.pageId]);
    expect(report.error.code).toBe('COLAB_PAGE_CHECK');
    expect(report.findings.map((f: { code: string; line: number }) => [f.code, f.line])).toEqual([
      ['attachment-ref-unresolved', 3],
      ['attachment-ref-unresolved', 4],
    ]);

    // A write reads the same attachment list and reports the same two as warnings.
    const before = colab(['page', 'read', created.pageId]);
    const written = colab([
      'page',
      'write',
      created.pageId,
      '--file',
      html,
      '--expected-revision',
      before.revision,
    ]);
    expect(written.warnings.map((f: { code: string; line: number }) => [f.code, f.line])).toEqual([
      ['attachment-ref-unresolved', 3],
      ['attachment-ref-unresolved', 4],
    ]);

    // --strict refuses before anything is written; the page keeps its revision.
    const stable = colab(['page', 'read', created.pageId]);
    fs.writeFileSync(html, `${source}<p>More</p>\n`);
    const strict = refused([
      'page',
      'write',
      created.pageId,
      '--file',
      html,
      '--expected-revision',
      stable.revision,
      '--strict',
    ]);
    expect(strict.error.code).toBe('COLAB_PAGE_CHECK');
    expect(colab(['page', 'read', created.pageId])).toEqual(stable);

    // Only the two resolving references: nothing to report.
    fs.writeFileSync(html, source.split('\n').slice(0, 2).join('\n'));
    const clean = colab(['page', 'check', html, '--page', created.pageId]);
    expect(clean.ok).toBe(true);
    expect(clean.findings).toEqual([]);
  });
});
